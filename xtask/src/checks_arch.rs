//! Architectural boundary and `schema/compatibility.json` contract checks.

use std::fs;
use std::path::Path;

use engine_core::{CURRENT_COMPATIBILITY, CompatibilitySpec, TransitionMode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::{capture_cmd, is_cargo_offline, walk_repo_files};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionSpecificDefaults {
    pub v1_to_v2_audit_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityDescriptor {
    pub schema: String,
    pub service_name: String,
    pub config_schema_version: u32,
    pub state_format_version: u32,
    pub min_readable_state_format: u32,
    pub oldest_writable_state_format: u32,
    pub transition_mode: String,
    pub preserved_state_fields: Vec<String>,
    pub version_specific_defaults: VersionSpecificDefaults,
}

const FORBIDDEN_ENV_KEYS: &[&str] = &[
    "dev",
    "staging",
    "prod",
    "production",
    "environment",
    "cluster",
    "namespace",
    "replicas",
];

pub fn check_architecture(root: &Path) -> Result<String, String> {
    let files = walk_repo_files(root)?;
    for path in &files {
        if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && matches!(ext, "sh" | "py")
        {
            let rel = path.strip_prefix(root).unwrap_or(path);
            return Err(format!(
                "forbidden script file `{}`: zero .sh or .py files permitted",
                rel.display()
            ));
        }
    }

    let meta_args: &[&str] = if is_cargo_offline() {
        &[
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
        ]
    } else {
        &["metadata", "--no-deps", "--format-version", "1", "--locked"]
    };
    let metadata_raw = capture_cmd(root, "cargo", meta_args)?;
    let metadata: Value = serde_json::from_str(&metadata_raw)
        .map_err(|e| format!("invalid cargo metadata JSON: {e}"))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata missing packages array".to_owned())?;

    let find_pkg = |name: &str| -> Result<&Value, String> {
        packages
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
            .ok_or_else(|| format!("workspace missing required crate `{name}`"))
    };

    let core_pkg = find_pkg("engine-core")?;
    let core_deps = core_pkg
        .get("dependencies")
        .and_then(Value::as_array)
        .ok_or_else(|| "engine-core missing dependencies list".to_owned())?;
    if !core_deps.is_empty() {
        return Err(format!(
            "engine-core must have 0 external dependencies, found {}",
            core_deps.len()
        ));
    }

    let service_pkg = find_pkg("engine-service")?;
    let service_deps: Vec<&str> = service_pkg
        .get("dependencies")
        .and_then(Value::as_array)
        .ok_or_else(|| "engine-service missing dependencies list".to_owned())?
        .iter()
        .filter_map(|d| d.get("name").and_then(Value::as_str))
        .collect();
    if !service_deps.contains(&"engine-core") {
        return Err("engine-service must depend on engine-core".to_owned());
    }
    for banned in ["xtask", "kube", "k8s-openapi", "aws-sdk-s3"] {
        if service_deps.contains(&banned) {
            return Err(format!("engine-service must not depend on `{banned}`"));
        }
    }

    let core_lib = fs::read_to_string(root.join("crates/engine-core/src/lib.rs"))
        .map_err(|e| format!("failed reading engine-core/src/lib.rs: {e}"))?;
    if !core_lib.contains("#![no_std]") || !core_lib.contains("#![forbid(unsafe_code)]") {
        return Err(
            "engine-core/src/lib.rs must declare both #![no_std] and #![forbid(unsafe_code)]"
                .to_owned(),
        );
    }

    for path in &files {
        let rel = path.strip_prefix(root).unwrap_or(path);
        if rel.starts_with("crates/engine-core/src")
            && path.extension().and_then(|e| e.to_str()) == Some("rs")
        {
            let src = fs::read_to_string(path)
                .map_err(|e| format!("failed reading {}: {e}", rel.display()))?;
            for (idx, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("std::time") || code.contains("std::fs") {
                    return Err(format!(
                        "{}:{} references forbidden host I/O/clock API in Sans-I/O engine-core",
                        rel.display(),
                        idx + 1
                    ));
                }
            }
        }
    }

    Ok("engine-core (0 deps, no_std, sans-io) <- engine-service verified; 0 .sh/.py files".into())
}

pub fn check_schema_compat(root: &Path) -> Result<String, String> {
    let compat_path = root.join("schema/compatibility.json");
    let raw = fs::read_to_string(&compat_path)
        .map_err(|e| format!("failed reading {}: {e}", compat_path.display()))?;
    let desc = validate_compat_json_str(&raw, true)?;
    Ok(format!(
        "{} config=v{} state=v{} (min_read={}, oldest_write={}, mode={})",
        desc.service_name,
        desc.config_schema_version,
        desc.state_format_version,
        desc.min_readable_state_format,
        desc.oldest_writable_state_format,
        desc.transition_mode
    ))
}

pub fn validate_compat_json_str(
    raw: &str,
    enforce_code_parity: bool,
) -> Result<CompatibilityDescriptor, String> {
    let val: Value = serde_json::from_str(raw)
        .map_err(|e| format!("invalid schema/compatibility.json JSON: {e}"))?;
    let obj = val
        .as_object()
        .ok_or_else(|| "schema/compatibility.json must be a JSON object".to_owned())?;
    for key in FORBIDDEN_ENV_KEYS {
        if obj.contains_key(*key) {
            return Err(format!(
                "schema/compatibility.json must not couple to live environment key `{key}`"
            ));
        }
    }

    let desc: CompatibilityDescriptor = serde_json::from_value(val)
        .map_err(|e| format!("schema/compatibility.json contract mismatch: {e}"))?;

    let transition_mode = TransitionMode::parse(&desc.transition_mode)
        .map_err(|e| format!("invalid transition_mode `{}`: {e:?}", desc.transition_mode))?;
    let field_refs: Vec<&str> = desc
        .preserved_state_fields
        .iter()
        .map(String::as_str)
        .collect();

    let spec = CompatibilitySpec {
        schema: &desc.schema,
        service_name: &desc.service_name,
        config_schema_version: desc.config_schema_version,
        state_format_version: desc.state_format_version,
        min_readable_state_format: desc.min_readable_state_format,
        oldest_writable_state_format: desc.oldest_writable_state_format,
        transition_mode,
        preserved_state_fields: &field_refs,
        v1_to_v2_audit_epoch: desc.version_specific_defaults.v1_to_v2_audit_epoch,
    };
    spec.validate()
        .map_err(|e| format!("compatibility invariant violation: {e:?}"))?;

    if enforce_code_parity && spec != CURRENT_COMPATIBILITY {
        return Err(format!(
            "schema/compatibility.json drifted from engine_core::CURRENT_COMPATIBILITY ({CURRENT_COMPATIBILITY:?})"
        ));
    }
    Ok(desc)
}

#[cfg(test)]
mod tests {
    use super::validate_compat_json_str;

    #[test]
    fn validates_four_transition_modes_and_compatibility_bounds() {
        let valid_expand = r#"{
          "schema": "service.compatibility/v1",
          "service_name": "admission-engine",
          "config_schema_version": 1,
          "state_format_version": 2,
          "min_readable_state_format": 1,
          "oldest_writable_state_format": 1,
          "transition_mode": "expand",
          "preserved_state_fields": ["window_epoch_ms", "consumed_units", "last_sequence"],
          "version_specific_defaults": { "v1_to_v2_audit_epoch": 1 }
        }"#;
        assert!(validate_compat_json_str(valid_expand, true).is_ok());

        let invalid_expand = valid_expand.replace(
            "\"oldest_writable_state_format\": 1",
            "\"oldest_writable_state_format\": 2",
        );
        assert!(validate_compat_json_str(&invalid_expand, false).is_err());

        let valid_contract = invalid_expand.replace("\"expand\"", "\"contract\"");
        assert!(validate_compat_json_str(&valid_contract, false).is_ok());

        let invalid_forward_only = valid_contract.replace("\"contract\"", "\"forward_only\"");
        assert!(validate_compat_json_str(&invalid_forward_only, false).is_err());

        let valid_reversible = invalid_expand.replace("\"expand\"", "\"reversible\"");
        assert!(validate_compat_json_str(&valid_reversible, false).is_ok());
    }
}
