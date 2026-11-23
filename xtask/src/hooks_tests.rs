//! Unit tests for 4-host `PreToolUse` hook evaluation and compound shell tokenization.

use serde_json::{Value, json};

use crate::hooks::{HookEnv, HookFormat, RepoHookEnv, evaluate_pre_tool_use};
use crate::util::workspace_root;

struct MockHookEnv {
    branch: Option<&'static str>,
    valid_receipt: bool,
    valid_tag: &'static str,
    local_tag_at_head: Option<&'static str>,
}

impl HookEnv for MockHookEnv {
    fn current_branch(&self) -> Option<String> {
        self.branch.map(ToOwned::to_owned)
    }

    fn has_valid_head_receipt(&self) -> bool {
        self.valid_receipt
    }

    fn preflight_tag(&self, tag: &str) -> Result<(), String> {
        if tag == self.valid_tag {
            Ok(())
        } else {
            Err(format!("tag `{tag}` failed preflight version/notes parity"))
        }
    }

    fn local_tag_matches_head(&self, tag: &str) -> Result<(), String> {
        if self.local_tag_at_head == Some(tag) {
            Ok(())
        } else {
            Err(format!("local tag `{tag}` does not exist at HEAD"))
        }
    }
}

const ALL_FORMATS: [HookFormat; 4] = [
    HookFormat::Antigravity,
    HookFormat::Claude,
    HookFormat::Codex,
    HookFormat::Copilot,
];

#[test]
fn antigravity_emits_decision_allow_while_claude_codex_copilot_emit_empty_object() {
    let env = MockHookEnv {
        branch: Some("develop-agent"),
        valid_receipt: true,
        valid_tag: "v0.1.0",
        local_tag_at_head: Some("v0.1.0"),
    };

    for fmt in ALL_FORMATS {
        for bad_input in ["", "   ", "{not-json", r#"{"toolArgs":"{broken"}"#] {
            let out = evaluate_pre_tool_use(bad_input, fmt, &env);
            assert!(!out.allowed);
            assert!(out.stdout_json.contains("deny"));
        }

        let pwd_out = evaluate_pre_tool_use(
            r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"pwd"}}}"#,
            fmt,
            &env,
        );
        assert!(pwd_out.allowed);
        let parsed: Value = serde_json::from_str(&pwd_out.stdout_json).unwrap();
        match fmt {
            HookFormat::Antigravity => assert_eq!(parsed, json!({ "decision": "allow" })),
            HookFormat::Claude | HookFormat::Codex | HookFormat::Copilot => {
                assert_eq!(parsed, json!({}));
                assert!(!pwd_out.stdout_json.contains("allow"));
            }
        }
    }
}

#[test]
fn catches_subshell_wrapper_and_path_prefixed_bypasses_and_verifies_codex_schema() {
    let env = MockHookEnv {
        branch: Some("main"),
        valid_receipt: true,
        valid_tag: "v0.1.0",
        local_tag_at_head: None,
    };

    for blocked_cmd in [
        "gh pr merge 1 --squash",
        "(git push origin main)",
        "{ /usr/bin/git push origin release/0.1; }",
        "bash -lc 'git push origin main'",
        "echo $(gh pr merge 42 --admin --rebase)",
        "`git push origin v9.9.9`",
        "env FOO=1 command git push origin v0.1.0",
        "git push origin HEAD:refs/heads/main",
        "gh api repos/org/svc/actions/runs/9/pending_deployments -f state=approved",
        "gh run review 9 --approve --repo org/svc # tag-release",
    ] {
        let payload = json!({
            "toolCall": { "name": "run_command", "args": { "CommandLine": blocked_cmd } }
        })
        .to_string();
        for fmt in ALL_FORMATS {
            let res = evaluate_pre_tool_use(&payload, fmt, &env);
            assert!(!res.allowed, "expected `{blocked_cmd}` to be denied");
            let parsed: Value = serde_json::from_str(&res.stdout_json).unwrap();
            match fmt {
                HookFormat::Antigravity => assert_eq!(parsed["decision"], "deny"),
                HookFormat::Claude | HookFormat::Codex => {
                    assert!(parsed.get("decision").is_none());
                    assert_eq!(parsed["hookSpecificOutput"]["permissionDecision"], "deny");
                }
                HookFormat::Copilot => assert_eq!(parsed["permissionDecision"], "deny"),
            }
        }
    }

    let root = workspace_root();
    let real_env = RepoHookEnv { root: &root };
    let rogue_tag =
        r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"git push origin v9.9.9"}}}"#;
    assert!(!evaluate_pre_tool_use(rogue_tag, HookFormat::Antigravity, &real_env).allowed);
}

#[test]
fn allows_verified_local_tag_at_head_and_copilot_nested_tool_args() {
    let env = MockHookEnv {
        branch: Some("develop-agent"),
        valid_receipt: true,
        valid_tag: "v0.1.0",
        local_tag_at_head: Some("v0.1.0"),
    };

    for allowed_cmd in [
        "git push origin develop-agent",
        "git push origin feat/expand-v2",
        "git push origin v0.1.0",
        "git push origin HEAD:refs/tags/v0.1.0",
        "gh pr merge 42 --rebase",
    ] {
        let copilot_payload = json!({
            "toolName": "bash",
            "toolArgs": json!({ "command": allowed_cmd }).to_string(),
        })
        .to_string();
        let res = evaluate_pre_tool_use(&copilot_payload, HookFormat::Copilot, &env);
        assert!(
            res.allowed,
            "expected `{allowed_cmd}` allowed: {}",
            res.reason
        );
        assert_eq!(res.stdout_json, "{}");
    }
}
