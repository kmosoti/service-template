---
name: idempotent-workflow
description: Execute tree-addressed verification receipts, rebase-only branch promotion, and deterministic builds without redundant work or working-tree dirt.
---

# Idempotent Workflow Skill

Use this skill to run fast, repeatable local and CI verification loops in `service-template`.

## Workflow

1. **Tree-Addressed Receipt Check**:
   - Run `cargo xtask verify`. When `git status --porcelain` is clean and `(HEAD^{tree}, Cargo.lock sha256, rustc --version)` matches `evidence/verification.json`, `xtask` returns immediately with `[idempotent cache hit]`.
   - Pass `--force` (`cargo xtask verify --force`) only when explicitly auditing deterministic re-execution or in CI gates.
2. **Isolated Build Targets**:
   - Keep `target/`, `evidence/`, and `dist/` untracked (per `.gitignore`). Never commit build artifacts.
3. **Linear Rebase History**:
   - Target `develop-agent` for routine integration PRs and always merge with `gh pr merge <PR> --rebase`.
   - Never merge into `main` or `release/*` as an agent; leave promotion PRs for human approval on the `main-release` Environment.
