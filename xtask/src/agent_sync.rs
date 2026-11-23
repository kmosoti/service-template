//! Cross-tool agentic configuration, skill, subagent, and 4-host hook parity checks.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

const REQUIRED_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "SECURITY.md",
    ".github/copilot-instructions.md",
    ".github/hooks/forge-guard.json",
    ".agents/plugin.json",
    ".agents/hooks.json",
    ".agents/skills/evidence-seeker/SKILL.md",
    ".agents/skills/idempotent-workflow/SKILL.md",
    ".agents/skills/schema-and-slsa-release/SKILL.md",
    ".agents/plugins/service-forge/plugin.json",
    ".agents/plugins/service-forge/agents/adversarial-breaker.md",
    ".agents/plugins/service-forge/agents/stack-reviewer.md",
    ".claude/settings.json",
    ".claude/agents/adversarial-breaker.md",
    ".claude/agents/stack-reviewer.md",
    ".claude/skills/evidence-seeker/SKILL.md",
    ".claude/skills/idempotent-workflow/SKILL.md",
    ".claude/skills/schema-and-slsa-release/SKILL.md",
    ".codex/hooks.json",
    ".codex/agents/adversarial-breaker.toml",
    ".codex/agents/stack-reviewer.toml",
];

const ALLOWED_PLUGIN_KEYS: &[&str] = &[
    "name",
    "displayName",
    "version",
    "description",
    "suggestedPrompts",
];

const ALLOWED_AG_EVENTS: &[&str] = &[
    "SessionStart",
    "PreInvocation",
    "PostInvocation",
    "Stop",
    "PreToolUse",
    "PostToolUse",
];

pub fn check_agent_configs(root: &Path) -> Result<String, String> {
    for rel in REQUIRED_FILES {
        let path = root.join(rel);
        let meta =
            fs::metadata(&path).map_err(|_| format!("missing required agentic file `{rel}`"))?;
        if !meta.is_file() || meta.len() == 0 {
            return Err(format!("agentic file `{rel}` must be non-empty"));
        }
    }

    if root
        .join(".agents/plugins/service-forge/hooks.json")
        .exists()
    {
        return Err(
            "duplicate `.agents/plugins/service-forge/hooks.json` must be removed; keep `.agents/hooks.json`"
                .to_owned(),
        );
    }

    let ws_plugin_raw =
        fs::read_to_string(root.join(".agents/plugin.json")).map_err(|e| e.to_string())?;
    validate_plugin_manifest(&ws_plugin_raw, "service-template-agents")?;

    let forge_plugin_raw =
        fs::read_to_string(root.join(".agents/plugins/service-forge/plugin.json"))
            .map_err(|e| e.to_string())?;
    validate_plugin_manifest(&forge_plugin_raw, "service-forge")?;

    let ag_hooks_raw =
        fs::read_to_string(root.join(".agents/hooks.json")).map_err(|e| e.to_string())?;
    validate_antigravity_hooks(&ag_hooks_raw, ".agents/hooks.json")?;

    let claude_raw =
        fs::read_to_string(root.join(".claude/settings.json")).map_err(|e| e.to_string())?;
    let _: Value = serde_json::from_str(&claude_raw)
        .map_err(|e| format!("invalid .claude/settings.json: {e}"))?;
    if !claude_raw.contains("--manifest-path")
        || !claude_raw.contains("CLAUDE_PROJECT_DIR")
        || !claude_raw.contains("hook pre-tool-use --format claude")
    {
        return Err(
            ".claude/settings.json must wire `--manifest-path \"${CLAUDE_PROJECT_DIR:-.}/Cargo.toml\" ... hook pre-tool-use --format claude`"
                .to_owned(),
        );
    }

    let codex_hooks_raw =
        fs::read_to_string(root.join(".codex/hooks.json")).map_err(|e| e.to_string())?;
    let _: Value = serde_json::from_str(&codex_hooks_raw)
        .map_err(|e| format!("invalid .codex/hooks.json: {e}"))?;
    if !codex_hooks_raw
        .contains("cargo run --quiet --package xtask -- hook pre-tool-use --format codex")
        || codex_hooks_raw.contains("--manifest-path")
    {
        return Err(
            ".codex/hooks.json must invoke `cargo run --quiet --package xtask -- hook pre-tool-use --format codex` without relative --manifest-path"
                .to_owned(),
        );
    }

    let copilot_hooks_raw = fs::read_to_string(root.join(".github/hooks/forge-guard.json"))
        .map_err(|e| e.to_string())?;
    let copilot_val: Value = serde_json::from_str(&copilot_hooks_raw)
        .map_err(|e| format!("invalid .github/hooks/forge-guard.json: {e}"))?;
    if copilot_val.get("version").and_then(Value::as_u64) != Some(1)
        || !copilot_hooks_raw.contains("hook pre-tool-use --format copilot")
    {
        return Err(
            ".github/hooks/forge-guard.json must declare `version: 1` and `--format copilot`"
                .to_owned(),
        );
    }

    let ag_skills = list_skill_dirs(&root.join(".agents/skills"))?;
    let claude_skills = list_skill_dirs(&root.join(".claude/skills"))?;
    if ag_skills != claude_skills {
        return Err(format!(
            "skill set drift between .agents/skills ({ag_skills:?}) and .claude/skills ({claude_skills:?})"
        ));
    }

    let ag_agents = list_stem_names(&root.join(".agents/plugins/service-forge/agents"), "md")?;
    let claude_agents = list_stem_names(&root.join(".claude/agents"), "md")?;
    let codex_agents = list_stem_names(&root.join(".codex/agents"), "toml")?;
    if ag_agents != claude_agents || ag_agents != codex_agents {
        return Err(format!(
            "agent roster drift across .agents ({ag_agents:?}), .claude ({claude_agents:?}), .codex ({codex_agents:?})"
        ));
    }

    for agent in &codex_agents {
        let toml_path = root.join(".codex/agents").join(format!("{agent}.toml"));
        let raw = fs::read_to_string(&toml_path).map_err(|e| e.to_string())?;
        if !raw.contains(&format!("name = \"{agent}\"")) || !raw.contains("description =") {
            return Err(format!(
                "{} missing required `name` or `description` keys",
                toml_path.display()
            ));
        }
    }

    Ok(format!(
        "{} files, 4 host hooks, {} skills, {} subagents synchronized",
        REQUIRED_FILES.len(),
        ag_skills.len(),
        ag_agents.len()
    ))
}

fn validate_plugin_manifest(raw: &str, expected_name: &str) -> Result<(), String> {
    let val: Value = serde_json::from_str(raw).map_err(|e| format!("invalid plugin.json: {e}"))?;
    let obj = val
        .as_object()
        .ok_or_else(|| "plugin.json must be a JSON object".to_owned())?;
    for key in obj.keys() {
        if !ALLOWED_PLUGIN_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "plugin.json contains unsupported strict-mode key `{key}`"
            ));
        }
    }
    if obj.get("name").and_then(Value::as_str) != Some(expected_name) {
        return Err(format!("plugin.json `name` must equal `{expected_name}`"));
    }
    for req in ["displayName", "version", "description"] {
        if obj
            .get(req)
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(format!("plugin.json `{req}` must be a non-empty string"));
        }
    }
    let prompts = obj
        .get("suggestedPrompts")
        .and_then(Value::as_array)
        .ok_or_else(|| "plugin.json `suggestedPrompts` must be an array".to_owned())?;
    if prompts.is_empty() || !prompts.iter().all(|p| p.as_str().is_some()) {
        return Err("plugin.json `suggestedPrompts` must be a non-empty string array".to_owned());
    }
    Ok(())
}

fn validate_antigravity_hooks(raw: &str, label: &str) -> Result<(), String> {
    let val: Value = serde_json::from_str(raw).map_err(|e| format!("invalid {label} JSON: {e}"))?;
    let groups = val
        .as_object()
        .ok_or_else(|| format!("{label} must be a grouped JSON object"))?;
    if groups.is_empty() || groups.contains_key("hooks") {
        return Err(format!(
            "{label} must use named hook groups (not top-level `hooks`)"
        ));
    }

    let mut found_guard = false;
    for (group_name, events_val) in groups {
        let events = events_val
            .as_object()
            .ok_or_else(|| format!("{label}: group `{group_name}` must be an object"))?;
        for event_name in events.keys() {
            if !ALLOWED_AG_EVENTS.contains(&event_name.as_str()) {
                return Err(format!(
                    "{label}: group `{group_name}` uses unknown event `{event_name}`"
                ));
            }
        }
        if let Some(rules) = events.get("PreToolUse").and_then(Value::as_array) {
            for rule in rules {
                let matcher_ok = rule.get("matcher").and_then(Value::as_str) == Some("run_command");
                let cmd_ok = rule
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|arr| {
                        arr.iter().any(|h| {
                            h.get("type").and_then(Value::as_str) == Some("command")
                                && h.get("command").and_then(Value::as_str).is_some_and(|c| {
                                    c.contains("--manifest-path ../Cargo.toml")
                                        && c.contains("hook pre-tool-use --format antigravity")
                                })
                        })
                    });
                if matcher_ok && cmd_ok {
                    found_guard = true;
                }
            }
        }
    }

    if !found_guard {
        return Err(format!(
            "{label} must define `PreToolUse` `run_command` invoking `--manifest-path ../Cargo.toml ... hook pre-tool-use --format antigravity`"
        ));
    }
    Ok(())
}

fn list_skill_dirs(base: &Path) -> Result<BTreeSet<String>, String> {
    let mut set = BTreeSet::new();
    for entry in fs::read_dir(base).map_err(|e| format!("read {}: {e}", base.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let skill_md = path.join("SKILL.md");
            let content = fs::read_to_string(&skill_md)
                .map_err(|e| format!("missing {}: {e}", skill_md.display()))?;
            if !content.contains(&format!("name: {name}")) {
                return Err(format!(
                    "{} frontmatter must declare `name: {name}`",
                    skill_md.display()
                ));
            }
            set.insert(name);
        }
    }
    Ok(set)
}

fn list_stem_names(dir: &Path, ext: &str) -> Result<BTreeSet<String>, String> {
    let mut set = BTreeSet::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some(ext)
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            set.insert(stem.to_owned());
        }
    }
    Ok(set)
}
