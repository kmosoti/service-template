# Security & Governance Trust Boundaries

## Vulnerability Reporting

Do not open public GitHub issues for exploitable vulnerabilities. Report security findings privately through GitHub Security Advisories on your instantiated repository.

## Enforcement Layers & Boundary Honesty

`service-template` separates **cooperative local developer/agent guardrails** from **server-enforced GitHub Rulesets and Environment boundaries**:

### 1. Server-Enforced Boundaries (Authoritative)
Configured declaratively via `cargo xtask github-setup <OWNER/REPO> [--main-pr-approvals <u8>]` and verified by typed JSON readback:

- **`service-template-main-protection` (`refs/heads/main`)**:
  - Blocks force-pushes, deletions, and direct pushes (`bypass_actors: []`).
  - Requires pull requests (`allowed_merge_methods: ["merge"]`), `--main-pr-approvals` human reviews (default `1`; `0` optional for solo maintainers), `gates` + `main-release-gate` checks, and human deployment approval on the **`main-release`** Environment.
- **`service-template-agent-protection` (`refs/heads/develop-agent`)**:
  - Enforces linear rebase-only merges (`allowed_merge_methods: ["rebase"]`) so 2-commit `Trust-Boundary: verifier` stacks remain distinct commits, plus passing `gates`.
- **`service-template-maintenance-protection` (`refs/heads/release/*`)**:
  - Enforces linear rebase-only merges, `main-release` Environment approval, and `gates` + `main-release-gate` status checks.
- **Split `refs/tags/v*` Rulesets (`service-template-tag-immutability` + `service-template-tag-creation`)**:
  - `service-template-tag-immutability` blocks `update`, `deletion`, and `non_fast_forward` with **`bypass_actors: []`** (nobody can rewrite or delete an existing `v*` tag).
  - `service-template-tag-creation` blocks `creation` for non-admin actors, restricting tag creation to Repository Administrators (`actor_id: 5`).
- **Credential-Isolated Tag Creation & Draft-First Immutable Releases**:
  - Maintainers can cut tags locally (`git tag -s vX.Y.Z <sha> && git push origin vX.Y.Z`) with zero GitHub Secrets, OR dispatch `.github/workflows/cut-release-tag.yml` from `main`, where `verify-candidate` runs unprivileged (`persist-credentials: false`, zero secrets) and `create-release-tag` runs API-only (`no checkout`, `no cargo`) inside the **`tag-release`** Environment (`main`-only branch policy).
  - `.github/workflows/release.yml` creates draft releases first, verifies asset SHA-256 digests against `dist/SHA256SUMS` (supporting idempotent retries), and atomically flips `--draft=false` under **`stable-publish`** for GitHub Immutable Releases compatibility.

### 2. Local `PreToolUse` Agent Hooks (Cooperative Guardrail, Not a Sandbox)
- Configured across Antigravity (`.agents/hooks.json`), Claude Code (`.claude/settings.json`), Codex (`.codex/hooks.json`), and Copilot (`.github/hooks/forge-guard.json`).
- Emits `{"decision":"allow"}` for Antigravity and `{}` for Claude/Codex/Copilot on neutral commands, and fails closed with exit `0` on malformed/empty JSON or policy violations. Client-side hooks prevent routine workflow drift; untrusted containment relies on OS/container sandboxing and the server-enforced GitHub Rulesets above.
