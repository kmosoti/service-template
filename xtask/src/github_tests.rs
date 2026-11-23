//! Unit tests for GitHub setup plans (`M1`, `H12`) and typed readback validation (`M2`).

use crate::github::build_setup_plan;
use crate::github_readback::{
    ReadbackActor, ReadbackBundle, ReadbackBypassActor, ReadbackEnvironment, ReadbackPolicy,
    ReadbackProtectionRule, ReadbackReviewerEntry, ReadbackRule, ReadbackRuleset,
    validate_readback_bundle,
};

fn make_env(reviewer_id: u64) -> ReadbackEnvironment {
    ReadbackEnvironment {
        protection_rules: vec![ReadbackProtectionRule {
            rule_type: "required_reviewers".into(),
            reviewers: vec![ReadbackReviewerEntry {
                reviewer: ReadbackActor { id: reviewer_id },
            }],
        }],
    }
}

#[test]
fn validates_five_rulesets_split_tag_protections_and_rejects_bad_readback() {
    let plan = build_setup_plan("example-org/service-template", 77, 0).expect("valid slug");
    assert_eq!(plan.main_pr_approvals, 0);
    assert_eq!(plan.rulesets.len(), 5);

    // Tag immutability has zero bypass actors; tag creation restricts bypass to role 5.
    assert_eq!(
        plan.rulesets[3]["name"],
        "service-template-tag-immutability"
    );
    assert_eq!(
        plan.rulesets[3]["bypass_actors"].as_array().unwrap().len(),
        0
    );
    assert_eq!(plan.rulesets[4]["name"], "service-template-tag-creation");
    assert_eq!(plan.rulesets[4]["bypass_actors"][0]["actor_id"], 5);

    let rulesets: Vec<ReadbackRuleset> = plan
        .rulesets
        .iter()
        .map(|v| serde_json::from_value(v.clone()).expect("decode plan ruleset"))
        .collect();

    let mut bundle = ReadbackBundle {
        main_release_env: make_env(77),
        tag_release_env: make_env(77),
        tag_release_policies: vec![ReadbackPolicy {
            name: "main".into(),
            policy_type: "branch".into(),
        }],
        stable_publish_env: make_env(77),
        stable_publish_policies: vec![ReadbackPolicy {
            name: "v*".into(),
            policy_type: "tag".into(),
        }],
        rulesets,
    };
    assert!(validate_readback_bundle(&bundle, &plan).is_ok());

    // Wrong reviewer ID on `tag-release` -> rejected!
    bundle.tag_release_env = make_env(999);
    assert!(validate_readback_bundle(&bundle, &plan).is_err());
    bundle.tag_release_env = make_env(77);

    // Stale extra deployment branch policy on `stable-publish` -> rejected!
    bundle.stable_publish_policies.push(ReadbackPolicy {
        name: "develop-agent".into(),
        policy_type: "branch".into(),
    });
    assert!(validate_readback_bundle(&bundle, &plan).is_err());
    bundle.stable_publish_policies.pop();

    // Rogue bypass actor injected onto `service-template-tag-immutability` -> rejected!
    bundle.rulesets[3]
        .bypass_actors
        .push(ReadbackBypassActor { actor_id: Some(5) });
    assert!(validate_readback_bundle(&bundle, &plan).is_err());
    bundle.rulesets[3].bypass_actors.clear();

    // Missing `required_deployments` on `service-template-maintenance-protection` -> rejected!
    bundle.rulesets[2]
        .rules
        .retain(|r: &ReadbackRule| r.rule_type != "required_deployments");
    assert!(validate_readback_bundle(&bundle, &plan).is_err());
}
