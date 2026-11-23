//! Clean-tree staging build, path-remapped release binary check, and `dist/release-manifest.json`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tar::Archive;

use crate::archive::{
    build_deterministic_tar_gz, map_host_to_target_label, parse_rustc_host,
    scan_binary_for_host_paths,
};
use crate::checks_arch::{CompatibilityDescriptor, check_schema_compat, validate_compat_json_str};
use crate::util::{
    canonical_jcs_bytes, capture_cmd, is_cargo_offline, sha256_bytes, workspace_version,
};
use crate::verify::git_porcelain_all;

pub const RELEASE_MANIFEST_SCHEMA: &str = "service.release-manifest/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema: String,
    pub service_name: String,
    pub version: String,
    pub target: String,
    pub archive_name: String,
    pub engine_digest: String,
    pub binary_digest: String,
    pub compatibility_digest: String,
    pub compatibility: CompatibilityDescriptor,
    pub source_commit: String,
    pub source_tree: String,
    pub rustc_version: String,
}

pub fn package_release(root: &Path, out_dir: &Path) -> Result<(), String> {
    let porcelain = git_porcelain_all(root)?;
    if !porcelain.trim().is_empty() {
        return Err(
            "refusing `cargo xtask package` on dirty working tree; commit or clean changes first"
                .to_owned(),
        );
    }

    check_schema_compat(root)?;
    let version = workspace_version(root)?;
    let compat_raw = fs::read_to_string(root.join("schema/compatibility.json"))
        .map_err(|e| format!("failed reading schema/compatibility.json: {e}"))?;
    let compat = validate_compat_json_str(&compat_raw, true)?;
    let compat_val: Value = serde_json::from_str(&compat_raw).map_err(|e| e.to_string())?;
    let jcs_compat_hex = sha256_bytes(&canonical_jcs_bytes(&compat_val)?);

    let rustc_vv = capture_cmd(root, "rustc", &["-vV"])?;
    let host_triple = parse_rustc_host(&rustc_vv)?;
    let target_label = map_host_to_target_label(host_triple)?;

    let pid = std::process::id();
    let staging_dir = env::temp_dir().join(format!("svc-pkg-stage-{pid}"));
    let staging_target = env::temp_dir().join(format!("svc-pkg-target-{pid}"));
    let _ = fs::remove_dir_all(&staging_dir);
    let _ = fs::remove_dir_all(&staging_target);
    fs::create_dir_all(&staging_dir).map_err(|e| format!("create staging dir: {e}"))?;

    let build_res = build_staged_binary(
        root,
        &staging_dir,
        &staging_target,
        host_triple.contains("windows"),
    );
    let _ = fs::remove_dir_all(&staging_dir);
    let _ = fs::remove_dir_all(&staging_target);
    let (bin_name, bin_bytes) = build_res?;

    let archive_name = format!("{}-v{version}-{target_label}.tar.gz", compat.service_name);
    let archive_bytes = build_deterministic_tar_gz(&bin_name, &bin_bytes, compat_raw.as_bytes())?;

    let archive_sha = sha256_bytes(&archive_bytes);
    let binary_sha = sha256_bytes(&bin_bytes);
    let compat_file_sha = sha256_bytes(compat_raw.as_bytes());

    let source_commit = capture_cmd(root, "git", &["rev-parse", "HEAD"])?;
    let source_tree = capture_cmd(root, "git", &["rev-parse", "HEAD^{tree}"])?;
    let rustc_version = capture_cmd(root, "rustc", &["--version"])?;

    let manifest = ReleaseManifest {
        schema: RELEASE_MANIFEST_SCHEMA.to_owned(),
        service_name: compat.service_name.clone(),
        version,
        target: target_label.to_owned(),
        archive_name: archive_name.clone(),
        engine_digest: format!("sha256:{archive_sha}"),
        binary_digest: format!("sha256:{binary_sha}"),
        compatibility_digest: format!("sha256:{jcs_compat_hex}"),
        compatibility: compat,
        source_commit,
        source_tree,
        rustc_version,
    };
    let mut manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    manifest_json.push('\n');
    let manifest_sha = sha256_bytes(manifest_json.as_bytes());

    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed creating {}: {e}", out_dir.display()))?;
    fs::write(out_dir.join(&archive_name), &archive_bytes)
        .map_err(|e| format!("write {archive_name}: {e}"))?;
    fs::write(out_dir.join("compatibility.json"), &compat_raw)
        .map_err(|e| format!("write compatibility.json: {e}"))?;
    fs::write(out_dir.join("release-manifest.json"), &manifest_json)
        .map_err(|e| format!("write release-manifest.json: {e}"))?;

    let sha256sums = format!(
        "{archive_sha}  {archive_name}\n\
         {compat_file_sha}  compatibility.json\n\
         {manifest_sha}  release-manifest.json\n"
    );
    fs::write(out_dir.join("SHA256SUMS"), sha256sums)
        .map_err(|e| format!("write SHA256SUMS: {e}"))?;

    println!(
        "ok: packaged {archive_name} (engine_digest = sha256:{archive_sha}) into {}",
        out_dir.display()
    );
    Ok(())
}

fn build_staged_binary(
    root: &Path,
    staging_dir: &Path,
    staging_target: &Path,
    is_windows: bool,
) -> Result<(String, Vec<u8>), String> {
    let git_out = Command::new("git")
        .current_dir(root)
        .args(["archive", "--format=tar", "HEAD"])
        .output()
        .map_err(|e| format!("git archive HEAD failed: {e}"))?;
    if !git_out.status.success() {
        return Err("git archive HEAD exited non-zero".to_owned());
    }
    Archive::new(git_out.stdout.as_slice())
        .unpack(staging_dir)
        .map_err(|e| format!("unpack staged HEAD archive: {e}"))?;

    let (cargo_home, rustup_home) = resolve_toolchain_homes();
    let encoded_rustflags = [
        format!(
            "--remap-path-prefix={}={}",
            staging_dir.display(),
            "/workspace"
        ),
        format!(
            "--remap-path-prefix={}={}",
            cargo_home.display(),
            "/cargo-home"
        ),
        format!(
            "--remap-path-prefix={}={}",
            rustup_home.display(),
            "/rustup-home"
        ),
    ]
    .join("\x1f");

    let mut build_cmd = Command::new("cargo");
    build_cmd
        .current_dir(staging_dir)
        .env("CARGO_TARGET_DIR", staging_target)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_ENCODED_RUSTFLAGS", &encoded_rustflags)
        .arg("build");
    if is_cargo_offline() {
        build_cmd.arg("--offline");
    }
    build_cmd.args(["--release", "--locked", "-p", "engine-service"]);

    let status = build_cmd
        .status()
        .map_err(|e| format!("staged cargo build failed: {e}"))?;
    if !status.success() {
        return Err(format!("staged cargo build exited with {status}"));
    }

    let bin_name = if is_windows {
        "admission-engine.exe"
    } else {
        "admission-engine"
    };
    let bin_path = staging_target.join("release").join(bin_name);
    let bin_str = bin_path.to_string_lossy().into_owned();

    let self_check = Command::new(&bin_str)
        .arg("--self-check")
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("invoke --self-check: {e}"))?;
    if !self_check.success() {
        return Err("release binary --self-check failed".to_owned());
    }

    let printed = capture_cmd(staging_dir, &bin_str, &["--print-compatibility"])?;
    let expected = fs::read_to_string(staging_dir.join("schema/compatibility.json"))
        .map_err(|e| e.to_string())?;
    if printed.trim() != expected.trim() {
        return Err(
            "release binary --print-compatibility drifted from schema/compatibility.json".into(),
        );
    }

    let bin_bytes = fs::read(&bin_path).map_err(|e| format!("read release binary: {e}"))?;
    scan_binary_for_host_paths(
        &bin_bytes,
        &[
            root.to_string_lossy().as_bytes(),
            staging_dir.to_string_lossy().as_bytes(),
            cargo_home.to_string_lossy().as_bytes(),
            rustup_home.to_string_lossy().as_bytes(),
            b"/home/",
            b"/Users/",
        ],
    )?;

    Ok((bin_name.to_owned(), bin_bytes))
}

fn resolve_toolchain_homes() -> (PathBuf, PathBuf) {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent-home"));
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cargo"));
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".rustup"));
    (cargo_home, rustup_home)
}
