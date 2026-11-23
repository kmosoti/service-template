//! Unit and temp-repo tests for strict bottom-trailer `Trust-Boundary` checks (`H6` + `H8`).

use std::fs;
use std::process::Command;

use crate::trust_boundary::{
    CommitRecord, check_trust_boundary, has_bottom_verifier_trailer, verify_commit_stack,
};

#[test]
fn parses_bottom_git_trailer_strictly_and_rejects_empty_or_evil_merge_bypasses() {
    assert!(has_bottom_verifier_trailer(
        "test(core): add spec\n\nTrust-Boundary: verifier\nSigned-off-by: Dev <d@example.com>\n"
    ));
    // Body mention followed by non-trailer prose MUST be rejected!
    assert!(!has_bottom_verifier_trailer(
        "test(core): add spec\n\nTrust-Boundary: verifier\n\nThis trailing prose invalidates trailer position.\n"
    ));
    // Subject-only line without preceding blank paragraph MUST be rejected!
    assert!(!has_bottom_verifier_trailer("Trust-Boundary: verifier\n"));

    // Claiming `Trust-Boundary: verifier` while touching only README.md -> rejected!
    let fake_verifier = vec![
        CommitRecord {
            sha: "aaaaaaaa".into(),
            parent_count: 1,
            message: "docs: pretend verifier\n\nTrust-Boundary: verifier\n".into(),
            changed_paths: vec!["README.md".into()],
            unilateral_merge_paths: vec![],
        },
        CommitRecord {
            sha: "bbbbbbbb".into(),
            parent_count: 1,
            message: "feat: change codec\n".into(),
            changed_paths: vec!["crates/engine-core/src/codec.rs".into()],
            unilateral_merge_paths: vec![],
        },
    ];
    assert!(verify_commit_stack(&fake_verifier, true).is_err());

    // Two contiguous verified pairs `[V1, I1, V2, I2]` pass cleanly!
    let two_pairs = vec![
        CommitRecord {
            sha: "11111111".into(),
            parent_count: 1,
            message: "test: pair 1\n\nTrust-Boundary: verifier\n".into(),
            changed_paths: vec!["crates/engine-core/tests/snapshot_spec.rs".into()],
            unilateral_merge_paths: vec![],
        },
        CommitRecord {
            sha: "22222222".into(),
            parent_count: 1,
            message: "feat: pair 1\n".into(),
            changed_paths: vec!["crates/engine-core/src/codec.rs".into()],
            unilateral_merge_paths: vec![],
        },
        CommitRecord {
            sha: "33333333".into(),
            parent_count: 1,
            message: "test: pair 2\n\nTrust-Boundary: verifier\n".into(),
            changed_paths: vec!["schema/compatibility.json".into()],
            unilateral_merge_paths: vec![],
        },
        CommitRecord {
            sha: "44444444".into(),
            parent_count: 1,
            message: "feat: pair 2\n".into(),
            changed_paths: vec!["crates/engine-core/src/compat.rs".into()],
            unilateral_merge_paths: vec![],
        },
    ];
    assert!(verify_commit_stack(&two_pairs, true).is_ok());

    // Evil merge sneaking unilateral verifier + impl edits -> rejected!
    let evil_merge = vec![CommitRecord {
        sha: "99999999".into(),
        parent_count: 2,
        message: "Merge pull request #1\n".into(),
        changed_paths: vec![],
        unilateral_merge_paths: vec![
            "crates/engine-core/tests/snapshot_spec.rs".into(),
            "crates/engine-core/src/codec.rs".into(),
        ],
    }];
    assert!(verify_commit_stack(&evil_merge, true).is_err());
}

#[test]
fn allows_multi_pr_branch_history_in_temp_git_repo() {
    let dir = std::env::temp_dir().join(format!("tb-h6-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("crates/engine-core/src")).unwrap();
    fs::create_dir_all(dir.join("crates/engine-core/tests")).unwrap();

    let git = |args: &[&str]| {
        let st = Command::new("git")
            .current_dir(&dir)
            .args(args)
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);

    fs::write(dir.join("crates/engine-core/src/engine.rs"), "// v0\n").unwrap();
    fs::write(dir.join("crates/engine-core/tests/spec.rs"), "// t0\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-m", "feat: scaffold"]);

    // PR 1: pure bugfix impl commit
    fs::write(dir.join("crates/engine-core/src/engine.rs"), "// v1 fix\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-m", "fix: edge case"]);

    // PR 2: commit 2 (verifier) + commit 3 (impl)
    fs::write(dir.join("crates/engine-core/tests/spec.rs"), "// t1\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-m", "test: add spec\n\nTrust-Boundary: verifier"]);

    fs::write(dir.join("crates/engine-core/src/codec.rs"), "// c1\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-m", "feat: codec update"]);

    assert!(check_trust_boundary(&dir, Some("HEAD~2")).is_ok());
    assert!(check_trust_boundary(&dir, None).is_ok());

    let _ = fs::remove_dir_all(&dir);
}
