# GitHub Copilot Instructions (`service-template`)

Follow the repository rules in `AGENTS.md`:

1. **Pure Rust Only**: Never add `.sh` or `.py` scripts; extend `xtask/` (`#![forbid(unsafe_code)]`) instead.
2. **Crate Boundaries**:
   - `crates/engine-core`: `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, zero external dependencies, zero `std::time` or `std::fs`, verified on `thumbv7em-none-eabi`.
   - `crates/engine-service`: thin CLI/stdio shell (`admission-engine`) around `engine-core`.
3. **Schema & Rollback Discipline**:
   - Any change to configuration or snapshot state formats must update `schema/compatibility.json` and `engine_core::compat` together, obeying Expand/Contract state inertia rules (`reversible` vs `forward_only`, `steady` / `expand` / `contract`).
   - Never add `dev`, `staging`, or `prod` environment configuration to this repository.
4. **Modularity & Trust Boundary**:
   - Keep every `.rs` file `<= 300` lines (hard cap `350` lines).
   - Separate verifier/test commits (`Trust-Boundary: verifier`) from `crates/*/src/**` implementation commits.
   - Pin all GitHub Actions in `.github/workflows/*.yml` to exact 40-character lowercase commit SHAs.
