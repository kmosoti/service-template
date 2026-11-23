#![forbid(unsafe_code)]

//! Pure-Rust workspace automation, verification receipt generator, SLSA artifact
//! packager, 4-host governance hook engine, and GitHub Ruleset provisioner.

mod agent_sync;
mod archive;
mod checks_arch;
mod checks_repo;
mod fingerprint;
mod github;
mod github_api;
mod github_readback;
#[cfg(test)]
mod github_tests;
mod hooks;
mod hooks_git;
mod hooks_shell;
#[cfg(test)]
mod hooks_tests;
mod jcs;
mod knowledge;
mod lineage;
#[cfg(test)]
mod lineage_tests;
mod package;
mod trust_boundary;
#[cfg(test)]
mod trust_boundary_tests;
mod util;
mod verify;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use hooks::{HookFormat, run_pre_tool_use_stdin};
use util::workspace_root;

fn print_help() {
    eprintln!(
        "Usage: cargo xtask <command> [options]\n\n\
         Commands:\n  \
           verify [--force]                                Run all 12 gates and emit evidence/verification.json\n  \
           package [--out <dir>]                           Build stripped release binary, tarball & release-manifest.json\n  \
           preflight-tag <tag>                             Validate release tag, compat parity, notes & HEAD^{{tree}} receipt\n  \
           release-lineage <tag> [--commit <rev>]          Verify exact main / origin/release/X.Y ancestry for <tag>\n  \
           sync-branches [--check | --apply]               Verify develop-agent ancestry/tree parity (or align local ref)\n  \
           check-trust-boundary [--base <git-ref>]         Validate scoped 2-commit Trust-Boundary: verifier separation\n  \
           hook pre-tool-use --format <antigravity|claude|codex|copilot>\n  \
           github-setup <OWNER/REPO> [--main-pr-approvals <u8>] [--dry-run]\n  \
           check-architecture                              Verify Sans-I/O crate DAG and zero .sh/.py invariant\n  \
           check-schema-compat                             Verify schema/compatibility.json parity and transition bounds"
    );
}

fn dispatch() -> Result<(), String> {
    let root = workspace_root();
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        print_help();
        return Err("missing subcommand".to_owned());
    };

    match cmd {
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        "verify" => {
            let mut force = false;
            for arg in &args[1..] {
                match arg.as_str() {
                    "--force" => force = true,
                    other => return Err(format!("unknown option for `verify`: {other}")),
                }
            }
            verify::run_verify(&root, force)
        }
        "package" => {
            let out_dir = match args.len() {
                1 => root.join("dist"),
                3 if args[1] == "--out" => {
                    let p = PathBuf::from(&args[2]);
                    if p.is_absolute() { p } else { root.join(p) }
                }
                _ => return Err("usage: cargo xtask package [--out <dir>]".to_owned()),
            };
            package::package_release(&root, &out_dir)
        }
        "preflight-tag" => {
            if args.len() != 2 {
                return Err("usage: cargo xtask preflight-tag <tag>".to_owned());
            }
            lineage::preflight_tag(&root, &args[1])
        }
        "release-lineage" => {
            let (tag, commit_ref) = match args.len() {
                2 => (args[1].as_str(), "HEAD"),
                4 if args[2] == "--commit" => (args[1].as_str(), args[3].as_str()),
                _ => {
                    return Err(
                        "usage: cargo xtask release-lineage <tag> [--commit <rev>]".to_owned()
                    );
                }
            };
            let msg = lineage::check_release_lineage(&root, tag, commit_ref)?;
            println!("ok: {msg}");
            Ok(())
        }
        "sync-branches" => {
            let apply = match args.len() {
                1 => false,
                2 if args[1] == "--check" => false,
                2 if args[1] == "--apply" => true,
                _ => return Err("usage: cargo xtask sync-branches [--check | --apply]".into()),
            };
            let status = lineage::sync_branches(&root, apply)?;
            println!("ok: sync-branches verdict = {status:?}");
            Ok(())
        }
        "check-trust-boundary" => {
            let base = match args.len() {
                1 => None,
                3 if args[1] == "--base" => Some(args[2].as_str()),
                _ => {
                    return Err("usage: cargo xtask check-trust-boundary [--base <git-ref>]".into());
                }
            };
            let msg = trust_boundary::check_trust_boundary(&root, base)?;
            println!("ok: {msg}");
            Ok(())
        }
        "hook" => {
            if args.len() != 4 || args[1] != "pre-tool-use" || args[2] != "--format" {
                return Err(
                    "usage: cargo xtask hook pre-tool-use --format <antigravity|claude|codex|copilot>"
                        .into(),
                );
            }
            let fmt = HookFormat::parse(&args[3])?;
            run_pre_tool_use_stdin(&root, fmt)
        }
        "github-setup" => parse_and_run_github_setup(&root, &args[1..]),
        "check-architecture" => {
            println!("ok: {}", checks_arch::check_architecture(&root)?);
            Ok(())
        }
        "check-schema-compat" => {
            println!("ok: {}", checks_arch::check_schema_compat(&root)?);
            Ok(())
        }
        other => {
            print_help();
            Err(format!("unknown subcommand `{other}`"))
        }
    }
}

fn parse_and_run_github_setup(root: &std::path::Path, args: &[String]) -> Result<(), String> {
    let Some(repo) = args.first().map(String::as_str) else {
        return Err(
            "usage: cargo xtask github-setup <OWNER/REPO> [--main-pr-approvals <u8>] [--dry-run]"
                .into(),
        );
    };
    let mut approvals: u8 = 1;
    let mut dry_run = false;
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--main-pr-approvals" => {
                let raw = args
                    .get(i + 1)
                    .ok_or("missing integer value for `--main-pr-approvals`")?;
                approvals = raw
                    .parse::<u8>()
                    .map_err(|_| format!("invalid `--main-pr-approvals` value `{raw}`"))?;
                i += 2;
            }
            other => return Err(format!("unknown github-setup option `{other}`")),
        }
    }
    github::run_github_setup(root, repo, approvals, dry_run)
}

fn main() -> ExitCode {
    match dispatch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
