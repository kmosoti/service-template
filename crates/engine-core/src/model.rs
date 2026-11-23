//! Domain commands, monotonic time, configuration, and admission effects.

use alloc::string::String;
use alloc::vec::Vec;

use crate::compat::{CONFIG_SCHEMA_VERSION, STATE_FORMAT_VERSION};

/// Deterministic monotonic timestamp in milliseconds injected by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct MonoTime(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    pub config_schema_version: u32,
    pub window_ms: u64,
    pub max_units_per_window: u64,
    pub max_burst_units: u64,
    pub state_write_version: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            config_schema_version: CONFIG_SCHEMA_VERSION,
            window_ms: 1_000,
            max_units_per_window: 100,
            max_burst_units: 100,
            state_write_version: STATE_FORMAT_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Evaluate {
        now: MonoTime,
        subject: String,
        cost: u64,
    },
    Snapshot {
        now: MonoTime,
        target_format_version: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    EmptySubject,
    InvalidCost,
    BurstExceeded,
    RateLimited,
    ClockRegression {
        current_epoch_ms: u64,
        provided_epoch_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Admit {
        remaining_units: u64,
        window_epoch_ms: u64,
    },
    Reject {
        reason: RejectReason,
        remaining_units: u64,
        retry_after_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Evaluated {
        subject: String,
        sequence: u64,
        decision: Decision,
    },
    SnapshotReady {
        format_version: u32,
        sequence: u64,
        payload: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    UnsupportedConfigSchema {
        expected: u32,
        found: u32,
    },
    ZeroWindowDuration,
    ZeroUnitCapacity,
    InvalidBurstCapacity {
        max_burst_units: u64,
        max_units_per_window: u64,
    },
    SnapshotBudgetExceedsLimit {
        consumed_units: u64,
        max_units_per_window: u64,
    },
    Snapshot(crate::codec::SnapshotError),
}

impl From<crate::codec::SnapshotError> for CoreError {
    fn from(err: crate::codec::SnapshotError) -> Self {
        Self::Snapshot(err)
    }
}
