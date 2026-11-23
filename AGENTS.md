# `service-template` Agent Operating Contract

You are operating inside **`service-template`** (Archetype 3 of 4: `research-template` -> `library-template` -> `service-template` -> `platform-template`).
This repository governs a **deployable transformation engine / protocol daemon** (`admission-engine`) whose deliverable is an **immutable, SLSA-attested binary artifact (`engine_digest`) paired with an explicit compatibility contract (`schema/compatibility.json`)**.

## Hard Architectural Invariants

1. **Zero `.sh` or `.py` Files**:
   - Never create `.sh` or `.py` files anywhere in the repository. All automation, checks, packaging, branch parity checks, and hooks live in `xtask/` (`#![forbid(unsafe_code)]`).
2. **Sans-I/O `engine-core` Isolation**:
   - `crates/engine-core` MUST remain `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, with **zero external dependencies** and zero `std::time` or `std::fs` references, compiling cleanly for `--target thumbv7em-none-eabi`.
3. **Zero Environment Coupling & Platform Config Interop**:
   - Never commit live `dev`, `staging`, or `prod` topologies into this repository or `schema/compatibility.json`.
   - `admission-engine --config <path>` accepts both standalone `EngineConfig` (`#[serde(deny_unknown_fields)]`) and downstream `platform.domain-config/v1` envelopes (`engine_config` projection).
4. **Zero-Drift Compatibility Descriptor (`schema/compatibility.json`)**:
   - Keep `schema/compatibility.json` (`transition_mode`: `"reversible" | "expand" | "contract" | "forward_only"`), `engine_core::CURRENT_COMPATIBILITY`, and `admission-engine --print-compatibility` in exact parity.
5. **Small Focused Modules**:
   - Keep every `.rs` file concise (`<= 300` lines target; `350` lines hard gate enforced by `cargo xtask verify`).
6. **Scoped Two-Commit `Trust-Boundary: verifier` Protocol**:
   - Never mix verifier/test edits (`xtask/**`, `.github/**`, `schema/**`, `crates/*/tests/**`) and production code (`crates/*/src/**`) in a single non-root commit.
   - `Trust-Boundary: verifier` MUST appear in the **bottom Git trailer block** of a commit that modifies at least one verifier file, and MUST precede contract/codec `crates/*/src/**` commits in any PR stack (`develop-agent` enforces rebase-only merges so 2-commit stacks stay intact).

## `.agents/` Workspace vs `.agents/plugins/service-forge/` Scope

- **`.agents/` (`service-template-agents`)**: Workspace-local Antigravity configuration active inside this repository (`.agents/skills/*` and `.agents/hooks.json` running `PreToolUse` on `run_command`).
- **`.agents/plugins/service-forge/` (`service-forge`)**: Standalone distributable Antigravity plugin package providing the `adversarial-breaker` and `stack-reviewer` subagents without duplicating workspace `hooks.json`.

## Governance & Boundary Honesty

- Agents push feature branches or integrate onto `develop-agent` (`gh pr merge <PR> --rebase`), never pushing directly to `main` or `release/*`, never approving `main-release`, `tag-release`, or `stable-publish` GitHub Environments, and never pushing unverified `v*` tags.
- Local 4-host `PreToolUse` hooks (`.agents/hooks.json`, `.claude/settings.json`, `.codex/hooks.json`, `.github/hooks/forge-guard.json`) are cooperative workflow guardrails (`{"decision":"allow"}` for Antigravity, `{}` for Claude/Codex/Copilot on neutral commands); authoritative enforcement lives server-side in the 5 GitHub Rulesets and 3 protected GitHub Environments (`SECURITY.md`).

## Standard Verification Loop

```bash
cargo xtask verify
cargo xtask package
```
