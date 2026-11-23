//! Scoped two-commit `Trust-Boundary: verifier` enforcement (`H6` + `H8`).

use std::collections::BTreeSet;
use std::env;
use std::path::Path;

use crate::util::capture_cmd;

#[derive(Debug, Clone)]
pub struct CommitRecord {
    pub sha: String,
    pub parent_count: usize,
    pub message: String,
    pub changed_paths: Vec<String>,
    pub unilateral_merge_paths: Vec<String>,
}

enum CommitSelection {
    SingleHead(String),
    Range {
        spec: String,
        is_pr_stack: bool,
        merge_head: Option<String>,
    },
}

pub fn check_trust_boundary(root: &Path, explicit_base: Option<&str>) -> Result<String, String> {
    if !root.join(".git").exists() {
        return Ok("git not initialized yet (pre-commit workspace check)".to_owned());
    }

    let selection = resolve_selection(root, explicit_base)?;
    let (mut shas, is_pr_stack) = match selection {
        CommitSelection::SingleHead(sha) => (vec![sha], false),
        CommitSelection::Range {
            spec,
            is_pr_stack,
            merge_head,
        } => {
            let out = capture_cmd(root, "git", &["rev-list", "--reverse", &spec])?;
            let mut list: Vec<String> = out
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if let Some(m) = merge_head {
                list.push(m);
            }
            (list, is_pr_stack)
        }
    };

    if shas.is_empty() {
        return Ok("0 commits in range (up to date with base)".to_owned());
    }

    let mut commits = Vec::with_capacity(shas.len());
    for sha in shas.drain(..) {
        let parents_line = capture_cmd(root, "git", &["rev-list", "--parents", "-n", "1", &sha])?;
        let tokens: Vec<&str> = parents_line.split_whitespace().collect();
        let parent_count = tokens.len().saturating_sub(1);
        let message = capture_cmd(root, "git", &["show", "-s", "--format=%B", &sha])?;

        let (changed_paths, unilateral_merge_paths) = if parent_count <= 1 {
            let diff_out = capture_cmd(
                root,
                "git",
                &["diff-tree", "--no-commit-id", "--name-only", "-r", &sha],
            )?;
            let paths = diff_out
                .lines()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            (paths, Vec::new())
        } else {
            let p1 = tokens[1];
            let p2 = tokens[2];
            let d1: BTreeSet<String> =
                capture_cmd(root, "git", &["diff", "--name-only", p1, &sha])?
                    .lines()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            let d2: BTreeSet<String> =
                capture_cmd(root, "git", &["diff", "--name-only", p2, &sha])?
                    .lines()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            let unilateral = d1.intersection(&d2).cloned().collect();
            (Vec::new(), unilateral)
        };

        commits.push(CommitRecord {
            sha,
            parent_count,
            message,
            changed_paths,
            unilateral_merge_paths,
        });
    }

    verify_commit_stack(&commits, is_pr_stack)
}

fn resolve_selection(root: &Path, explicit_base: Option<&str>) -> Result<CommitSelection, String> {
    if let Some(base) = explicit_base {
        return Ok(CommitSelection::Range {
            spec: format!("{base}..HEAD"),
            is_pr_stack: true,
            merge_head: None,
        });
    }
    if let Ok(gh_base) = env::var("GITHUB_BASE_REF") {
        let trimmed = gh_base.trim();
        if !trimmed.is_empty() {
            let remote_ref = format!("refs/remotes/origin/{trimmed}");
            if capture_cmd(root, "git", &["rev-parse", "--verify", &remote_ref]).is_ok() {
                return Ok(CommitSelection::Range {
                    spec: format!("{remote_ref}..HEAD"),
                    is_pr_stack: true,
                    merge_head: None,
                });
            }
        }
    }

    let parents_line = capture_cmd(root, "git", &["rev-list", "--parents", "-n", "1", "HEAD"])?;
    let parts: Vec<&str> = parents_line.split_whitespace().collect();
    let head_sha = (*parts.first().ok_or("missing HEAD sha")?).to_owned();
    let parent_count = parts.len().saturating_sub(1);

    if parent_count == 0 {
        Ok(CommitSelection::SingleHead(head_sha))
    } else if parent_count >= 2 {
        Ok(CommitSelection::Range {
            spec: "HEAD^1..HEAD^2".to_owned(),
            is_pr_stack: true,
            merge_head: Some(head_sha),
        })
    } else {
        Ok(CommitSelection::Range {
            spec: "HEAD~1..HEAD".to_owned(),
            is_pr_stack: false,
            merge_head: None,
        })
    }
}

pub fn verify_commit_stack(commits: &[CommitRecord], is_pr_stack: bool) -> Result<String, String> {
    let mut segment_verified = false;
    let mut segment_has_unverified_impl = false;
    let mut checked = 0usize;

    for commit in commits {
        let short_sha = &commit.sha[..commit.sha.len().min(8)];
        if commit.parent_count == 0 {
            checked += 1;
            continue;
        }

        if commit.parent_count >= 2 {
            let uni_ver = commit
                .unilateral_merge_paths
                .iter()
                .any(|p| is_verifier_path(p));
            let uni_impl = commit
                .unilateral_merge_paths
                .iter()
                .any(|p| is_impl_path(p));
            if uni_ver && uni_impl {
                return Err(format!(
                    "merge commit {short_sha} introduces mixed verifier + implementation conflict resolution"
                ));
            }
            if uni_ver {
                return Err(format!(
                    "merge commit {short_sha} unilaterally modifies verifier surfaces outside merged parents"
                ));
            }
            checked += 1;
            continue;
        }

        let touches_verifier = commit.changed_paths.iter().any(|p| is_verifier_path(p));
        let touches_impl = commit.changed_paths.iter().any(|p| is_impl_path(p));
        let touches_contract_impl = commit.changed_paths.iter().any(|p| {
            matches!(
                p.as_str(),
                "crates/engine-core/src/compat.rs" | "crates/engine-core/src/codec.rs"
            )
        });
        let has_verifier_trailer = has_bottom_verifier_trailer(&commit.message);

        if touches_verifier && touches_impl {
            return Err(format!(
                "commit {short_sha} mixes verifier surfaces and production `src/` in one commit"
            ));
        }
        if touches_verifier && !has_verifier_trailer {
            return Err(format!(
                "commit {short_sha} modifies verifier paths without bottom `Trust-Boundary: verifier` trailer"
            ));
        }
        if has_verifier_trailer && touches_impl {
            return Err(format!(
                "commit {short_sha} declares `Trust-Boundary: verifier` while modifying production `src/`"
            ));
        }
        if has_verifier_trailer && !touches_verifier {
            return Err(format!(
                "commit {short_sha} declares `Trust-Boundary: verifier` without modifying any verifier file"
            ));
        }

        if is_pr_stack {
            if has_verifier_trailer {
                if segment_has_unverified_impl {
                    return Err(format!(
                        "commit {short_sha} places `Trust-Boundary: verifier` after an unverified implementation commit"
                    ));
                }
                segment_verified = true;
            } else if touches_impl {
                if touches_contract_impl && !segment_verified {
                    return Err(format!(
                        "commit {short_sha} modifies contract/codec `src/` without a preceding `Trust-Boundary: verifier` commit"
                    ));
                }
                if !segment_verified {
                    segment_has_unverified_impl = true;
                }
            }
        }
        checked += 1;
    }

    Ok(format!(
        "{checked} commit(s) satisfy Trust-Boundary separation"
    ))
}

fn is_verifier_path(path: &str) -> bool {
    path.starts_with("xtask/")
        || path.starts_with(".github/")
        || path.starts_with("schema/")
        || (path.starts_with("crates/") && path.contains("/tests/"))
}

fn is_impl_path(path: &str) -> bool {
    path.starts_with("crates/") && path.contains("/src/")
}

/// Parses `Trust-Boundary: verifier` strictly from the final Git trailer paragraph.
pub fn has_bottom_verifier_trailer(message: &str) -> bool {
    let trimmed = message.trim_end();
    let Some((_, bottom_block)) = trimmed.rsplit_once("\n\n") else {
        return false;
    };
    let lines: Vec<&str> = bottom_block.lines().collect();
    if lines.is_empty() || !lines.iter().all(|l| is_git_trailer_line(l)) {
        return false;
    }
    lines
        .iter()
        .any(|line| line.trim().eq_ignore_ascii_case("Trust-Boundary: verifier"))
}

fn is_git_trailer_line(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return true;
    }
    let Some((key, val)) = line.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && !val.trim().is_empty()
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
