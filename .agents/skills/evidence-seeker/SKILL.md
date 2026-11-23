---
name: evidence-seeker
description: Ground architectural, compatibility, and bug-fix claims in discriminating tests, binary self-checks, and linked epistemic notes.
---

# Evidence Seeker Skill

Use this skill whenever diagnosing state-machine behavior, proposing a `schema/compatibility.json` change, or recording engineering findings in `knowledge/`.

## Workflow

1. **Separate Observation from Assumption**:
   - Reproduce behavior deterministically in `crates/engine-core/tests/` (injecting explicit `MonoTime` and `CodecPolicy`) or `crates/engine-service/tests/integration.rs` before proposing production edits.
2. **Enforce the Two-Commit Verifier Boundary**:
   - Commit the discriminating test or `xtask` check first with trailer `Trust-Boundary: verifier`.
   - Implement the minimal `crates/*/src/**` change in a separate follow-up commit.
3. **Verify Compatibility & Artifact Parity**:
   - Run `cargo run --locked -p engine-service -- --print-compatibility` and `cargo run --locked -p engine-service -- --self-check`.
   - Run `cargo xtask verify` and inspect `evidence/verification.json`.
4. **Record Durable Knowledge**:
   - Capture empirical measurements in `knowledge/observations/O-NNNN-*.md`, state-migration analyses in `knowledge/research/R-NNNN-*.md`, and architectural contracts in `knowledge/decisions/D-NNNN-*.md` with `added_by: agent` and `human_ratified: false`, then index them in `knowledge/README.md`.
