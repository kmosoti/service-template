---
id: D-0001
kind: decision
status: proposed
date: 2026-01-15
added_by: agent
human_ratified: false
links:
  - knowledge/research/R-0001-expand-contract-state-inertia.md
  - knowledge/observations/O-0001-deterministic-engine-packaging.md
  - schema/compatibility.json
  - crates/engine-core/src/compat.rs
---

# D-0001: Sans-I/O Core Paired with First-Class Schema Compatibility Descriptor

## Context

Service repositories decay when host clock/filesystem I/O is interleaved with admission state logic or when live deployment topologies (`dev`, `staging`, `prod`) are embedded inside the service codebase.

## Decision

1. **Sans-I/O `engine-core`**: All window-epoch admission (`window_epoch_ms`, `consumed_units`, `last_sequence`, `audit_epoch`) and snapshot codec logic lives in `#![no_std]` `crates/engine-core` with zero external dependencies and overflow-safe `checked_add` budget accounting.
2. **Atomic Shell & Envelope Projection (`engine-service`)**: `admission-engine` accepts standalone `EngineConfig` (`#[serde(deny_unknown_fields)]`) and `platform.domain-config/v1` envelopes (`--config`), committing final post-batch checkpoints atomically via `<state-out>.tmp.<pid>` + `sync_all()` + `rename`.
3. **Shared `schema/compatibility.json` Contract**: Declares `service_name`, `config_schema_version`, `state_format_version`, `min_readable_state_format`, `oldest_writable_state_format`, `transition_mode`, `preserved_state_fields`, and `version_specific_defaults`, continuously parity-checked against `engine_core::CURRENT_COMPATIBILITY` and `admission-engine --print-compatibility`.
