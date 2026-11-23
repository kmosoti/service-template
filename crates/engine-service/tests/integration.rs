#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use engine_service::{
    CompatibilityManifest, ServiceConfig, WireResponse, compatibility_manifest, load_config_file,
    render_compatibility_json, self_check,
};
use serde_json::json;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn print_compatibility_matches_schema_descriptor_exactly() {
    let schema_raw = fs::read_to_string(workspace_root().join("schema/compatibility.json"))
        .expect("read compatibility.json");
    let schema_doc: CompatibilityManifest =
        serde_json::from_str(&schema_raw).expect("parse compatibility.json");
    assert_eq!(schema_doc, compatibility_manifest());
    assert_eq!(schema_doc.validate(), Ok(()));

    let rendered = render_compatibility_json().expect("render manifest");
    assert_eq!(rendered.trim(), schema_raw.trim());

    let bin = env!("CARGO_BIN_EXE_admission-engine");
    let output = Command::new(bin)
        .arg("--print-compatibility")
        .output()
        .expect("invoke --print-compatibility");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert_eq!(stdout.trim(), schema_raw.trim());
}

#[test]
fn binary_self_check_and_standalone_fixture_config_succeed() {
    self_check().expect("library self-check");
    let fixture_cfg = load_config_file(&workspace_root().join("fixtures/engine-config.json"))
        .expect("load fixtures/engine-config.json");
    assert_eq!(
        fixture_cfg,
        ServiceConfig {
            config_schema_version: 1,
            window_ms: 1_000,
            max_units_per_window: 100,
            max_burst_units: 25,
            state_write_version: 2,
        }
    );

    let bin = env!("CARGO_BIN_EXE_admission-engine");
    let output = Command::new(bin)
        .arg("--self-check")
        .output()
        .expect("invoke --self-check");
    assert!(output.status.success());
}

#[test]
fn two_process_state_out_persists_final_post_batch_state_never_stale_mid_stream_snapshot() {
    let temp_dir =
        std::env::temp_dir().join(format!("admission-h11-two-proc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let cfg_path = temp_dir.join("config.json");
    let snap_path = temp_dir.join("state.snap");

    let cfg = ServiceConfig {
        config_schema_version: 1,
        window_ms: 1_000,
        max_units_per_window: 100,
        max_burst_units: 100,
        state_write_version: 2,
    };
    fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap()).expect("write config");

    // Process 1: budget=100 -> admit 20 (seq=1) -> Snapshot (seq=2) -> admit 30 (seq=3, 50 left).
    let bin = env!("CARGO_BIN_EXE_admission-engine");
    let mut proc1 = Command::new(bin)
        .args([
            "--run-stdio-batch",
            "--config",
            cfg_path.to_str().unwrap(),
            "--state-out",
            snap_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn process 1");
    proc1
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"{\"op\":\"evaluate\",\"now_ms\":100,\"subject\":\"client-1\",\"cost\":20}\n\
              {\"op\":\"snapshot\",\"now_ms\":200}\n\
              {\"op\":\"evaluate\",\"now_ms\":300,\"subject\":\"client-1\",\"cost\":30}\n",
        )
        .expect("write batch 1");
    let out1 = proc1.wait_with_output().expect("wait process 1");
    assert!(out1.status.success());
    let events1: Vec<WireResponse> = String::from_utf8(out1.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events1.len(), 3);

    // Process 2: restart from `--state-out` and request `cost=65` in the same window.
    // MUST be denied with `remaining_units = 50` and `sequence = 4` (never reusing 3!).
    let mut proc2 = Command::new(bin)
        .args([
            "--run-stdio-batch",
            "--config",
            cfg_path.to_str().unwrap(),
            "--state-in",
            snap_path.to_str().unwrap(),
            "--state-out",
            snap_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn process 2");
    proc2
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"op\":\"evaluate\",\"now_ms\":400,\"subject\":\"client-1\",\"cost\":65}\n")
        .expect("write batch 2");
    let out2 = proc2.wait_with_output().expect("wait process 2");
    assert!(out2.status.success());
    let resp2: WireResponse =
        serde_json::from_str(String::from_utf8(out2.stdout).unwrap().trim()).unwrap();
    assert_eq!(
        resp2,
        WireResponse::Evaluated {
            subject: "client-1".to_owned(),
            sequence: 4,
            admitted: false,
            remaining_units: 50,
            window_epoch_ms: None,
            reject_reason: Some("rate_limited".to_owned()),
            retry_after_ms: Some(600),
        }
    );

    // Interrupted / malformed mid-batch stream MUST preserve prior checkpoint intact!
    let prior_checkpoint = fs::read(&snap_path).expect("read committed checkpoint");
    let mut proc_abort = Command::new(bin)
        .args([
            "--run-stdio-batch",
            "--config",
            cfg_path.to_str().unwrap(),
            "--state-in",
            snap_path.to_str().unwrap(),
            "--state-out",
            snap_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aborted process");
    proc_abort
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"{\"op\":\"evaluate\",\"now_ms\":450,\"subject\":\"client-1\",\"cost\":25}\n\
              {\"op\":\"corrupt_truncated_json\"\n",
        )
        .expect("write partial corrupt batch");
    let out_abort = proc_abort.wait_with_output().expect("wait aborted process");
    assert!(!out_abort.status.success());
    assert_eq!(
        fs::read(&snap_path).expect("prior checkpoint still readable"),
        prior_checkpoint
    );
    let leftover_tmps: Vec<_> = fs::read_dir(&temp_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
        .collect();
    assert!(leftover_tmps.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn accepts_platform_domain_config_v1_envelope_directly_on_cli() {
    let temp_dir =
        std::env::temp_dir().join(format!("admission-h9-envelope-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let domain_cfg_path = temp_dir.join("domain-config.json");

    let domain_envelope = json!({
        "schema": "platform.domain-config/v1",
        "service_name": "admission-engine",
        "replicas": 4,
        "engine_config": {
            "config_schema_version": 1,
            "window_ms": 1000,
            "max_units_per_window": 100,
            "max_burst_units": 25,
            "state_write_version": 2
        }
    });
    fs::write(
        &domain_cfg_path,
        serde_json::to_string_pretty(&domain_envelope).unwrap(),
    )
    .expect("write platform envelope");

    let bin = env!("CARGO_BIN_EXE_admission-engine");
    let mut child = Command::new(bin)
        .args([
            "--run-stdio-batch",
            "--config",
            domain_cfg_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn with platform config envelope");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"{\"op\":\"evaluate\",\"now_ms\":50,\"subject\":\"api-gw\",\"cost\":20}\n\
              {\"op\":\"evaluate\",\"now_ms\":60,\"subject\":\"api-gw\",\"cost\":30}\n",
        )
        .expect("write commands");
    let out = child.wait_with_output().expect("wait child");
    assert!(out.status.success());
    let responses: Vec<WireResponse> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert!(matches!(
        responses[0],
        WireResponse::Evaluated {
            admitted: true,
            remaining_units: 80,
            ..
        }
    ));
    assert_eq!(
        responses[1],
        WireResponse::Evaluated {
            subject: "api-gw".to_owned(),
            sequence: 2,
            admitted: false,
            remaining_units: 80,
            window_epoch_ms: None,
            reject_reason: Some("burst_exceeded".to_owned()),
            retry_after_ms: Some(940),
        }
    );

    let _ = fs::remove_dir_all(&temp_dir);
}
