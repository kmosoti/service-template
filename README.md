# `service-template`

**Archetype 3 of 4** in the decoupled engineering lifecycle suite (`research-template` -> `library-template` -> **`service-template`** -> `platform-template`).

`service-template` governs **deployable transformation engines, protocol daemons, and stateful services** whose unit of truth is an **immutable, SLSA-attested binary artifact (`engine_digest`) paired with an explicit state and configuration compatibility descriptor (`schema/compatibility.json`)** — completely decoupled from any live `dev`, `staging`, or `prod` deployment topology.

---

## Lifecycle & Domain Thread

Across the 4-template suite, **`research-template`**, **`library-template` (`quota-core`)**, and **`service-template` (`admission-engine`)** share the admission-control / quota problem domain **conceptually** as standalone, self-contained workspace templates (with zero cross-repository Cargo source dependencies). By contrast, **`service-template` -> `platform-template` is a concrete byte-and-contract handoff**:

1. **`research-template` -> `library-template` -> `service-template` (Conceptual Domain Thread)**:
   - `research-template` explores empirical burst-traffic workloads; `library-template` (`quota-core`) models `#![no_std]` quota API ratcheting; and **`service-template` (`admission-engine`)** implements a standalone `#![no_std]` window-epoch admission engine (`window_epoch_ms`, `consumed_units`, `last_sequence`) with an Expand/Contract versioned snapshot codec (`v1 <-> v2`).
2. **`service-template` -> `platform-template` (Concrete Byte & Contract Handoff)**:
   - `cargo xtask package` emits `dist/admission-engine-vX.Y.Z-<target>.tar.gz`, `dist/compatibility.json` (`service.compatibility/v1`), and `dist/release-manifest.json` (`service.release-manifest/v1` with RFC 8785 JCS `compatibility_digest` and `engine_digest`), consumed directly by `platform-template`'s `import-service-release --archive` workflow.
   - Conversely, `admission-engine --run-stdio-batch --config <path>` accepts `platform-template`'s `platform.domain-config/v1` envelopes directly alongside standalone `EngineConfig` (`fixtures/engine-config.json`).

---

## Quick Start

```bash
# Run all 12 verification gates and atomically write `evidence/verification.json`
cargo xtask verify --force

# Re-run idempotently (instant cache hit when HEAD, tree, refs, lockfile, toolchain & schema match)
cargo xtask verify

# Build path-stripped release binary, deterministic `.tar.gz`, `SHA256SUMS` & `dist/release-manifest.json`
cargo xtask package

# Preview 3 GitHub Environments + 5 Repository Rulesets (pass `--main-pr-approvals 0` for solo repos)
cargo xtask github-setup example-org/admission-engine --dry-run
cargo xtask github-setup example-org/admission-engine --main-pr-approvals 0 --dry-run

# Cut a release tag locally as repository admin (zero GitHub Secrets required)
git tag -s v0.1.0 HEAD && git push origin v0.1.0
```

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. Copyright (c) 2026 Kennedy Mosoti.
