#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn invoke_hook_in_dir(cwd: &Path, format: &str, stdin_json: &str) -> (Option<i32>, Value) {
    let bin = env!("CARGO_BIN_EXE_xtask");
    let mut child = Command::new(bin)
        .current_dir(cwd)
        .args(["hook", "pre-tool-use", "--format", format])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn xtask hook");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin_json.as_bytes())
        .expect("write payload");
    let out = child.wait_with_output().expect("wait xtask hook");
    let parsed: Value = serde_json::from_slice(&out.stdout).expect("valid hook stdout JSON");
    (out.status.code(), parsed)
}

#[test]
fn all_four_formats_work_from_tmp_and_docs_with_exact_wire_contracts() {
    let tmp = std::env::temp_dir();
    let docs_dir = workspace_root().join("docs");

    for cwd in [tmp.as_path(), docs_dir.as_path()] {
        for payload in [
            "",
            "{malformed",
            r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"gh pr merge 1 --squash"}}}"#,
            r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"git push origin v9.9.9"}}}"#,
            r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"(git push origin main)"}}}"#,
            r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"bash -c 'git push origin release/0.1'"}}}"#,
        ] {
            let (ag_code, ag_json) = invoke_hook_in_dir(cwd, "antigravity", payload);
            assert_eq!(ag_code, Some(0));
            assert_eq!(ag_json["decision"], "deny");

            let (cl_code, cl_json) = invoke_hook_in_dir(cwd, "claude", payload);
            assert_eq!(cl_code, Some(0));
            assert_eq!(cl_json["hookSpecificOutput"]["permissionDecision"], "deny");

            let (cx_code, cx_json) = invoke_hook_in_dir(cwd, "codex", payload);
            assert_eq!(cx_code, Some(0));
            assert!(cx_json.get("decision").is_none());
            assert_eq!(cx_json["hookSpecificOutput"]["permissionDecision"], "deny");

            let (cp_code, cp_json) = invoke_hook_in_dir(cwd, "copilot", payload);
            assert_eq!(cp_code, Some(0));
            assert_eq!(cp_json["permissionDecision"], "deny");
        }

        let pwd_payload = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"pwd"}}}"#;
        let (ag_code, ag_val) = invoke_hook_in_dir(cwd, "antigravity", pwd_payload);
        assert_eq!(ag_code, Some(0));
        assert_eq!(ag_val, json!({ "decision": "allow" }));

        for fmt in ["claude", "codex", "copilot"] {
            let (code, val) = invoke_hook_in_dir(cwd, fmt, pwd_payload);
            assert_eq!(code, Some(0));
            assert_eq!(val, json!({}));
        }
    }
}
