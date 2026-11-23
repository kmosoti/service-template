//! Temp-repo unit tests for `release-lineage` (`M1`) and all 5 `sync-branches` graphs (`H1`).

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::lineage::{
    SyncDrift, SyncVerdict, check_release_lineage, classify_branch_sync, parse_semver_tag,
};

fn run_git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git command");
    assert!(st.success(), "git {:?} failed in {}", args, dir.display());
}

#[test]
fn parses_semver_tags_and_enforces_exact_release_xy_lineage() {
    assert!(parse_semver_tag("v0.1.0").is_ok());
    assert!(parse_semver_tag("v1.2.3-rc.1").is_ok());
    assert!(parse_semver_tag("0.1.0").is_err());
    assert!(parse_semver_tag("v0.1.0-rc.0").is_err());

    let dir = std::env::temp_dir().join(format!("lineage-m1-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    run_git(&dir, &["init", "-b", "main"]);
    run_git(&dir, &["config", "user.name", "Test"]);
    run_git(&dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("f.txt"), "root\n").unwrap();
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-m", "feat: root"]);
    run_git(
        &dir,
        &["update-ref", "refs/remotes/origin/main", "refs/heads/main"],
    );

    assert!(check_release_lineage(&dir, "v1.2.0", "HEAD").is_ok());
    assert!(check_release_lineage(&dir, "v1.2.1", "HEAD").is_err());

    run_git(
        &dir,
        &["update-ref", "refs/remotes/origin/release/1.1", "HEAD"],
    );
    assert!(check_release_lineage(&dir, "v1.2.1", "HEAD").is_err());

    run_git(
        &dir,
        &["update-ref", "refs/remotes/origin/release/1.2", "HEAD"],
    );
    assert!(check_release_lineage(&dir, "v1.2.1", "HEAD").is_ok());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn classify_branch_sync_verifies_all_five_graph_states() {
    let dir = std::env::temp_dir().join(format!("sync-h1-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    run_git(&dir, &["init", "-b", "main"]);
    run_git(&dir, &["config", "user.name", "Test"]);
    run_git(&dir, &["config", "user.email", "test@example.com"]);
    fs::write(dir.join("a.txt"), "v0\n").unwrap();
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-m", "feat: v0"]);

    // 1. ExactMatch
    run_git(&dir, &["branch", "develop-agent", "main"]);
    assert_eq!(
        classify_branch_sync(&dir, "refs/heads/main", "refs/heads/develop-agent").unwrap(),
        Ok(SyncVerdict::ExactMatch)
    );

    // 2. PromotedAncestor (`develop-agent` merged into `main` via `--no-ff`, equal trees)
    run_git(&dir, &["checkout", "develop-agent"]);
    fs::write(dir.join("a.txt"), "v1\n").unwrap();
    run_git(&dir, &["commit", "-am", "feat: d1"]);
    run_git(&dir, &["checkout", "main"]);
    run_git(
        &dir,
        &["merge", "--no-ff", "develop-agent", "-m", "Merge D1"],
    );
    assert_eq!(
        classify_branch_sync(&dir, "refs/heads/main", "refs/heads/develop-agent").unwrap(),
        Ok(SyncVerdict::PromotedAncestor)
    );

    // 3. AgentAheadOfPromotedBase (`develop-agent` advances to D2 -> D3 while `main` merges D2)
    run_git(&dir, &["checkout", "develop-agent"]);
    run_git(&dir, &["merge", "--ff-only", "main"]);
    fs::write(dir.join("b.txt"), "d2\n").unwrap();
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-m", "feat: d2"]);
    run_git(&dir, &["branch", "promoted-d2"]);
    fs::write(dir.join("c.txt"), "d3\n").unwrap();
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-m", "feat: d3"]);
    run_git(&dir, &["checkout", "main"]);
    run_git(&dir, &["merge", "--no-ff", "promoted-d2", "-m", "Merge D2"]);
    assert_eq!(
        classify_branch_sync(&dir, "refs/heads/main", "refs/heads/develop-agent").unwrap(),
        Ok(SyncVerdict::AgentAheadOfPromotedBase)
    );

    // 4. MainHasUnbackportedChanges (`main` gains a direct hotfix commit not on `develop-agent`)
    fs::write(dir.join("hotfix.txt"), "fix\n").unwrap();
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-m", "fix: main hotfix"]);
    assert_eq!(
        classify_branch_sync(&dir, "refs/heads/main", "refs/heads/develop-agent").unwrap(),
        Err(SyncDrift::MainHasUnbackportedChanges)
    );

    // 5. NonAncestorSameTree (orphan branch with identical tree, no ancestry)
    run_git(&dir, &["checkout", "--orphan", "orphan-dev"]);
    run_git(&dir, &["commit", "-m", "orphan identical tree"]);
    assert_eq!(
        classify_branch_sync(&dir, "refs/heads/main", "refs/heads/orphan-dev").unwrap(),
        Err(SyncDrift::NonAncestorSameTree)
    );

    let _ = fs::remove_dir_all(&dir);
}
