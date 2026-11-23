---
name: schema-and-slsa-release
description: Manage Expand/Contract state compatibility descriptors, release tag preflight checks, and SLSA-attested RELEASE-EVIDENCE.json packaging.
---

# Schema & SLSA Release Skill

Use this skill when preparing release candidates (`alpha`/`beta`/`rc`), stable releases, or `schema/compatibility.json` transitions for downstream `platform-template` consumption.

## Workflow

1. **Audit Expand/Contract Transition Bounds**:
   - When introducing `state_format_version = N`, ship an `expand` release first (`min_readable_state_format = N - 1`, `max_writable_prior_state_format = N - 1`, `migration_class = "reversible"`, `expand_contract_phase = "expand"`).
   - Advance `max_writable_prior_state_format = N` (`contract` / `steady`) only in a subsequent release after fleet soak completes downstream.
2. **Synchronize Version & Release Notes**:
   - Update `[workspace.package].version` in `Cargo.toml`, `engine_version` in `schema/compatibility.json`, and author `docs/releases/<tag>.md`.
3. **Run Verification, Tag Preflight & Packaging**:
   - Run `cargo xtask verify --force` to refresh `evidence/verification.json` for `HEAD^{tree}`.
   - Run `cargo xtask preflight-tag <tag>` (`vX.Y.Z` or `vX.Y.Z-(alpha|beta|rc).N`).
   - Run `cargo xtask package --out dist` and confirm `dist/RELEASE-EVIDENCE.json`, `dist/compatibility.json`, `dist/SHA256SUMS`, and `dist/*.tar.gz` are emitted cleanly.
