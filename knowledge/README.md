# Epistemic Knowledge Graph (`knowledge/`)

This directory stores durable engineering rationale for **`service-template`**, separated by epistemic status so agents and human maintainers never conflate empirical observations, external literature/analysis, and binding architectural decisions.

## Categories & Identifier Prefixes

| Directory | Prefix | `kind` | Purpose |
| :--- | :--- | :--- | :--- |
| `decisions/` | `D-NNNN` | `decision` | Binding architectural invariants, compatibility rules, and boundary contracts. |
| `research/` | `R-NNNN` | `research` | Literature synthesis, protocol analyses, and state-migration models. |
| `observations/` | `O-NNNN` | `observation` | Empirical build, packaging, performance, or failure-mode measurements. |

## Ratification Invariant

Every note carries YAML frontmatter validated by `cargo xtask verify` (`check-knowledge`):

- Notes authored by agents MUST declare `added_by: agent` and `human_ratified: false`.
- Only a human-reviewed pull request targeting `main` may flip `human_ratified` to `true`.
- Every path listed under `links` MUST resolve to a tracked repository file.

## Index

### Decisions
- [D-0001-sans-io-and-schema-descriptor.md](decisions/D-0001-sans-io-and-schema-descriptor.md) — Sans-I/O `engine-core` boundary paired with `schema/compatibility.json` as a first-class release contract.

### Research
- [R-0001-expand-contract-state-inertia.md](research/R-0001-expand-contract-state-inertia.md) — Expand/Contract state evolution and rollback-safe snapshot inertia for stateful services.

### Observations
- [O-0001-deterministic-engine-packaging.md](observations/O-0001-deterministic-engine-packaging.md) — Deterministic release archive digests (`engine_digest`) and SLSA provenance handoff to `platform-template`.
