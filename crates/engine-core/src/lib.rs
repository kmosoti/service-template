#![no_std]
#![forbid(unsafe_code)]

//! Pure Sans-I/O admission state machine and Expand/Contract snapshot codec.
//!
//! `engine-core` has zero external dependencies, performs no filesystem or clock
//! I/O, and compiles cleanly onto bare-metal `thumbv7em-none-eabi`.

extern crate alloc;

pub mod codec;
pub mod compat;
pub mod engine;
pub mod model;

pub use codec::{CodecPolicy, SnapshotData, SnapshotError, decode_snapshot, encode_snapshot};
pub use compat::{
    COMPATIBILITY_SCHEMA, CONFIG_SCHEMA_VERSION, CURRENT_COMPATIBILITY, CompatError,
    CompatibilitySpec, MIN_READABLE_STATE_FORMAT, OLDEST_WRITABLE_STATE_FORMAT,
    PRESERVED_STATE_FIELDS, SERVICE_NAME, STATE_FORMAT_VERSION, TransitionMode,
    V1_TO_V2_AUDIT_EPOCH,
};
pub use engine::EngineState;
pub use model::{Command, CoreError, Decision, Effect, EngineConfig, MonoTime, RejectReason};
