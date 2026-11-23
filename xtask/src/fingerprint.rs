//! Full pre/post `InputFingerprint` (`HEAD`, `HEAD^{tree}`, sorted refs digest,
//! dirty file content SHA-256s, lockfile, toolchain, and compatibility schema).

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::util::{capture_cmd, capture_cmd_raw, sha256_bytes, sha256_file};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFingerprint {
    pub head_commit: String,
    pub head_tree: String,
    pub refs_sha256: String,
    pub dirty_files_sha256: String,
    pub tree_hash: String,
    pub worktree_dirty: bool,
    pub cargo_lock_sha256: String,
    pub rust_toolchain_sha256: String,
    pub compatibility_sha256: String,
    pub rustc_version: String,
}

pub fn git_porcelain_all(root: &Path) -> Result<String, String> {
    if !root.join(".git").exists() {
        return Ok("uninitialized".to_owned());
    }
    capture_cmd_raw(
        root,
        "git",
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
}

pub fn compute_fingerprint(root: &Path) -> Result<InputFingerprint, String> {
    let rustc_version = capture_cmd(root, "rustc", &["--version"])?;
    let cargo_lock_sha256 = sha256_file(&root.join("Cargo.lock"))?;
    let rust_toolchain_sha256 = sha256_file(&root.join("rust-toolchain.toml"))?;
    let compatibility_sha256 = sha256_file(&root.join("schema/compatibility.json"))?;

    if !root.join(".git").exists() {
        return Ok(InputFingerprint {
            head_commit: "uncommitted".to_owned(),
            head_tree: "uncommitted-tree".to_owned(),
            refs_sha256: "uninitialized".to_owned(),
            dirty_files_sha256: "uninitialized".to_owned(),
            tree_hash: "dirty:uninitialized".to_owned(),
            worktree_dirty: true,
            cargo_lock_sha256,
            rust_toolchain_sha256,
            compatibility_sha256,
            rustc_version,
        });
    }

    let head_commit = capture_cmd(root, "git", &["rev-parse", "HEAD"])?;
    let head_tree = capture_cmd(root, "git", &["rev-parse", "HEAD^{tree}"])?;
    let refs_sha256 = compute_sorted_refs_digest(root)?;
    let porcelain = git_porcelain_all(root)?;
    let worktree_dirty = !porcelain.trim().is_empty();
    let dirty_files_sha256 = compute_dirty_contents_digest(root, &porcelain);
    let tree_hash = if worktree_dirty {
        format!("dirty:{dirty_files_sha256}")
    } else {
        head_tree.clone()
    };

    Ok(InputFingerprint {
        head_commit,
        head_tree,
        refs_sha256,
        dirty_files_sha256,
        tree_hash,
        worktree_dirty,
        cargo_lock_sha256,
        rust_toolchain_sha256,
        compatibility_sha256,
        rustc_version,
    })
}

fn compute_sorted_refs_digest(root: &Path) -> Result<String, String> {
    let raw = capture_cmd_raw(root, "git", &["show-ref"]).unwrap_or_default();
    let mut lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.sort_by_key(|line| line.split_whitespace().nth(1).unwrap_or(""));
    let joined = lines.join("\n");
    Ok(sha256_bytes(joined.as_bytes()))
}

pub fn compute_dirty_contents_digest(root: &Path, porcelain: &str) -> String {
    let mut entries: Vec<&str> = porcelain.lines().filter(|l| l.len() > 3).collect();
    entries.sort_unstable();
    let mut hasher = Sha256::new();
    for line in entries {
        let status = &line[..2];
        let raw_rel = line[3..].trim();
        let rel = raw_rel.rsplit(" -> ").next().unwrap_or(raw_rel);
        hasher.update(status.as_bytes());
        hasher.update(b"\0");
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        match fs::read(root.join(rel)) {
            Ok(bytes) => hasher.update(sha256_bytes(&bytes).as_bytes()),
            Err(_) => hasher.update(b"MISSING"),
        }
        hasher.update(b"\n");
    }
    sha256_bytes(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::compute_dirty_contents_digest;

    #[test]
    fn dirty_digest_tracks_file_content_mutations_at_identical_status() {
        let dir = std::env::temp_dir().join(format!("fp-m4-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("work.txt");

        fs::write(&f, "alpha\n").unwrap();
        let d1 = compute_dirty_contents_digest(&dir, " M work.txt\n");
        fs::write(&f, "beta\n").unwrap();
        let d2 = compute_dirty_contents_digest(&dir, " M work.txt\n");
        assert_ne!(d1, d2);

        let _ = fs::remove_dir_all(&dir);
    }
}
