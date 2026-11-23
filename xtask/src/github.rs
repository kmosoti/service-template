//! Declarative GitHub governance plan: 3 dedicated Environments (`main-release`,
//! `tag-release`, `stable-publish`) and 5 surgical Repository Rulesets.

use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::github_api::{
    align_develop_agent_branch, delete_obsolete_rulesets, gh_api_json, gh_api_paginated_array,
    reconcile_env_policies, resolve_authenticated_user_id, verify_repo_reachable,
};
use crate::github_readback::fetch_and_verify_readback;

#[derive(Debug, Clone, Serialize)]
pub struct GithubSetupPlan {
    pub repository: String,
    pub reviewer_user_id: u64,
    pub main_pr_approvals: u8,
    pub repo_patch: Value,
    pub main_release_environment: Value,
    pub tag_release_environment: Value,
    pub tag_release_policies: Vec<Value>,
    pub stable_publish_environment: Value,
    pub stable_publish_policies: Vec<Value>,
    pub integration_branch: String,
    pub rulesets: Vec<Value>,
}

pub fn run_github_setup(
    root: &Path,
    repo: &str,
    main_pr_approvals: u8,
    dry_run: bool,
) -> Result<(), String> {
    validate_repo_slug(repo)?;
    if dry_run {
        let plan = build_setup_plan(repo, 1, main_pr_approvals)?;
        let rendered = serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?;
        println!("{rendered}");
        return Ok(());
    }

    let reviewer_id = resolve_authenticated_user_id(root)?;
    let repo_node_id = verify_repo_reachable(root, repo)?;
    let plan = build_setup_plan(repo, reviewer_id, main_pr_approvals)?;

    // 1. Direct ref check & atomic expectedOid alignment of fresh-clone `develop-agent` before rulesets.
    align_develop_agent_branch(root, repo, &repo_node_id, &plan.integration_branch)?;

    // 2. Repository merge settings.
    gh_api_json(root, "PATCH", &format!("repos/{repo}"), &plan.repo_patch)?;

    // 3. Dedicated environments + exact custom branch/tag policy reconciliation (pruning stale extras).
    gh_api_json(
        root,
        "PUT",
        &format!("repos/{repo}/environments/main-release"),
        &plan.main_release_environment,
    )?;
    gh_api_json(
        root,
        "PUT",
        &format!("repos/{repo}/environments/tag-release"),
        &plan.tag_release_environment,
    )?;
    reconcile_env_policies(root, repo, "tag-release", &plan.tag_release_policies)?;

    gh_api_json(
        root,
        "PUT",
        &format!("repos/{repo}/environments/stable-publish"),
        &plan.stable_publish_environment,
    )?;
    reconcile_env_policies(root, repo, "stable-publish", &plan.stable_publish_policies)?;

    // 4. Paginated upsert of all 5 rulesets & cleanup of superseded single-tag ruleset names.
    let existing = gh_api_paginated_array(root, &format!("repos/{repo}/rulesets"))?;
    delete_obsolete_rulesets(root, repo, &existing)?;
    for ruleset in &plan.rulesets {
        let name = ruleset["name"].as_str().unwrap_or_default();
        let existing_id = existing.iter().find_map(|r| {
            if r.get("name").and_then(Value::as_str) == Some(name) {
                r.get("id").and_then(Value::as_u64)
            } else {
                None
            }
        });
        match existing_id {
            Some(id) => gh_api_json(root, "PUT", &format!("repos/{repo}/rulesets/{id}"), ruleset)?,
            None => gh_api_json(root, "POST", &format!("repos/{repo}/rulesets"), ruleset)?,
        }
    }

    // 5. Typed JSON readback verification of environments, policies, and all 5 rulesets.
    fetch_and_verify_readback(root, repo, &plan)?;
    println!("ok: configured and readback-verified 3 environments & 5 rulesets for {repo}");
    Ok(())
}

pub fn build_setup_plan(
    repo: &str,
    reviewer_user_id: u64,
    main_pr_approvals: u8,
) -> Result<GithubSetupPlan, String> {
    validate_repo_slug(repo)?;
    let reviewers = json!([{ "type": "User", "id": reviewer_user_id }]);

    let repo_patch = json!({
        "allow_merge_commit": true,
        "allow_squash_merge": false,
        "allow_rebase_merge": true,
        "delete_branch_on_merge": true,
        "allow_auto_merge": true
    });

    let main_release_environment = json!({
        "prevent_self_review": false,
        "reviewers": reviewers,
        "deployment_branch_policy": null
    });

    let custom_policy_env = json!({
        "prevent_self_review": false,
        "reviewers": reviewers,
        "deployment_branch_policy": {
            "protected_branches": false,
            "custom_branch_policies": true
        }
    });

    let rulesets = vec![
        branch_ruleset(
            "service-template-main-protection",
            &["refs/heads/main"],
            &["merge"],
            main_pr_approvals,
            false,
            &["main-release"],
            &["gates", "main-release-gate"],
        ),
        branch_ruleset(
            "service-template-agent-protection",
            &["refs/heads/develop-agent"],
            &["rebase"],
            0,
            true,
            &[],
            &["gates"],
        ),
        branch_ruleset(
            "service-template-maintenance-protection",
            &["refs/heads/release/*"],
            &["rebase"],
            main_pr_approvals,
            true,
            &["main-release"],
            &["gates", "main-release-gate"],
        ),
        json!({
            "name": "service-template-tag-immutability",
            "target": "tag",
            "enforcement": "active",
            "bypass_actors": [],
            "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
            "rules": [
                { "type": "update" },
                { "type": "deletion" },
                { "type": "non_fast_forward" }
            ]
        }),
        json!({
            "name": "service-template-tag-creation",
            "target": "tag",
            "enforcement": "active",
            "bypass_actors": [
                { "actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always" }
            ],
            "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
            "rules": [{ "type": "creation" }]
        }),
    ];

    Ok(GithubSetupPlan {
        repository: repo.to_owned(),
        reviewer_user_id,
        main_pr_approvals,
        repo_patch,
        main_release_environment,
        tag_release_environment: custom_policy_env.clone(),
        tag_release_policies: vec![json!({ "name": "main", "type": "branch" })],
        stable_publish_environment: custom_policy_env,
        stable_publish_policies: vec![json!({ "name": "v*", "type": "tag" })],
        integration_branch: "develop-agent".to_owned(),
        rulesets,
    })
}

fn branch_ruleset(
    name: &str,
    includes: &[&str],
    merge_methods: &[&str],
    approvals: u8,
    linear_history: bool,
    deployments: &[&str],
    checks: &[&str],
) -> Value {
    let mut rules = vec![
        json!({ "type": "deletion" }),
        json!({ "type": "non_fast_forward" }),
    ];
    if linear_history {
        rules.push(json!({ "type": "required_linear_history" }));
    }
    rules.push(json!({
        "type": "pull_request",
        "parameters": {
            "allowed_merge_methods": merge_methods,
            "dismiss_stale_reviews_on_push": true,
            "require_code_owner_review": false,
            "require_last_push_approval": false,
            "required_approving_review_count": approvals,
            "required_review_thread_resolution": true
        }
    }));
    if !deployments.is_empty() {
        rules.push(json!({
            "type": "required_deployments",
            "parameters": { "required_deployment_environments": deployments }
        }));
    }
    let check_objs: Vec<Value> = checks.iter().map(|c| json!({ "context": c })).collect();
    let strict_checks = name != "service-template-main-protection";
    rules.push(json!({
        "type": "required_status_checks",
        "parameters": {
            "strict_required_status_checks_policy": strict_checks,
            "required_status_checks": check_objs
        }
    }));

    json!({
        "name": name,
        "target": "branch",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": { "ref_name": { "include": includes, "exclude": [] } },
        "rules": rules
    })
}

pub fn validate_repo_slug(repo: &str) -> Result<(), String> {
    let Some((owner, name)) = repo.split_once('/') else {
        return Err(format!("invalid repository `{repo}` (expected OWNER/REPO)"));
    };
    let ok_part = |s: &str| {
        !s.is_empty()
            && !s.contains('/')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if !ok_part(owner) || !ok_part(name) {
        return Err(format!("invalid repository slug `{repo}`"));
    }
    Ok(())
}
