//! Direct HTTP-status-checked `gh api` transport, atomic `expectedOid` branch alignment,
//! and custom deployment policy pruning.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::util::capture_cmd;

pub enum ApiLookup {
    Found(Value),
    NotFound,
}

const OBSOLETE_RULESET_NAMES: &[&str] = &[
    "service-template-immutable-tags",
    "service-template-agent-and-maintenance-protection",
];

pub fn resolve_authenticated_user_id(root: &Path) -> Result<u64, String> {
    let user_val = gh_api_get_required(root, "user")?;
    user_val.get("id").and_then(Value::as_u64).ok_or_else(|| {
        "could not resolve authenticated reviewer `id` from `gh api user`".to_owned()
    })
}

pub fn verify_repo_reachable(root: &Path, repo: &str) -> Result<String, String> {
    let repo_val = gh_api_get_required(root, &format!("repos/{repo}"))?;
    repo_val
        .get("node_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("missing GraphQL `node_id` on `repos/{repo}`"))
}

pub fn align_develop_agent_branch(
    root: &Path,
    repo: &str,
    repo_node_id: &str,
    branch: &str,
) -> Result<(), String> {
    let main_ref = gh_api_get_required(root, &format!("repos/{repo}/git/ref/heads/main"))?;
    let main_sha = main_ref
        .pointer("/object/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing `main` SHA".to_owned())?;
    let main_commit = gh_api_get_required(root, &format!("repos/{repo}/git/commits/{main_sha}"))?;
    let main_tree = main_commit
        .pointer("/tree/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing `main` tree SHA".to_owned())?;

    let dev_lookup = gh_api_lookup(root, &format!("repos/{repo}/git/ref/heads/{branch}"))?;
    let dev_val = match dev_lookup {
        ApiLookup::NotFound => {
            return gh_api_json(
                root,
                "POST",
                &format!("repos/{repo}/git/refs"),
                &json!({ "ref": format!("refs/heads/{branch}"), "sha": main_sha }),
            );
        }
        ApiLookup::Found(v) => v,
    };

    let agent_sha = dev_val
        .pointer("/object/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{branch}` SHA"))?;
    if agent_sha == main_sha {
        return Ok(());
    }

    let agent_commit = gh_api_get_required(root, &format!("repos/{repo}/git/commits/{agent_sha}"))?;
    let parents = agent_commit
        .get("parents")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing `parents` array on `{agent_sha}`"))?;
    let agent_tree = agent_commit
        .pointer("/tree/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `tree.sha` on `{agent_sha}`"))?;

    if !parents.is_empty() {
        let cmp = gh_api_lookup(root, &format!("repos/{repo}/compare/main...{branch}"))?;
        if let ApiLookup::Found(v) = cmp
            && v.pointer("/merge_base_commit/sha")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        {
            return Ok(());
        }
        return Err(format!(
            "refusing automatic `{branch}` alignment: `{branch}` ({agent_sha}, parents={}) has unrelated multi-commit history relative to `main` ({main_sha})",
            parents.len()
        ));
    }
    if agent_tree != main_tree {
        return Err(format!(
            "refusing automatic `{branch}` alignment: orphan root `{branch}` ({agent_sha}, tree={agent_tree}) differs from `main` ({main_sha}, tree={main_tree})"
        ));
    }

    // Atomic server-side compare-and-swap via GraphQL `updateRefs(beforeOid, afterOid)`.
    let mutation = "mutation($repoId: ID!, $refName: GitRefname!, $newOid: GitObjectID!, $oldOid: GitObjectID!) { \
        updateRefs(input: { repositoryId: $repoId, refUpdates: [{ name: $refName, afterOid: $newOid, beforeOid: $oldOid, force: true }] }) { \
            clientMutationId \
        } \
    }";
    let gql_body = json!({
        "query": mutation,
        "variables": {
            "repoId": repo_node_id,
            "refName": format!("refs/heads/{branch}"),
            "newOid": main_sha,
            "oldOid": agent_sha
        }
    });
    let resp = gh_api_json_response(root, "POST", "graphql", &gql_body)?;
    if resp.get("errors").is_some() {
        return Err(format!("GraphQL atomic `updateRefs` lease failed: {resp}"));
    }
    Ok(())
}

pub fn reconcile_env_policies(
    root: &Path,
    repo: &str,
    env_name: &str,
    desired: &[Value],
) -> Result<(), String> {
    let endpoint = format!("repos/{repo}/environments/{env_name}/deployment-branch-policies");
    let existing = fetch_env_policies(root, &endpoint)?;

    for item in &existing {
        let id = item
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("missing policy id on `{env_name}`"))?;
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let wanted = desired.iter().any(|d| {
            d.get("name").and_then(Value::as_str) == Some(name)
                && d.get("type").and_then(Value::as_str) == Some(kind)
        });
        if !wanted {
            gh_api_delete(root, &format!("{endpoint}/{id}"))?;
        }
    }

    for policy in desired {
        let name = policy
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind = policy
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let already = existing.iter().any(|p| {
            p.get("name").and_then(Value::as_str) == Some(name)
                && p.get("type").and_then(Value::as_str) == Some(kind)
        });
        if !already {
            gh_api_json(root, "POST", &endpoint, policy)?;
        }
    }
    Ok(())
}

pub fn fetch_env_policies(root: &Path, endpoint: &str) -> Result<Vec<Value>, String> {
    let raw = capture_cmd(root, "gh", &["api", "--paginate", "--slurp", endpoint])?;
    let pages: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(arr) = pages.as_array() {
        for page in arr {
            if let Some(items) = page.get("branch_policies").and_then(Value::as_array) {
                out.extend(items.iter().cloned());
            }
        }
    }
    Ok(out)
}

pub fn delete_obsolete_rulesets(root: &Path, repo: &str, existing: &[Value]) -> Result<(), String> {
    for item in existing {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if OBSOLETE_RULESET_NAMES.contains(&name)
            && let Some(id) = item.get("id").and_then(Value::as_u64)
        {
            gh_api_delete(root, &format!("repos/{repo}/rulesets/{id}"))?;
        }
    }
    Ok(())
}

pub fn gh_api_paginated_array(root: &Path, endpoint: &str) -> Result<Vec<Value>, String> {
    let raw = capture_cmd(root, "gh", &["api", "--paginate", "--slurp", endpoint])?;
    let pages: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut flat = Vec::new();
    if let Some(arr) = pages.as_array() {
        for page in arr {
            if let Some(items) = page.as_array() {
                flat.extend(items.iter().cloned());
            }
        }
    }
    Ok(flat)
}

pub fn gh_api_get_required(root: &Path, endpoint: &str) -> Result<Value, String> {
    match gh_api_lookup(root, endpoint)? {
        ApiLookup::Found(val) => Ok(val),
        ApiLookup::NotFound => Err(format!("GitHub API `{endpoint}` returned 404 Not Found")),
    }
}

pub fn gh_api_lookup(root: &Path, endpoint: &str) -> Result<ApiLookup, String> {
    let out = Command::new("gh")
        .current_dir(root)
        .args(["api", "-i", endpoint])
        .output()
        .map_err(|e| format!("failed executing `gh api -i {endpoint}`: {e}"))?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let status_code = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);

    let body = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map(|(_, b)| b.trim())
        .unwrap_or("");

    match status_code {
        200..=299 => {
            let parsed: Value = serde_json::from_str(body)
                .map_err(|e| format!("invalid JSON from `{endpoint}`: {e}"))?;
            Ok(ApiLookup::Found(parsed))
        }
        404 => Ok(ApiLookup::NotFound),
        code => Err(format!(
            "`gh api {endpoint}` failed with HTTP {code}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

pub fn gh_api_delete(root: &Path, endpoint: &str) -> Result<(), String> {
    let out = Command::new("gh")
        .current_dir(root)
        .args(["api", "--method", "DELETE", endpoint])
        .output()
        .map_err(|e| format!("failed spawning `gh api DELETE {endpoint}`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`gh api DELETE {endpoint}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

pub fn gh_api_json(root: &Path, method: &str, endpoint: &str, body: &Value) -> Result<(), String> {
    let _ = gh_api_json_response(root, method, endpoint, body)?;
    Ok(())
}

pub fn gh_api_json_response(
    root: &Path,
    method: &str,
    endpoint: &str,
    body: &Value,
) -> Result<Value, String> {
    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let mut child = Command::new("gh")
        .current_dir(root)
        .args(["api", "--method", method, endpoint, "--input", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed spawning `gh api`: {e}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(&payload).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "`gh api --method {method} {endpoint}` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    if out.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("invalid response JSON: {e}"))
}
