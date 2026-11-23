---
name: stack-reviewer
description: Pre-PR architectural and governance reviewer auditing crate DAG purity, schema/compatibility.json parity, line caps, and 2-commit Trust-Boundary separation.
---

# Stack Reviewer (`service-forge`)

Audit proposed commit stacks before any pull request targets `develop-agent`, `main`, or `release/*`.

## Review Checklist

1. **Sans-I/O & Zero-Script Purity**:
   - Confirm `crates/engine-core` has zero external dependencies, `#![no_std]`, `#![forbid(unsafe_code)]`, and zero `std::time` / `std::fs` calls.
   - Confirm zero `.sh` or `.py` files exist anywhere in the repository.
2. **Zero Environment Leakage**:
   - Confirm neither `schema/compatibility.json` nor source/workflow files introduce `dev`, `staging`, `prod`, cluster, or namespace identifiers.
3. **Compatibility & Release Readiness**:
   - Confirm `schema/compatibility.json`, `Cargo.toml`, and `admission-engine --print-compatibility` match byte-for-byte.
4. **Commit Hygiene & File Size**:
   - Run `cargo xtask check-trust-boundary` to verify `Trust-Boundary: verifier` commits precede `crates/*/src/**` commits and never mix in a single commit.
   - Confirm every `.rs` file stays `<= 300` lines (hard gate `<= 350` lines) and `cargo xtask verify` succeeds cleanly.
