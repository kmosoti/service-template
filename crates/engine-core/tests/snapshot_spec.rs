#![forbid(unsafe_code)]

use engine_core::{
    CodecPolicy, Command, CoreError, EngineConfig, EngineState, MonoTime, SnapshotError,
    V1_TO_V2_AUDIT_EPOCH,
};

#[test]
fn mixed_writer_expand_round_trip_preserves_fields_and_defaults_audit_epoch() {
    // 1. Start with a v2 `expand` engine whose `audit_epoch` has advanced beyond 1.
    let v2_cfg = EngineConfig {
        config_schema_version: 1,
        window_ms: 1_000,
        max_units_per_window: 100,
        max_burst_units: 100,
        state_write_version: 1,
    };
    let mut v2_origin =
        EngineState::with_policy(v2_cfg.clone(), CodecPolicy::v2_expand()).expect("v2 init");
    v2_origin
        .apply(Command::Evaluate {
            now: MonoTime(100),
            subject: "alpha".into(),
            cost: 20,
        })
        .expect("window 0");
    v2_origin
        .apply(Command::Evaluate {
            now: MonoTime(2_400),
            subject: "alpha".into(),
            cost: 45,
        })
        .expect("window 2000 rollover");
    assert_eq!(v2_origin.window_epoch_ms(), 2_000);
    assert_eq!(v2_origin.consumed_units(), 45);
    assert_eq!(v2_origin.last_sequence(), 2);
    assert_eq!(v2_origin.audit_epoch(), 2);

    // 2. Encode as v1 during `expand` mixed-writer window.
    let v1_from_v2 = v2_origin
        .checkpoint(Some(1))
        .expect("v2 expand encodes v1 bridge snapshot");

    // 3. Restore onto a v1-only reader (`max_readable = 1`) and re-emit as v1.
    let v1_cfg = EngineConfig {
        state_write_version: 1,
        ..v2_cfg.clone()
    };
    let (ver_on_v1, v1_reader) =
        EngineState::from_snapshot_bytes(v1_cfg, CodecPolicy::v1_only(), &v1_from_v2)
            .expect("v1 reader restores v1 bridge snapshot");
    assert_eq!(ver_on_v1, 1);
    assert_eq!(v1_reader.window_epoch_ms(), 2_000);
    assert_eq!(v1_reader.consumed_units(), 45);
    assert_eq!(v1_reader.last_sequence(), 2);

    let v1_rewritten = v1_reader
        .checkpoint(Some(1))
        .expect("v1 reader writes v1 snapshot");

    // 4. Restore back onto a v2 `expand` engine: preserved fields are exact and
    //    `audit_epoch` defaults deterministically to `V1_TO_V2_AUDIT_EPOCH` (1).
    let (ver_on_v2, v2_restored) =
        EngineState::from_snapshot_bytes(v2_cfg, CodecPolicy::v2_expand(), &v1_rewritten)
            .expect("v2 expand restores v1 snapshot");
    assert_eq!(ver_on_v2, 1);
    assert_eq!(v2_restored.window_epoch_ms(), 2_000);
    assert_eq!(v2_restored.consumed_units(), 45);
    assert_eq!(v2_restored.last_sequence(), 2);
    assert_eq!(v2_restored.audit_epoch(), V1_TO_V2_AUDIT_EPOCH);
}

#[test]
fn contract_v2_only_snapshot_is_rejected_fail_closed_by_v1_reader() {
    let contract_cfg = EngineConfig {
        config_schema_version: 1,
        window_ms: 1_000,
        max_units_per_window: 100,
        max_burst_units: 50,
        state_write_version: 2,
    };
    let mut contract_engine =
        EngineState::with_policy(contract_cfg.clone(), CodecPolicy::v2_contract())
            .expect("v2 contract init");
    contract_engine
        .apply(Command::Evaluate {
            now: MonoTime(3_100),
            subject: "beta".into(),
            cost: 30,
        })
        .expect("admit beta");

    let mut v2_snapshot = contract_engine
        .checkpoint(Some(2))
        .expect("emit v2-only snapshot");

    let v1_cfg = EngineConfig {
        state_write_version: 1,
        ..contract_cfg
    };
    let rejected_by_v1 =
        EngineState::from_snapshot_bytes(v1_cfg, CodecPolicy::v1_only(), &v2_snapshot);
    assert_eq!(
        rejected_by_v1,
        Err(CoreError::Snapshot(SnapshotError::UnsupportedReadVersion {
            found: 2,
            min_readable: 1,
            current: 1,
        }))
    );

    // Also verify v2 `contract` engine rejects retired v1 snapshots (`min_readable = 2`)
    // and rejects tampered frames fail-closed.
    v2_snapshot[16] ^= 0xff;
    let tampered = EngineState::from_snapshot_bytes(
        EngineConfig::default(),
        CodecPolicy::v2_contract(),
        &v2_snapshot,
    );
    assert!(matches!(
        tampered,
        Err(CoreError::Snapshot(SnapshotError::ChecksumMismatch { .. }))
    ));
}
