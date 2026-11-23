//! Shared workspace discovery, subprocess execution, hashing, and file walking.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits directly under workspace root")
        .to_path_buf()
}

pub fn is_cargo_offline() -> bool {
    env::var("CARGO_NET_OFFLINE")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

/// Serializes JSON in strict RFC 8785 JCS form (UTF-16 key order, safe integer bounds).
pub fn canonical_jcs_bytes(val: &Value) -> Result<Vec<u8>, String> {
    crate::jcs::to_jcs_bytes(val)
}

pub fn run_cmd(cwd: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .status()
        .map_err(|e| format!("failed to spawn `{program} {}`: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!(
            "command `{program} {}` exited with status {status}",
            args.join(" ")
        ));
    }
    Ok(())
}

pub fn run_cargo(cwd: &Path, subcommand: &str, rest: &[&str]) -> Result<(), String> {
    let mut full_args = Vec::with_capacity(rest.len() + 2);
    full_args.push(subcommand);
    if is_cargo_offline() && !rest.contains(&"--offline") {
        full_args.push("--offline");
    }
    full_args.extend_from_slice(rest);
    run_cmd(cwd, "cargo", &full_args)
}

pub fn capture_cmd(cwd: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute `{program} {}`: {e}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`{program} {}` failed ({}): {}",
            args.join(" "),
            output.status,
            stderr.trim()
        ));
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("invalid UTF-8 stdout: {e}"))?;
    Ok(stdout.trim().to_owned())
}

pub fn capture_cmd_raw(cwd: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute `{program} {}`: {e}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`{program} {}` failed ({}): {}",
            args.join(" "),
            output.status,
            stderr.trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("invalid UTF-8 stdout: {e}"))
}

pub fn walk_repo_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    walk_dir(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_dir(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|e| format!("failed reading directory {}: {e}", current.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if current == root && matches!(name.as_ref(), ".git" | "target" | "dist" | "evidence") {
            continue;
        }
        if name == ".git" {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|e| format!("stat error on {}: {e}", path.display()))?;
        if file_type.is_dir() {
            walk_dir(root, &path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

pub fn workspace_version(root: &Path) -> Result<String, String> {
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|e| format!("failed reading Cargo.toml: {e}"))?;
    let mut in_workspace_pkg = false;
    for raw_line in cargo_toml.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            in_workspace_pkg = line == "[workspace.package]";
            continue;
        }
        if in_workspace_pkg
            && let Some(rest) = line.strip_prefix("version")
            && let Some(val) = rest.trim_start().strip_prefix('=')
        {
            let unquoted = val.trim().trim_matches('"');
            if !unquoted.is_empty() {
                return Ok(unquoted.to_owned());
            }
        }
    }
    Err("could not locate [workspace.package].version in Cargo.toml".to_owned())
}
