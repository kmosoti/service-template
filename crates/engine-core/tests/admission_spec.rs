#![forbid(unsafe_code)]

use engine_core::{
    COMPATIBILITY_SCHEMA, CURRENT_COMPATIBILITY, CodecPolicy, Command, CompatError,
    CompatibilitySpec, CoreError, Decision, Effect, EngineConfig, EngineState, MonoTime,
    PRESERVED_STATE_FIELDS, RejectReason, SERVICE_NAME, SnapshotData, SnapshotError,
    TransitionMode, V1_TO_V2_AUDIT_EPOCH, encode_snapshot,
};

#[test]
fn deterministic_admission_burst_and_window_rollover() {
    let cfg = EngineConfig {
        config_schema_version: 1,
        window_ms: 1_000,
        max_units_per_window: 100,
        max_burst_units: 60,
        state_write_version: 2,
    };
    let mut engine = EngineState::new(cfg).expect("valid config");

    let admit = engine
        .apply(Command::Evaluate {
            now: MonoTime(100),
            subject: "tenant-a".into(),
            cost: 60,
        })
        .expect("evaluate 1");
    assert_eq!(
        admit,
        Effect::Evaluated {
            subject: "tenant-a".into(),
            sequence: 1,
            decision: Decision::Admit {
                remaining_units: 40,
                window_epoch_ms: 0,
            },
        }
    );

    let burst_denied = engine
        .apply(Command::Evaluate {
            now: MonoTime(200),
            subject: "tenant-a".into(),
            cost: 65,
        })
        .expect("burst check");
    assert_eq!(
        burst_denied,
        Effect::Evaluated {
            subject: "tenant-a".into(),
            sequence: 2,
            decision: Decision::Reject {
                reason: RejectReason::BurstExceeded,
                remaining_units: 40,
                retry_after_ms: 800,
            },
        }
    );

    let rate_limited = engine
        .apply(Command::Evaluate {
            now: MonoTime(500),
            subject: "tenant-a".into(),
            cost: 45,
        })
        .expect("rate limit check");
    assert_eq!(
        rate_limited,
        Effect::Evaluated {
            subject: "tenant-a".into(),
            sequence: 3,
            decision: Decision::Reject {
                reason: RejectReason::RateLimited,
                remaining_units: 40,
                retry_after_ms: 500,
            },
        }
    );

    let rollover = engine
        .apply(Command::Evaluate {
            now: MonoTime(1_100),
            subject: "tenant-a".into(),
            cost: 45,
        })
        .expect("window rollover");
    assert_eq!(
        rollover,
        Effect::Evaluated {
            subject: "tenant-a".into(),
            sequence: 4,
            decision: Decision::Admit {
                remaining_units: 55,
                window_epoch_ms: 1_000,
            },
        }
    );
}

#[test]
fn u64_max_limit_and_startup_config_constraints_fail_closed() {
    let max_cfg = EngineConfig {
        config_schema_version: 1,
        window_ms: 1_000,
        max_units_per_window: u64::MAX,
        max_burst_units: u64::MAX,
        state_write_version: 2,
    };
    let snap = encode_snapshot(
        &SnapshotData {
            window_epoch_ms: 1_000,
            consumed_units: u64::MAX,
            last_sequence: 10,
            audit_epoch: 2,
        },
        2,
        CodecPolicy::v2_expand(),
    )
    .expect("encode u64::MAX snapshot");

    let (_, mut engine) =
        EngineState::from_snapshot_bytes(max_cfg, CodecPolicy::v2_expand(), &snap)
            .expect("restore u64::MAX snapshot");
    let overflow_attempt = engine
        .apply(Command::Evaluate {
            now: MonoTime(1_200),
            subject: "overflow-probe".into(),
            cost: 1,
        })
        .expect("evaluate cost=1 at u64::MAX");
    assert_eq!(
        overflow_attempt,
        Effect::Evaluated {
            subject: "overflow-probe".into(),
            sequence: 11,
            decision: Decision::Reject {
                reason: RejectReason::RateLimited,
                remaining_units: 0,
                retry_after_ms: 800,
            },
        }
    );

    // Startup config constraint checks (`H13`):
    assert_eq!(
        EngineState::new(EngineConfig {
            window_ms: 0,
            ..EngineConfig::default()
        }),
        Err(CoreError::ZeroWindowDuration)
    );
    assert_eq!(
        EngineState::new(EngineConfig {
            max_units_per_window: 0,
            ..EngineConfig::default()
        }),
        Err(CoreError::ZeroUnitCapacity)
    );
    assert_eq!(
        EngineState::new(EngineConfig {
            max_burst_units: 0,
            ..EngineConfig::default()
        }),
        Err(CoreError::InvalidBurstCapacity {
            max_burst_units: 0,
            max_units_per_window: 100,
        })
    );
    assert_eq!(
        EngineState::new(EngineConfig {
            max_units_per_window: 50,
            max_burst_units: 51,
            ..EngineConfig::default()
        }),
        Err(CoreError::InvalidBurstCapacity {
            max_burst_units: 51,
            max_units_per_window: 50,
        })
    );
    assert_eq!(
        EngineState::with_policy(
            EngineConfig {
                state_write_version: 1,
                ..EngineConfig::default()
            },
            CodecPolicy::v2_contract(),
        ),
        Err(CoreError::Snapshot(
            SnapshotError::UnsupportedWriteVersion {
                requested: 1,
                oldest_writable: 2,
                current: 2,
            }
        ))
    );
    assert_eq!(
        EngineState::with_policy(
            EngineConfig {
                state_write_version: 3,
                ..EngineConfig::default()
            },
            CodecPolicy::v2_expand(),
        ),
        Err(CoreError::Snapshot(
            SnapshotError::UnsupportedWriteVersion {
                requested: 3,
                oldest_writable: 1,
                current: 2,
            }
        ))
    );

    // Reject snapshot restore when `consumed_units > max_units_per_window`.
    let overbudget_snap = encode_snapshot(
        &SnapshotData {
            window_epoch_ms: 0,
            consumed_units: 101,
            last_sequence: 1,
            audit_epoch: 1,
        },
        2,
        CodecPolicy::v2_expand(),
    )
    .expect("encode overbudget snapshot");
    assert_eq!(
        EngineState::from_snapshot_bytes(
            EngineConfig::default(),
            CodecPolicy::v2_expand(),
            &overbudget_snap
        ),
        Err(CoreError::SnapshotBudgetExceedsLimit {
            consumed_units: 101,
            max_units_per_window: 100,
        })
    );
}

#[test]
fn transition_mode_round_trips_all_four_platform_modes() {
    for (mode, wire) in [
        (TransitionMode::Reversible, "reversible"),
        (TransitionMode::Expand, "expand"),
        (TransitionMode::Contract, "contract"),
        (TransitionMode::ForwardOnly, "forward_only"),
    ] {
        assert_eq!(mode.as_str(), wire);
        assert_eq!(TransitionMode::parse(wire), Ok(mode));
    }
    assert_eq!(
        TransitionMode::parse("steady"),
        Err(CompatError::UnknownTransitionMode)
    );
    assert_eq!(CURRENT_COMPATIBILITY.validate(), Ok(()));

    let reversible_v1 = CompatibilitySpec {
        schema: COMPATIBILITY_SCHEMA,
        service_name: SERVICE_NAME,
        config_schema_version: 1,
        state_format_version: 1,
        min_readable_state_format: 1,
        oldest_writable_state_format: 1,
        transition_mode: TransitionMode::Reversible,
        preserved_state_fields: &PRESERVED_STATE_FIELDS,
        v1_to_v2_audit_epoch: V1_TO_V2_AUDIT_EPOCH,
    };
    assert_eq!(reversible_v1.validate(), Ok(()));

    let forward_only_v2 = CompatibilitySpec {
        min_readable_state_format: 2,
        oldest_writable_state_format: 2,
        transition_mode: TransitionMode::ForwardOnly,
        ..CURRENT_COMPATIBILITY
    };
    assert_eq!(forward_only_v2.validate(), Ok(()));

    let bad_forward_only = CompatibilitySpec {
        min_readable_state_format: 1,
        oldest_writable_state_format: 2,
        transition_mode: TransitionMode::ForwardOnly,
        ..CURRENT_COMPATIBILITY
    };
    assert_eq!(
        bad_forward_only.validate(),
        Err(CompatError::ForwardOnlyRequiresCutoverBounds)
    );
}
