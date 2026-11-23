//! 4-Host `PreToolUse` governance hooks (Antigravity, Claude Code, Codex, Copilot).
//!
//! Always exits `0`: emits `{}` on neutral pass-through (never auto-approving via
//! `permissionDecision: "allow"`) and fails closed with structured `"deny"` JSON on
//! policy violations or malformed/empty JSON input.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use crate::hooks_git::{check_git_push, extract_git_push_args};
use crate::hooks_shell::extract_shell_commands;
use crate::lineage::validate_tag_metadata;
use crate::util::capture_cmd;
use crate::verify::verify_head_receipt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFormat {
    Antigravity,
    Claude,
    Codex,
    Copilot,
}

impl HookFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "antigravity" => Ok(Self::Antigravity),
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "copilot" => Ok(Self::Copilot),
            other => Err(format!(
                "unsupported hook format `{other}` (expected `antigravity`, `claude`, `codex`, or `copilot`)"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    pub allowed: bool,
    pub reason: String,
    pub stdout_json: String,
}

pub trait HookEnv {
    fn current_branch(&self) -> Option<String>;
    fn has_valid_head_receipt(&self) -> bool;
    fn preflight_tag(&self, tag: &str) -> Result<(), String>;
    fn local_tag_matches_head(&self, tag: &str) -> Result<(), String>;
}

pub struct RepoHookEnv<'a> {
    pub root: &'a Path,
}

impl HookEnv for RepoHookEnv<'_> {
    fn current_branch(&self) -> Option<String> {
        capture_cmd(
            self.root,
            "git",
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .ok()
    }

    fn has_valid_head_receipt(&self) -> bool {
        verify_head_receipt(self.root).is_ok()
    }

    fn preflight_tag(&self, tag: &str) -> Result<(), String> {
        validate_tag_metadata(self.root, tag).map(|_| ())
    }

    fn local_tag_matches_head(&self, tag: &str) -> Result<(), String> {
        let head = capture_cmd(self.root, "git", &["rev-parse", "HEAD"])?;
        let peeled_ref = format!("refs/tags/{tag}^{{commit}}");
        let tag_commit = capture_cmd(self.root, "git", &["rev-parse", "--verify", &peeled_ref])
            .map_err(|_| format!("local tag `{tag}` does not exist"))?;
        if tag_commit != head {
            return Err(format!(
                "local tag `{tag}` ({tag_commit}) does not point to HEAD ({head})"
            ));
        }
        Ok(())
    }
}

pub fn run_pre_tool_use_stdin(root: &Path, format: HookFormat) -> Result<(), String> {
    let mut input = String::new();
    let outcome = match std::io::stdin().read_to_string(&mut input) {
        Ok(_) => {
            let env = RepoHookEnv { root };
            evaluate_pre_tool_use(&input, format, &env)
        }
        Err(err) => render_outcome(false, format!("failed reading hook stdin: {err}"), format),
    };
    println!("{}", outcome.stdout_json);
    Ok(())
}

pub fn evaluate_pre_tool_use(
    stdin_json: &str,
    format: HookFormat,
    env: &dyn HookEnv,
) -> HookOutcome {
    let trimmed = stdin_json.trim();
    if trimmed.is_empty() {
        return render_outcome(
            false,
            "empty PreToolUse stdin JSON payload (fail-closed)".to_owned(),
            format,
        );
    }

    let (allowed, reason) = match serde_json::from_str::<Value>(trimmed) {
        Ok(payload) => match extract_command(&payload) {
            Ok(Some(command)) => match check_command_policy(&command, env) {
                Ok(()) => (true, String::new()),
                Err(deny_reason) => (false, deny_reason),
            },
            Ok(None) => (true, String::new()),
            Err(parse_err) => (false, parse_err),
        },
        Err(err) => (
            false,
            format!("malformed PreToolUse JSON payload (fail-closed): {err}"),
        ),
    };

    render_outcome(allowed, reason, format)
}

fn render_outcome(allowed: bool, reason: String, format: HookFormat) -> HookOutcome {
    let response_val = if allowed {
        match format {
            HookFormat::Antigravity => json!({ "decision": "allow" }),
            HookFormat::Claude | HookFormat::Codex | HookFormat::Copilot => json!({}),
        }
    } else {
        match format {
            HookFormat::Antigravity => json!({
                "decision": "deny",
                "reason": reason,
            }),
            HookFormat::Claude | HookFormat::Codex => json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                }
            }),
            HookFormat::Copilot => json!({
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }),
        }
    };
    let stdout_json = serde_json::to_string(&response_val)
        .expect("serializing static hook JSON structure cannot fail");
    HookOutcome {
        allowed,
        reason,
        stdout_json,
    }
}

fn extract_command(payload: &Value) -> Result<Option<String>, String> {
    let candidates = [
        payload.pointer("/toolCall/args/CommandLine"),
        payload.pointer("/toolCall/args/command"),
        payload.pointer("/tool_input/command"),
        payload.pointer("/tool_input/CommandLine"),
        payload.pointer("/toolArgs/command"),
        payload.pointer("/toolArgs/CommandLine"),
        payload.pointer("/input/CommandLine"),
        payload.pointer("/input/command"),
        payload.pointer("/arguments/CommandLine"),
        payload.pointer("/arguments/command"),
        payload.get("CommandLine"),
        payload.get("command"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(s) = candidate.as_str() {
            return Ok(Some(s.to_owned()));
        }
    }
    if let Some(raw_args) = payload.get("toolArgs").and_then(Value::as_str) {
        let nested: Value = serde_json::from_str(raw_args)
            .map_err(|e| format!("malformed nested toolArgs JSON: {e}"))?;
        for key in ["command", "CommandLine"] {
            if let Some(s) = nested.get(key).and_then(Value::as_str) {
                return Ok(Some(s.to_owned()));
            }
        }
    }
    Ok(None)
}

fn check_command_policy(command: &str, env: &dyn HookEnv) -> Result<(), String> {
    for tokens in extract_shell_commands(command) {
        check_invocation(&tokens, env)?;
    }
    Ok(())
}

fn check_invocation(tokens: &[String], env: &dyn HookEnv) -> Result<(), String> {
    let joined_lower = tokens.join(" ").to_ascii_lowercase();

    if joined_lower.contains("pending_deployments")
        || joined_lower.contains("deployment_protection_rule")
        || ((joined_lower.contains("main-release")
            || joined_lower.contains("stable-publish")
            || joined_lower.contains("tag-release"))
            && (joined_lower.contains("approve") || joined_lower.contains("review")))
    {
        return Err(
            "agents may never approve `main-release`, `tag-release`, or `stable-publish` environment gates"
                .to_owned(),
        );
    }

    if tokens.first().map(String::as_str) == Some("gh")
        && tokens.get(1).map(String::as_str) == Some("api")
        && joined_lower.contains("/pulls/")
        && joined_lower.contains("/merge")
    {
        return Err("raw `gh api` pull-request merge bypass is prohibited".to_owned());
    }

    if tokens.len() >= 3 && tokens[0] == "gh" && tokens[1] == "pr" && tokens[2] == "merge" {
        check_gh_pr_merge(&tokens[3..])?;
    }

    if tokens.first().map(String::as_str) == Some("git")
        && let Some(push_args) = extract_git_push_args(&tokens[1..])
    {
        check_git_push(push_args, env)?;
    }

    Ok(())
}

fn check_gh_pr_merge(args: &[String]) -> Result<(), String> {
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "--admin" | "--merge" | "--squash" | "-m" | "-s"))
    {
        return Err(
            "`gh pr merge` forbids `--admin`, `--merge`, and `--squash`; rebase-only required"
                .to_owned(),
        );
    }
    if !args.iter().any(|a| matches!(a.as_str(), "--rebase" | "-r")) {
        return Err("`gh pr merge` must explicitly pass `--rebase`".to_owned());
    }
    if args.iter().any(|a| {
        a == "main"
            || a == "origin/main"
            || a.starts_with("release/")
            || a.starts_with("origin/release/")
    }) {
        return Err(
            "agents may not merge PRs targeting `main` or `release/*`; human gate required"
                .to_owned(),
        );
    }
    Ok(())
}
