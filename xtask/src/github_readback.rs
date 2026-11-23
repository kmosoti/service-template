//! Typed JSON readback verification for GitHub Environments, custom deployment
//! policies, and Repository Rulesets (`M2`).

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::github::GithubSetupPlan;
use crate::github_api::{fetch_env_policies, gh_api_get_required, gh_api_paginated_array};

#[derive(Debug, Deserialize)]
pub struct ReadbackEnvironment {
    #[serde(default)]
    pub protection_rules: Vec<ReadbackProtectionRule>,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackProtectionRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub reviewers: Vec<ReadbackReviewerEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackReviewerEntry {
    pub reviewer: ReadbackActor,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackActor {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackPolicy {
    pub name: String,
    #[serde(rename = "type", default)]
    pub policy_type: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackBypassActor {
    pub actor_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackRuleset {
    pub name: String,
    pub enforcement: String,
    #[serde(default)]
    pub bypass_actors: Vec<ReadbackBypassActor>,
    #[serde(default)]
    pub rules: Vec<ReadbackRule>,
}

#[derive(Debug, Deserialize)]
pub struct ReadbackRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub parameters: Value,
}

pub struct ReadbackBundle {
    pub main_release_env: ReadbackEnvironment,
    pub tag_release_env: ReadbackEnvironment,
    pub tag_release_policies: Vec<ReadbackPolicy>,
    pub stable_publish_env: ReadbackEnvironment,
    pub stable_publish_policies: Vec<ReadbackPolicy>,
    pub rulesets: Vec<ReadbackRuleset>,
}

pub fn fetch_and_verify_readback(
    root: &Path,
    repo: &str,
    plan: &GithubSetupPlan,
) -> Result<(), String> {
    let parse_env = |name: &str| -> Result<ReadbackEnvironment, String> {
        let v = gh_api_get_required(root, &format!("repos/{repo}/environments/{name}"))?;
        serde_json::from_value(v).map_err(|e| format!("decode environment `{name}`: {e}"))
    };
    let parse_policies = |name: &str| -> Result<Vec<ReadbackPolicy>, String> {
        let vals = fetch_env_policies(
            root,
            &format!("repos/{repo}/environments/{name}/deployment-branch-policies"),
        )?;
        vals.into_iter()
            .map(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
            .collect()
    };

    let summaries = gh_api_paginated_array(root, &format!("repos/{repo}/rulesets"))?;
    let mut rulesets = Vec::new();
    for item in summaries {
        if let Some(id) = item.get("id").and_then(Value::as_u64) {
            let full = gh_api_get_required(root, &format!("repos/{repo}/rulesets/{id}"))?;
            let decoded: ReadbackRuleset =
                serde_json::from_value(full).map_err(|e| format!("decode ruleset {id}: {e}"))?;
            rulesets.push(decoded);
        }
    }

    let bundle = ReadbackBundle {
        main_release_env: parse_env("main-release")?,
        tag_release_env: parse_env("tag-release")?,
        tag_release_policies: parse_policies("tag-release")?,
        stable_publish_env: parse_env("stable-publish")?,
        stable_publish_policies: parse_policies("stable-publish")?,
        rulesets,
    };
    validate_readback_bundle(&bundle, plan)
}

pub fn validate_readback_bundle(
    bundle: &ReadbackBundle,
    plan: &GithubSetupPlan,
) -> Result<(), String> {
    for (label, env) in [
        ("main-release", &bundle.main_release_env),
        ("tag-release", &bundle.tag_release_env),
        ("stable-publish", &bundle.stable_publish_env),
    ] {
        let has_reviewer = env.protection_rules.iter().any(|r| {
            r.rule_type == "required_reviewers"
                && r.reviewers
                    .iter()
                    .any(|rev| rev.reviewer.id == plan.reviewer_user_id)
        });
        if !has_reviewer {
            return Err(format!(
                "readback mismatch: environment `{label}` missing reviewer id {}",
                plan.reviewer_user_id
            ));
        }
    }

    if bundle.tag_release_policies.len() != 1
        || bundle.tag_release_policies[0].name != "main"
        || bundle.tag_release_policies[0].policy_type != "branch"
    {
        return Err("readback mismatch: `tag-release` policies must equal [main (branch)]".into());
    }

    if bundle.stable_publish_policies.len() != 1
        || bundle.stable_publish_policies[0].name != "v*"
        || bundle.stable_publish_policies[0].policy_type != "tag"
    {
        return Err("readback mismatch: `stable-publish` policies must equal [v* (tag)]".into());
    }

    verify_ruleset(
        &bundle.rulesets,
        "service-template-main-protection",
        &[],
        &["main-release"],
        &["gates", "main-release-gate"],
        &["merge"],
    )?;
    verify_ruleset(
        &bundle.rulesets,
        "service-template-agent-protection",
        &[],
        &[],
        &["gates"],
        &["rebase"],
    )?;
    verify_ruleset(
        &bundle.rulesets,
        "service-template-maintenance-protection",
        &[],
        &["main-release"],
        &["gates", "main-release-gate"],
        &["rebase"],
    )?;
    verify_tag_ruleset(
        &bundle.rulesets,
        "service-template-tag-immutability",
        &[],
        &["update", "deletion", "non_fast_forward"],
    )?;
    verify_tag_ruleset(
        &bundle.rulesets,
        "service-template-tag-creation",
        &[5],
        &["creation"],
    )?;
    Ok(())
}

fn verify_ruleset(
    rulesets: &[ReadbackRuleset],
    name: &str,
    expected_bypasses: &[u64],
    expected_deployments: &[&str],
    expected_checks: &[&str],
    expected_methods: &[&str],
) -> Result<(), String> {
    let rs = rulesets
        .iter()
        .find(|r| r.name == name && r.enforcement == "active")
        .ok_or_else(|| format!("readback missing active ruleset `{name}`"))?;
    let bypass_ids: Vec<u64> = rs.bypass_actors.iter().filter_map(|b| b.actor_id).collect();
    if bypass_ids != expected_bypasses {
        return Err(format!("readback `{name}` bypass_actors = {bypass_ids:?}"));
    }

    let pr_methods: Vec<&str> = rs
        .rules
        .iter()
        .find(|r| r.rule_type == "pull_request")
        .and_then(|r| r.parameters.get("allowed_merge_methods"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if pr_methods != expected_methods {
        return Err(format!(
            "readback `{name}` allowed_merge_methods = {pr_methods:?}"
        ));
    }

    let deployments: Vec<&str> = rs
        .rules
        .iter()
        .find(|r| r.rule_type == "required_deployments")
        .and_then(|r| r.parameters.get("required_deployment_environments"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if deployments != expected_deployments {
        return Err(format!(
            "readback `{name}` required_deployments = {deployments:?}"
        ));
    }

    let checks: Vec<&str> = rs
        .rules
        .iter()
        .find(|r| r.rule_type == "required_status_checks")
        .and_then(|r| r.parameters.get("required_status_checks"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("context").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if checks != expected_checks {
        return Err(format!(
            "readback `{name}` required_status_checks = {checks:?}"
        ));
    }
    Ok(())
}

fn verify_tag_ruleset(
    rulesets: &[ReadbackRuleset],
    name: &str,
    expected_bypasses: &[u64],
    expected_rule_types: &[&str],
) -> Result<(), String> {
    let rs = rulesets
        .iter()
        .find(|r| r.name == name && r.enforcement == "active")
        .ok_or_else(|| format!("readback missing active tag ruleset `{name}`"))?;
    let bypass_ids: Vec<u64> = rs.bypass_actors.iter().filter_map(|b| b.actor_id).collect();
    if bypass_ids != expected_bypasses {
        return Err(format!("readback `{name}` bypass_actors = {bypass_ids:?}"));
    }
    for rt in expected_rule_types {
        if !rs.rules.iter().any(|r| r.rule_type == *rt) {
            return Err(format!("readback `{name}` missing rule `{rt}`"));
        }
    }
    Ok(())
}
