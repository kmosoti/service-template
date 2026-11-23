# Contributing to `service-template`

## Branching, Environments & Promotion Flow

1. **Agent & Feature Integration (`develop-agent`)**:
   - Pull requests target `develop-agent` under `service-template-agent-protection` (`allowed_merge_methods: ["rebase"]`, preserving 2-commit `Trust-Boundary` stacks, required `gates` check).
2. **Human Promotion to `main` & Maintenance `release/X.Y`**:
   - Promoting `develop-agent` to `main` uses merge commits (`allowed_merge_methods: ["merge"]`) gated by `service-template-main-protection`, `gates` + `main-release-gate` status checks, and human approval on the **`main-release`** GitHub Environment (`--main-pr-approvals 0` supported in `github-setup` for solo maintainers).
   - Post-merge, `.github/workflows/sync-develop-agent.yml` runs read-only `cargo xtask sync-branches --check` (`permissions: contents: read`) to verify `develop-agent` ancestry and tree parity.
3. **Release Tags (`refs/tags/v*`) & Draft-First Publishing**:
   - Cut tags either locally as a repository admin (`git tag -s vX.Y.Z <sha> && git push origin vX.Y.Z`, zero GitHub Secrets) or via `.github/workflows/cut-release-tag.yml` (`tag-release` Environment).
   - `vX.Y.0` (and `-alpha.`/`-beta.`/`-rc.`) descend from `origin/main` or `origin/release/X.Y`; patch tags `vX.Y.Z` (`Z > 0`) descend strictly from `origin/release/X.Y`.
   - `.github/workflows/release.yml` uploads assets to a draft release, verifies SHA-256 digests, and atomically publishes (`stable-publish` Environment for stable tags).

## Scoped Two-Commit `Trust-Boundary: verifier` Protocol

Within any pull request (`cargo xtask check-trust-boundary --base origin/develop-agent`):

1. **Commit 1 (Verifier)**: modifies at least one verifier file (`xtask/**`, `.github/**`, `schema/**`, `crates/*/tests/**`), touches zero `crates/*/src/**` files, and ends with `Trust-Boundary: verifier` in the **bottom Git trailer block**.
2. **Commit 2 (Implementation)**: updates `crates/*/src/**` without modifying verifier files.

## Verification Before Opening a PR

```bash
cargo xtask verify
cargo xtask package
git config commit.template .github/.gitmessage
```
