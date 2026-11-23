---
id: O-0001
kind: observation
status: recorded
date: 2026-01-15
added_by: agent
human_ratified: false
links:
  - knowledge/decisions/D-0001-sans-io-and-schema-descriptor.md
  - knowledge/research/R-0001-expand-contract-state-inertia.md
  - xtask/src/package.rs
---

# O-0001: Path-Stripped Deterministic Engine Packaging and SLSA Handoff

## Observation

Unstripped Rust release binaries embed workstation source paths, `$CARGO_HOME`, and `$RUSTUP_HOME` in panic/symbol metadata, both leaking private filesystem structure and breaking cross-machine binary digest reproducibility.

## Measured Mitigation in `cargo xtask package`

1. **Clean-Tree Staging + Prefix Remapping**: `package.rs` refuses dirty worktrees, unpacks `git archive HEAD` into an isolated staging directory, compiles with `[profile.release] strip = "symbols"`, `codegen-units = 1`, `lto = "thin"`, `panic = "abort"`, and `CARGO_ENCODED_RUSTFLAGS` `--remap-path-prefix` across staging, `$CARGO_HOME`, and `$RUSTUP_HOME`, and scans the compiled binary bytes to fail closed on any residual host path.
2. **Pure-Rust Archive & `release-manifest.json`**: `tar` + `flate2` (`GzBuilder::mtime(0)`, `uid/gid = 0`, `mtime = 0`, sorted entries) emit a normalized `.tar.gz` paired with `dist/release-manifest.json` (`engine_digest`, `binary_digest`, JCS `compatibility_digest`, and embedded `compatibility`).
