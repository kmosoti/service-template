//! Idempotent workspace verification runner with pre/post `InputFingerprint`
//! quiescent checks and atomic `evidence/verification.json` persistence (`H4` + `M4`).

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent_sync::check_agent_configs;
use crate::checks_arch::{check_architecture, check_schema_compat};
use crate::checks_repo::{check_line_caps, check_privacy, check_workflow_pins};
pub use crate::fingerprint::{compute_fingerprint, git_porcelain_all};
use crate::knowledge::check_knowledge;
use crate::trust_boundary::check_trust_boundary;
use crate::util::{run_cargo, run_cmd};

pub const VERIFICATION_SCHEMA: &str = "service.verification/v1";

pub const EXPECTED_GATES: &[&str] = &[
    "cargo-fmt",
    "cargo-clippy",
    "cargo-test",
    "engine-core-no-std",
    "check-architecture",
    "check-schema-compat",
    "check-trust-boundary",
    "check-knowledge",
    "check-workflow-pins",
    "check-agent-configs",
    "check-line-caps",
    "check-privacy",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    pub gate: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReceipt {
    pub schema: String,
    pub status: String,
    pub head_commit: String,
    pub head_tree: String,
    pub refs_sha256: String,
    pub tree_hash: String,
    pub worktree_dirty: bool,
    pub cargo_lock_sha256: String,
    pub rust_toolchain_sha256: String,
    pub compatibility_sha256: String,
    pub rustc_version: String,
    pub gates: Vec<GateResult>,
}

pub fn verify_head_receipt(root: &Path) -> Result<VerificationReceipt, String> {
    let receipt_path = root.join("evidence/verification.json");
    let raw = fs::read_to_string(&receipt_path).map_err(|_| {
        "missing evidence/verification.json (run `cargo xtask verify` first)".to_owned()
    })?;
    let receipt: VerificationReceipt = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid evidence/verification.json: {e}"))?;
    if receipt.schema != VERIFICATION_SCHEMA || receipt.status != "pass" || receipt.worktree_dirty {
        return Err("evidence/verification.json is not a clean passing receipt".to_owned());
    }

    let fp = compute_fingerprint(root)?;
    if fp.worktree_dirty {
        return Err("working tree has uncommitted or untracked modifications".to_owned());
    }
    if receipt.head_commit != fp.head_commit
        || receipt.head_tree != fp.head_tree
        || receipt.refs_sha256 != fp.refs_sha256
        || receipt.tree_hash != fp.head_tree
    {
        return Err(
            "verification receipt does not match current HEAD commit, tree, or refs".into(),
        );
    }
    if receipt.cargo_lock_sha256 != fp.cargo_lock_sha256
        || receipt.rust_toolchain_sha256 != fp.rust_toolchain_sha256
        || receipt.compatibility_sha256 != fp.compatibility_sha256
    {
        return Err("lockfile, toolchain, or compatibility descriptor drifted from receipt".into());
    }
    let gates_ok = receipt.gates.len() == EXPECTED_GATES.len()
        && receipt
            .gates
            .iter()
            .zip(EXPECTED_GATES.iter())
            .all(|(g, exp)| g.gate == *exp && g.status == "pass");
    if !gates_ok {
        return Err("receipt gate list is incomplete or non-passing".to_owned());
    }
    Ok(receipt)
}

pub fn run_verify(root: &Path, force: bool) -> Result<(), String> {
    let pre_fp = compute_fingerprint(root)?;
    if !force
        && !pre_fp.worktree_dirty
        && let Ok(cached) = verify_head_receipt(root)
        && cached.rustc_version == pre_fp.rustc_version
    {
        println!(
            "[idempotent cache hit] evidence/verification.json matches clean HEAD^{{tree}} ({})",
            cached.head_tree
        );
        return Ok(());
    }

    let mut gates = Vec::with_capacity(EXPECTED_GATES.len());

    println!("==> [1/12] cargo fmt --all -- --check");
    run_cmd(root, "cargo", &["fmt", "--all", "--", "--check"])?;
    gates.push(pass("cargo-fmt", "rustfmt clean"));

    println!("==> [2/12] cargo clippy --workspace --all-targets --locked -- -D warnings");
    run_cargo(
        root,
        "clippy",
        &[
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    gates.push(pass("cargo-clippy", "0 warnings across all targets"));

    println!("==> [3/12] cargo test --workspace --locked");
    run_cargo(root, "test", &["--workspace", "--locked"])?;
    gates.push(pass("cargo-test", "all unit and integration tests passed"));

    println!("==> [4/12] cargo check --locked -p engine-core --target thumbv7em-none-eabi");
    run_cargo(
        root,
        "check",
        &[
            "--locked",
            "-p",
            "engine-core",
            "--target",
            "thumbv7em-none-eabi",
        ],
    )?;
    gates.push(pass("engine-core-no-std", "thumbv7em-none-eabi clean"));

    type CheckFn = fn(&Path) -> Result<String, String>;
    let checks: [(&str, CheckFn); 8] = [
        ("check-architecture", check_architecture),
        ("check-schema-compat", check_schema_compat),
        ("check-trust-boundary", |r| check_trust_boundary(r, None)),
        ("check-knowledge", check_knowledge),
        ("check-workflow-pins", check_workflow_pins),
        ("check-agent-configs", check_agent_configs),
        ("check-line-caps", check_line_caps),
        ("check-privacy", check_privacy),
    ];

    for (idx, (name, func)) in checks.iter().enumerate() {
        println!("==> [{}/12] {name}", idx + 5);
        let detail = func(root)?;
        gates.push(pass(name, &detail));
    }

    let post_fp = compute_fingerprint(root)?;
    if pre_fp != post_fp {
        return Err(
            "workspace inputs or refs mutated mid-verification; re-run `cargo xtask verify` on a quiescent tree"
                .to_owned(),
        );
    }

    let receipt = VerificationReceipt {
        schema: VERIFICATION_SCHEMA.to_owned(),
        status: "pass".to_owned(),
        head_commit: post_fp.head_commit,
        head_tree: post_fp.head_tree,
        refs_sha256: post_fp.refs_sha256,
        tree_hash: post_fp.tree_hash.clone(),
        worktree_dirty: post_fp.worktree_dirty,
        cargo_lock_sha256: post_fp.cargo_lock_sha256,
        rust_toolchain_sha256: post_fp.rust_toolchain_sha256,
        compatibility_sha256: post_fp.compatibility_sha256,
        rustc_version: post_fp.rustc_version,
        gates,
    };

    write_receipt_atomic(root, &receipt)?;
    println!(
        "ok: all 12 verification gates passed; receipt written (tree_hash={})",
        post_fp.tree_hash
    );
    Ok(())
}

fn write_receipt_atomic(root: &Path, receipt: &VerificationReceipt) -> Result<(), String> {
    let evidence_dir = root.join("evidence");
    fs::create_dir_all(&evidence_dir).map_err(|e| format!("create evidence/: {e}"))?;
    let mut rendered = serde_json::to_string_pretty(receipt).map_err(|e| e.to_string())?;
    rendered.push('\n');

    let final_path = evidence_dir.join("verification.json");
    let tmp_path = evidence_dir.join(format!("verification.json.tmp.{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(rendered.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, &final_path)
    })();
    if let Err(err) = res {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "atomic write of evidence/verification.json failed: {err}"
        ));
    }
    Ok(())
}

fn pass(gate: &str, detail: &str) -> GateResult {
    GateResult {
        gate: gate.to_owned(),
        status: "pass".to_owned(),
        detail: detail.to_owned(),
    }
}
