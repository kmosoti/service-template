# Claude Code Instructions (`service-template`)

Read and obey [AGENTS.md](AGENTS.md) and [SECURITY.md](SECURITY.md) before modifying any file in this workspace.

## Essential Commands

- `cargo xtask verify`: Runs all 12 verification gates (or reuses `evidence/verification.json` when the working tree is clean and `HEAD`, `HEAD^{tree}`, `Cargo.lock`, `rust-toolchain.toml`, `schema/compatibility.json`, `rustc --version`, and all gate statuses match).
- `cargo xtask verify --force`: Forces a complete re-execution of all 12 gates.
- `cargo xtask package [--out dist]`: Requires a clean working tree, exports `HEAD` into an isolated staging directory with `--remap-path-prefix` and `strip = "symbols"`, scans the binary for leaked host paths, builds a deterministic pure-Rust `.tar.gz`, and writes `dist/release-manifest.json`, `dist/compatibility.json`, and `dist/SHA256SUMS`.
- `cargo xtask release-lineage <tag>` & `cargo xtask preflight-tag <tag>`: Validates `main` vs exact `origin/release/X.Y` lineage (`Z > 0`), SemVer syntax, `Cargo.toml` parity, `docs/releases/<tag>.md`, and clean `HEAD^{tree}` verification receipt.
- `cargo xtask sync-branches [--check | --apply]`: Synchronizes `develop-agent` with `main` across merge promotions and `git cherry` patch-equivalent states.

## Active Guardrails & Subagents

- `.claude/settings.json` invokes `cargo run --quiet --manifest-path "${CLAUDE_PROJECT_DIR:-.}/Cargo.toml" --package xtask --locked -- hook pre-tool-use --format claude`, emitting `{}` on neutral commands and structured `deny` decisions on policy violations.
- Use `adversarial-breaker` to probe `u64::MAX` overflow boundaries, atomic `--state-out` crash preservation, and Expand/Contract `v1 <-> v2` projection invariants.
- Use `stack-reviewer` to audit scoped `Trust-Boundary: verifier` compliance, `.rs` line caps, and `schema/compatibility.json` parity before opening PRs.
