## Summary

- **Objective:**
- **Target Branch (`develop-agent` / `main` / `release/*`):**

## Compatibility & State Inertia Checklist

- [ ] `schema/compatibility.json`, `Cargo.toml`, and `admission-engine --print-compatibility` are in exact parity.
- [ ] State migration impact classified (`steady` / `expand` / `contract` and `reversible` / `forward_only`).
- [ ] Zero live environment (`dev` / `staging` / `prod`) identifiers introduced into this repository.

## Governance & Verification Checklist

- [ ] Verifier/test commits carry `Trust-Boundary: verifier` and precede production `crates/*/src/**` commits.
- [ ] `cargo xtask verify` passes all 12 gates cleanly.
- [ ] `cargo xtask package --out dist` succeeds and emits `RELEASE-EVIDENCE.json`.
- [ ] Epistemic notes in `knowledge/` updated if architectural or compatibility invariants changed (`human_ratified: false` for agent-authored entries).
