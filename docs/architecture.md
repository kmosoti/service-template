# `service-template` Architecture

## Position & Domain Thread in the 4-Template Suite

```text
research-template  -->  library-template  -->  service-template  -->  platform-template
(burst/window data)     (quota-core crate)     (admission-engine      (dev/staging/prod
                                                + compat schema)       domain GitOps)
```

- **Unit of truth**: an immutable, path-stripped, SLSA-attested release bundle (`engine_digest` in `dist/release-manifest.json`) paired with `schema/compatibility.json`.
- **Decoupled config boundary**: `admission-engine --config` accepts standalone `EngineConfig` (`fixtures/engine-config.json`) AND downstream `platform.domain-config/v1` envelopes (`engine_config` projection) without embedding live `dev`/`staging`/`prod` topology in this repository.

## Workspace Crate Boundaries

### 1. `crates/engine-core` (`#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, 0 external deps)
- Verified on bare-metal `thumbv7em-none-eabi` with zero `std::time` or `std::fs` imports.
- Overflow-safe (`checked_add`) admission state machine tracking `window_epoch_ms`, `consumed_units`, `last_sequence`, and `audit_epoch`.
- Checksummed binary snapshot codec (`ADMSNAP\0`) implementing mixed-writer `expand` projection (`v1 <-> v2`, preserving `window_epoch_ms`, `consumed_units`, `last_sequence` losslessly and defaulting `v1 -> v2` `audit_epoch` to `1`) and fail-closed `contract` bounds.

### 2. `crates/engine-service` (`admission-engine`)
- `--print-compatibility`: prints canonical JSON matching `schema/compatibility.json`.
- `--self-check`: executes deterministic admission + checkpoint round-trip verification.
- `--run-stdio-batch [--config <path>] [--state-in <path>] [--state-out <path>]`: processes NDJSON commands to EOF, flushes `stdout`, encodes the **final post-batch `EngineState`** at `state_write_version`, writes `<state-out>.tmp.<pid>`, calls `sync_all()`, and atomically renames onto `<state-out>`. Interrupted or malformed batches leave prior checkpoints untouched.

### 3. Pure-Rust `xtask/` Governance & Reproducible Packaging
- `cargo xtask verify [--force]`: snapshots `git status --porcelain=v1 --untracked-files=all` before and after all 12 gates, stamping `worktree_dirty` and `tree_hash`, and caching clean-tree receipts keyed on `(HEAD, HEAD^{tree}, Cargo.lock, rust-toolchain.toml, schema/compatibility.json, rustc --version, gates)`.
- `cargo xtask package [--out dist]`: refuses dirty worktrees, exports `HEAD` via `git archive`, compiles with `strip = "symbols"` and `--remap-path-prefix`, scans the binary bytes for leaked host paths, builds a normalized `.tar.gz` via `tar` + `flate2`, and emits `dist/release-manifest.json`.
- `cargo xtask release-lineage <tag>` & `cargo xtask sync-branches [--check | --apply]`: enforces exact `origin/release/X.Y` ancestry for patch tags (`Z > 0`) and synchronizes `develop-agent` with `main`.
