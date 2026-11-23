//! Deterministic Sans-I/O admission engine state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::{CodecPolicy, SnapshotData, SnapshotError, decode_snapshot, encode_snapshot};
use crate::compat::{CONFIG_SCHEMA_VERSION, V1_TO_V2_AUDIT_EPOCH};
use crate::model::{Command, CoreError, Decision, Effect, EngineConfig, MonoTime, RejectReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineState {
    config: EngineConfig,
    policy: CodecPolicy,
    window_epoch_ms: u64,
    consumed_units: u64,
    last_sequence: u64,
    audit_epoch: u64,
}

impl EngineState {
    pub fn new(config: EngineConfig) -> Result<Self, CoreError> {
        Self::with_policy(config, CodecPolicy::default())
    }

    pub fn with_policy(config: EngineConfig, policy: CodecPolicy) -> Result<Self, CoreError> {
        validate_config(&config, policy)?;
        Ok(Self {
            config,
            policy,
            window_epoch_ms: 0,
            consumed_units: 0,
            last_sequence: 0,
            audit_epoch: V1_TO_V2_AUDIT_EPOCH,
        })
    }

    pub fn from_snapshot_bytes(
        config: EngineConfig,
        policy: CodecPolicy,
        snapshot_bytes: &[u8],
    ) -> Result<(u32, Self), CoreError> {
        validate_config(&config, policy)?;
        let (loaded_version, data) = decode_snapshot(snapshot_bytes, policy)?;
        if data.consumed_units > config.max_units_per_window {
            return Err(CoreError::SnapshotBudgetExceedsLimit {
                consumed_units: data.consumed_units,
                max_units_per_window: config.max_units_per_window,
            });
        }
        let audit_epoch = if data.audit_epoch == 0 {
            V1_TO_V2_AUDIT_EPOCH
        } else {
            data.audit_epoch
        };
        Ok((
            loaded_version,
            Self {
                config,
                policy,
                window_epoch_ms: data.window_epoch_ms,
                consumed_units: data.consumed_units,
                last_sequence: data.last_sequence,
                audit_epoch,
            },
        ))
    }

    pub const fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub const fn window_epoch_ms(&self) -> u64 {
        self.window_epoch_ms
    }

    pub const fn consumed_units(&self) -> u64 {
        self.consumed_units
    }

    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub const fn audit_epoch(&self) -> u64 {
        self.audit_epoch
    }

    pub fn snapshot_data(&self) -> SnapshotData {
        SnapshotData {
            window_epoch_ms: self.window_epoch_ms,
            consumed_units: self.consumed_units,
            last_sequence: self.last_sequence,
            audit_epoch: self.audit_epoch,
        }
    }

    /// Encodes a checkpoint of the current post-batch state without mutating `last_sequence`.
    pub fn checkpoint(&self, target_format_version: Option<u32>) -> Result<Vec<u8>, CoreError> {
        let format_version = target_format_version.unwrap_or(self.config.state_write_version);
        let bytes = encode_snapshot(&self.snapshot_data(), format_version, self.policy)?;
        Ok(bytes)
    }

    pub fn apply(&mut self, command: Command) -> Result<Effect, CoreError> {
        match command {
            Command::Evaluate { now, subject, cost } => Ok(self.evaluate(now, subject, cost)),
            Command::Snapshot {
                now,
                target_format_version,
            } => self.snapshot_command(now, target_format_version),
        }
    }

    fn evaluate(&mut self, now: MonoTime, subject: String, cost: u64) -> Effect {
        self.last_sequence = self.last_sequence.saturating_add(1);
        let provided_epoch_ms = (now.0 / self.config.window_ms) * self.config.window_ms;

        if provided_epoch_ms < self.window_epoch_ms {
            let remaining = self.config.max_units_per_window - self.consumed_units;
            return Effect::Evaluated {
                subject,
                sequence: self.last_sequence,
                decision: Decision::Reject {
                    reason: RejectReason::ClockRegression {
                        current_epoch_ms: self.window_epoch_ms,
                        provided_epoch_ms,
                    },
                    remaining_units: remaining,
                    retry_after_ms: self.window_epoch_ms - provided_epoch_ms,
                },
            };
        }

        if provided_epoch_ms > self.window_epoch_ms {
            self.window_epoch_ms = provided_epoch_ms;
            self.consumed_units = 0;
            self.audit_epoch = self.audit_epoch.saturating_add(1);
        }

        let remaining_before = self.config.max_units_per_window - self.consumed_units;
        if subject.is_empty() {
            return Effect::Evaluated {
                subject,
                sequence: self.last_sequence,
                decision: Decision::Reject {
                    reason: RejectReason::EmptySubject,
                    remaining_units: remaining_before,
                    retry_after_ms: 0,
                },
            };
        }

        if cost == 0 {
            return Effect::Evaluated {
                subject,
                sequence: self.last_sequence,
                decision: Decision::Reject {
                    reason: RejectReason::InvalidCost,
                    remaining_units: remaining_before,
                    retry_after_ms: 0,
                },
            };
        }

        let window_reset_ms = self.window_epoch_ms.saturating_add(self.config.window_ms);
        let retry_after_ms = window_reset_ms.saturating_sub(now.0);

        if cost > self.config.max_burst_units {
            return Effect::Evaluated {
                subject,
                sequence: self.last_sequence,
                decision: Decision::Reject {
                    reason: RejectReason::BurstExceeded,
                    remaining_units: remaining_before,
                    retry_after_ms,
                },
            };
        }

        let decision = match self.consumed_units.checked_add(cost) {
            Some(next) if next <= self.config.max_units_per_window => {
                self.consumed_units = next;
                Decision::Admit {
                    remaining_units: self.config.max_units_per_window - next,
                    window_epoch_ms: self.window_epoch_ms,
                }
            }
            _ => Decision::Reject {
                reason: RejectReason::RateLimited,
                remaining_units: remaining_before,
                retry_after_ms,
            },
        };

        Effect::Evaluated {
            subject,
            sequence: self.last_sequence,
            decision,
        }
    }

    fn snapshot_command(
        &mut self,
        now: MonoTime,
        target_format_version: Option<u32>,
    ) -> Result<Effect, CoreError> {
        let format_version = target_format_version.unwrap_or(self.config.state_write_version);
        let provided_epoch_ms = (now.0 / self.config.window_ms) * self.config.window_ms;
        let mut next = self.clone();
        if provided_epoch_ms > next.window_epoch_ms {
            next.window_epoch_ms = provided_epoch_ms;
            next.consumed_units = 0;
            next.audit_epoch = next.audit_epoch.saturating_add(1);
        }
        next.last_sequence = next.last_sequence.saturating_add(1);
        let payload = encode_snapshot(&next.snapshot_data(), format_version, self.policy)?;
        *self = next;
        Ok(Effect::SnapshotReady {
            format_version,
            sequence: self.last_sequence,
            payload,
        })
    }
}

fn validate_config(config: &EngineConfig, policy: CodecPolicy) -> Result<(), CoreError> {
    if config.config_schema_version != CONFIG_SCHEMA_VERSION {
        return Err(CoreError::UnsupportedConfigSchema {
            expected: CONFIG_SCHEMA_VERSION,
            found: config.config_schema_version,
        });
    }
    if config.window_ms == 0 {
        return Err(CoreError::ZeroWindowDuration);
    }
    if config.max_units_per_window == 0 {
        return Err(CoreError::ZeroUnitCapacity);
    }
    if config.max_burst_units == 0 || config.max_burst_units > config.max_units_per_window {
        return Err(CoreError::InvalidBurstCapacity {
            max_burst_units: config.max_burst_units,
            max_units_per_window: config.max_units_per_window,
        });
    }
    if !policy.can_write(config.state_write_version) {
        return Err(CoreError::Snapshot(
            SnapshotError::UnsupportedWriteVersion {
                requested: config.state_write_version,
                oldest_writable: policy.oldest_writable_version,
                current: policy.current_version,
            },
        ));
    }
    Ok(())
}
