//! `git push` branch and `v*` release-tag protection policy for `PreToolUse` hooks.

use crate::hooks::HookEnv;

pub fn extract_git_push_args(git_args: &[String]) -> Option<&[String]> {
    let mut i = 0usize;
    while i < git_args.len() {
        match git_args[i].as_str() {
            "push" => return Some(&git_args[i + 1..]),
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            flag if flag.starts_with('-') => i += 1,
            _ => return None,
        }
    }
    None
}

pub fn check_git_push(args: &[String], env: &dyn HookEnv) -> Result<(), String> {
    let mut delete_mode = false;
    let mut positionals: Vec<&str> = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        let tok = args[i].as_str();
        match tok {
            "--all" | "--branches" | "--mirror" => {
                return Err(
                    "`git push --all/--mirror` is prohibited; push explicit feature or `develop-agent` branches"
                        .to_owned(),
                );
            }
            "--tags" | "--follow-tags" => {
                return Err(
                    "`git push --tags` is prohibited; push an explicit verified `v*` tag"
                        .to_owned(),
                );
            }
            "-d" | "--delete" => {
                delete_mode = true;
                i += 1;
            }
            "-o" | "--push-option" | "--repo" | "--receive-pack" | "--exec" => i += 2,
            flag if flag.starts_with('-') => i += 1,
            pos => {
                positionals.push(pos);
                i += 1;
            }
        }
    }

    if positionals.len() <= 1 {
        if let Some(branch) = env.current_branch()
            && is_protected_branch(&branch)
        {
            return Err(format!(
                "direct `git push` while checked out on `{branch}` is prohibited"
            ));
        }
        return Ok(());
    }

    for raw_spec in &positionals[1..] {
        let spec = raw_spec.strip_prefix('+').unwrap_or(raw_spec);
        let (src, dst, has_colon) = match spec.split_once(':') {
            Some((s, d)) => (s, d, true),
            None => (spec, spec, false),
        };

        if delete_mode || (has_colon && src.is_empty()) {
            let dst_branch = dst.strip_prefix("refs/heads/").unwrap_or(dst);
            let dst_tag = dst.strip_prefix("refs/tags/").unwrap_or(dst);
            if is_protected_branch(dst_branch) || is_v_tag(dst_tag) {
                return Err(format!(
                    "deleting protected branch or version tag `{dst}` is prohibited"
                ));
            }
            continue;
        }

        if dst == "HEAD"
            && let Some(branch) = env.current_branch()
            && is_protected_branch(&branch)
        {
            return Err(format!(
                "direct `git push` of HEAD while on protected branch `{branch}` is prohibited"
            ));
        }

        let dst_branch = dst.strip_prefix("refs/heads/").unwrap_or(dst);
        if !dst.starts_with("refs/tags/") && is_protected_branch(dst_branch) {
            return Err(format!(
                "direct `git push` targeting `{dst_branch}` is prohibited; open a pull request instead"
            ));
        }

        if let Some(tag) = extract_pushed_v_tag(src, dst)? {
            if !env.has_valid_head_receipt() {
                return Err(format!(
                    "refusing `git push` of `{tag}` without a valid `evidence/verification.json` matching `HEAD^{{tree}}`"
                ));
            }
            env.preflight_tag(tag)?;
            let is_named_local_tag = !has_colon || src == tag || src == format!("refs/tags/{tag}");
            if is_named_local_tag {
                env.local_tag_matches_head(tag)?;
            } else if src != "HEAD" {
                return Err(format!(
                    "release tag `{tag}` must be pushed from `HEAD` or local tag `{tag}`, found `{src}`"
                ));
            }
        }
    }
    Ok(())
}

fn is_protected_branch(branch: &str) -> bool {
    branch == "main" || branch.starts_with("release/")
}

fn is_v_tag(name: &str) -> bool {
    name.starts_with('v') && name.len() > 1 && name.as_bytes()[1].is_ascii_digit()
}

fn extract_pushed_v_tag<'a>(src: &'a str, dst: &'a str) -> Result<Option<&'a str>, String> {
    if let Some(tag) = dst.strip_prefix("refs/tags/") {
        if !is_v_tag(tag) {
            return Err(format!("release tag `{tag}` must follow `vX.Y.Z` naming"));
        }
        if let Some(src_tag) = src.strip_prefix("refs/tags/")
            && src_tag != tag
        {
            return Err(format!("mismatched tag refspec `{src}:{dst}`"));
        }
        return Ok(Some(tag));
    }
    if !dst.starts_with("refs/") && is_v_tag(dst) {
        return Ok(Some(dst));
    }
    if let Some(src_tag) = src.strip_prefix("refs/tags/") {
        return Err(format!(
            "pushing tag `{src_tag}` to non-tag destination `{dst}` is prohibited"
        ));
    }
    Ok(None)
}
