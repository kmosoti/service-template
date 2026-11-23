---
id: R-0001
kind: research
status: active
date: 2026-01-15
added_by: agent
human_ratified: false
links:
  - knowledge/decisions/D-0001-sans-io-and-schema-descriptor.md
  - knowledge/observations/O-0001-deterministic-engine-packaging.md
  - crates/engine-core/src/codec.rs
---

# R-0001: Expand/Contract State Inertia in Stateful Service Upgrades

## Problem Statement

Stateful services carry **state inertia**: if a canary `v2` instance writes a `v2`-only snapshot that `v1` instances cannot decode, rolling back the binary to `v1` fails or drops budget state.

## Four-Mode `TransitionMode` Contract Shared with `platform-template`

1. **`reversible`**: steady single-format reversible operation (`oldest_writable_state_format == state_format_version`).
2. **`expand`**: mixed-writer rollout bridge (`state_format_version = 2`, `min_readable_state_format = 1`, `oldest_writable_state_format = 1`). Preserves `["window_epoch_ms", "consumed_units", "last_sequence"]` losslessly across `v2 -> v1 -> v2` round-trips while defaulting `v1_to_v2_audit_epoch = 1`.
3. **`contract`**: post-soak writable contraction (`oldest_writable_state_format == state_format_version`).
4. **`forward_only`**: explicit one-way cutover (`min_readable_state_format == state_format_version` and `oldest_writable_state_format == state_format_version`), where `v1`-only readers fail closed on `v2` snapshots.
