//! Release tag preflight, exact `origin/release/X.Y` lineage validation (`M1`),
//! and read-only / local `develop-agent` branch alignment check (`H1`).

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::checks_arch::check_schema_compat;
use crate::util::{capture_cmd, workspace_version};
use crate::verify::verify_head_receipt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTag<'a> {
    pub raw_version: &'a str,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub prerelease: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncVerdict {
    ExactMatch,
    PromotedAncestor,
    AgentAheadOfPromotedBase,
    LocalCreatedOrFastForwarded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncDrift {
    MissingDevelopAgent,
    NonAncestorSameTree,
    MainHasUnbackportedChanges,
}

pub fn parse_semver_tag(tag: &str) -> Result<ParsedTag<'_>, String> {
    let ver = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("release tag `{tag}` must start with `v`"))?;
    let (core_ver, prerelease) = match ver.split_once('-') {
        Some((c, pre)) => (c, Some(pre)),
        None => (ver, None),
    };
    let parts: Vec<&str> = core_ver.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|p| is_canonical_uint(p)) {
        return Err(format!("invalid semver core in tag `{tag}`"));
    }
    let major = parts[0].parse::<u32>().map_err(|e| e.to_string())?;
    let minor = parts[1].parse::<u32>().map_err(|e| e.to_string())?;
    let patch = parts[2].parse::<u32>().map_err(|e| e.to_string())?;

    if let Some(pre) = prerelease {
        let Some((channel, num)) = pre.split_once('.') else {
            return Err(format!(
                "prerelease tag `{tag}` must match vX.Y.Z-(alpha|beta|rc).N"
            ));
        };
        if !matches!(channel, "alpha" | "beta" | "rc") || !is_canonical_uint(num) || num == "0" {
            return Err(format!("invalid prerelease suffix `{pre}` in tag `{tag}`"));
        }
    }

    Ok(ParsedTag {
        raw_version: ver,
        major,
        minor,
        patch,
        prerelease,
    })
}

fn is_canonical_uint(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0'))
}

pub fn validate_tag_metadata(root: &Path, tag: &str) -> Result<String, String> {
    let parsed = parse_semver_tag(tag)?;
    let cargo_ver = workspace_version(root)?;
    if cargo_ver != parsed.raw_version {
        return Err(format!(
            "tag `{tag}` expects Cargo.toml version `{}`, found `{cargo_ver}`",
            parsed.raw_version
        ));
    }
    check_schema_compat(root)?;

    let notes_path = root.join("docs/releases").join(format!("{tag}.md"));
    let meta = fs::metadata(&notes_path)
        .map_err(|_| format!("missing release notes `{}`", notes_path.display()))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!(
            "release notes `{}` must be non-empty",
            notes_path.display()
        ));
    }
    Ok(parsed.raw_version.to_owned())
}

pub fn preflight_tag(root: &Path, tag: &str) -> Result<(), String> {
    let expected_version = validate_tag_metadata(root, tag)?;
    let receipt = verify_head_receipt(root)?;
    println!(
        "ok: preflight-tag {tag} verified (version={expected_version}, tree={})",
        receipt.head_tree
    );
    Ok(())
}

pub fn check_release_lineage(root: &Path, tag: &str, commit_ref: &str) -> Result<String, String> {
    let parsed = parse_semver_tag(tag)?;
    let commit_sha = capture_cmd(root, "git", &["rev-parse", "--verify", commit_ref])?;
    let release_ref = format!(
        "refs/remotes/origin/release/{}.{}",
        parsed.major, parsed.minor
    );
    let main_ref = "refs/remotes/origin/main";

    if parsed.patch == 0 {
        if is_ancestor(root, &commit_sha, main_ref) {
            return Ok(format!("{tag} ({commit_sha}) verified on {main_ref}"));
        }
        if is_ancestor(root, &commit_sha, &release_ref) {
            return Ok(format!("{tag} ({commit_sha}) verified on {release_ref}"));
        }
        return Err(format!(
            "tag `{tag}` (Z == 0) must descend from `{main_ref}` or `{release_ref}`"
        ));
    }

    if is_ancestor(root, &commit_sha, &release_ref) {
        return Ok(format!(
            "patch tag `{tag}` ({commit_sha}) verified on exact lineage `{release_ref}`"
        ));
    }
    Err(format!(
        "patch tag `{tag}` (Z > 0) must descend from exact maintenance branch `{release_ref}`"
    ))
}

fn is_ancestor(root: &Path, anc: &str, desc: &str) -> bool {
    let exists = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", "--quiet", desc])
        .output()
        .is_ok_and(|o| o.status.success());
    exists
        && Command::new("git")
            .current_dir(root)
            .args(["merge-base", "--is-ancestor", anc, desc])
            .status()
            .is_ok_and(|s| s.success())
}

pub fn sync_branches(root: &Path, apply_local: bool) -> Result<SyncVerdict, String> {
    let has_origin = Command::new("git")
        .current_dir(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .is_ok_and(|o| o.status.success());

    let (main_ref, dev_ref) = if has_origin {
        let _ = Command::new("git")
            .current_dir(root)
            .args(["fetch", "origin", "+refs/heads/*:refs/remotes/origin/*"])
            .status();
        (
            "refs/remotes/origin/main",
            "refs/remotes/origin/develop-agent",
        )
    } else {
        ("refs/heads/main", "refs/heads/develop-agent")
    };

    match classify_branch_sync(root, main_ref, dev_ref)? {
        Ok(verdict) => {
            if apply_local && !has_origin && verdict == SyncVerdict::PromotedAncestor {
                let main_sha = capture_cmd(root, "git", &["rev-parse", main_ref])?;
                update_local_dev_ref(root, &main_sha)?;
                return Ok(SyncVerdict::LocalCreatedOrFastForwarded);
            }
            Ok(verdict)
        }
        Err(SyncDrift::MissingDevelopAgent) if apply_local && !has_origin => {
            let main_sha = capture_cmd(root, "git", &["rev-parse", main_ref])?;
            update_local_dev_ref(root, &main_sha)?;
            Ok(SyncVerdict::LocalCreatedOrFastForwarded)
        }
        Err(drift) => Err(format!(
            "`develop-agent` sync check failed ({drift:?}); open a rebase PR onto `develop-agent`"
        )),
    }
}

pub fn classify_branch_sync(
    root: &Path,
    main_ref: &str,
    dev_ref: &str,
) -> Result<Result<SyncVerdict, SyncDrift>, String> {
    let main_sha = capture_cmd(root, "git", &["rev-parse", "--verify", main_ref])?;
    let Ok(dev_sha) = capture_cmd(root, "git", &["rev-parse", "--verify", dev_ref]) else {
        return Ok(Err(SyncDrift::MissingDevelopAgent));
    };

    if dev_sha == main_sha {
        return Ok(Ok(SyncVerdict::ExactMatch));
    }

    let main_tree = capture_cmd(root, "git", &["rev-parse", &format!("{main_sha}^{{tree}}")])?;
    let dev_tree = capture_cmd(root, "git", &["rev-parse", &format!("{dev_sha}^{{tree}}")])?;

    if is_ancestor(root, &dev_sha, &main_sha) {
        if dev_tree == main_tree {
            return Ok(Ok(SyncVerdict::PromotedAncestor));
        }
        return Ok(Err(SyncDrift::MainHasUnbackportedChanges));
    }

    if is_ancestor(root, &main_sha, &dev_sha) {
        return Ok(Ok(SyncVerdict::AgentAheadOfPromotedBase));
    }

    // Check if `main` is a merge promotion of `M^2` where `M^2` is an ancestor of `dev_sha`
    // and `main_tree == M^2^{tree}` (agent moved ahead while PR merge was ratified).
    if let Ok(m2) = capture_cmd(
        root,
        "git",
        &["rev-parse", "--verify", &format!("{main_sha}^2")],
    ) && is_ancestor(root, &m2, &dev_sha)
        && let Ok(m2_tree) = capture_cmd(root, "git", &["rev-parse", &format!("{m2}^{{tree}}")])
        && m2_tree == main_tree
    {
        return Ok(Ok(SyncVerdict::AgentAheadOfPromotedBase));
    }

    if dev_tree == main_tree {
        return Ok(Err(SyncDrift::NonAncestorSameTree));
    }
    Ok(Err(SyncDrift::MainHasUnbackportedChanges))
}

fn update_local_dev_ref(root: &Path, sha: &str) -> Result<(), String> {
    let st = Command::new("git")
        .current_dir(root)
        .args(["update-ref", "refs/heads/develop-agent", sha])
        .status()
        .map_err(|e| format!("git update-ref develop-agent: {e}"))?;
    if !st.success() {
        return Err("updating local refs/heads/develop-agent failed".to_owned());
    }
    Ok(())
}
