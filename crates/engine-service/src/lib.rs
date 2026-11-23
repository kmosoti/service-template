#![forbid(unsafe_code)]

//! Thin imperative shell wrapping `engine-core` with JSON config validation,
//! `platform.domain-config/v1` envelope extraction, atomic checkpoint persistence,
//! and deterministic NDJSON batch execution.
//!
//! ## Checkpoint Durability Semantics (`--state-out`)
//! `--run-stdio-batch` evaluates the incoming NDJSON command stream to EOF, flushes
//! all NDJSON responses on `stdout`, encodes a snapshot of the **final post-batch
//! `EngineState`** using `state_write_version` (never an intermediate `Command::Snapshot`),
//! writes the bytes to a sibling temporary file `<state-out>.tmp.<pid>`, calls
//! `File::sync_all()`, and atomically renames onto `<state-out>`. Crashed or malformed
//! streams return an error before checkpoint commit, leaving any prior `<state-out>`
//! file untouched.

pub mod wire;

use std::fmt;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use engine_core::{
    CodecPolicy, Command, CompatError, CoreError, Decision, Effect, EngineConfig, EngineState,
    MonoTime,
};
use serde_json::Value;
pub use wire::{
    CompatibilityManifest, PLATFORM_DOMAIN_CONFIG_SCHEMA, PlatformDomainEnvelope, ServiceConfig,
    VersionSpecificDefaults, WireCommand, WireResponse,
};

#[derive(Debug)]
pub enum ServiceError {
    Io(std::io::Error),
    Json(serde_json::Error),
    ConfigEnvelope(String),
    Core(CoreError),
    Compatibility(CompatError),
    SelfCheckFailed(&'static str),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "JSON serialization error: {err}"),
            Self::ConfigEnvelope(msg) => write!(f, "config envelope error: {msg}"),
            Self::Core(err) => write!(f, "engine-core error: {err:?}"),
            Self::Compatibility(err) => write!(f, "compatibility error: {err:?}"),
            Self::SelfCheckFailed(msg) => write!(f, "self-check failed: {msg}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<std::io::Error> for ServiceError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for ServiceError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<CoreError> for ServiceError {
    fn from(err: CoreError) -> Self {
        Self::Core(err)
    }
}

impl From<CompatError> for ServiceError {
    fn from(err: CompatError) -> Self {
        Self::Compatibility(err)
    }
}

pub fn compatibility_manifest() -> CompatibilityManifest {
    CompatibilityManifest::current()
}

pub fn render_compatibility_json() -> Result<String, ServiceError> {
    let manifest = compatibility_manifest();
    manifest.validate()?;
    let mut rendered = serde_json::to_string_pretty(&manifest)?;
    rendered.push('\n');
    Ok(rendered)
}

pub fn load_config_file(path: &Path) -> Result<ServiceConfig, ServiceError> {
    let raw = fs::read_to_string(path)?;
    parse_config_str(&raw)
}

pub fn parse_config_str(raw: &str) -> Result<ServiceConfig, ServiceError> {
    let val: Value = serde_json::from_str(raw)?;
    if val.get("schema").and_then(Value::as_str) == Some(PLATFORM_DOMAIN_CONFIG_SCHEMA) {
        let env: PlatformDomainEnvelope = serde_json::from_value(val)?;
        return env
            .into_service_config()
            .map_err(ServiceError::ConfigEnvelope);
    }
    let cfg: ServiceConfig = serde_json::from_value(val)?;
    Ok(cfg)
}

pub fn self_check() -> Result<(), ServiceError> {
    let manifest = compatibility_manifest();
    manifest.validate()?;

    let cfg = EngineConfig::default();
    let policy = CodecPolicy::default();
    let mut engine = EngineState::with_policy(cfg.clone(), policy)?;

    let first = engine.apply(Command::Evaluate {
        now: MonoTime(500),
        subject: "self-check".to_owned(),
        cost: 25,
    })?;
    let Effect::Evaluated {
        decision: Decision::Admit {
            remaining_units: 75,
            ..
        },
        ..
    } = first
    else {
        return Err(ServiceError::SelfCheckFailed(
            "unexpected initial admission decision",
        ));
    };

    let checkpoint = engine.checkpoint(None)?;
    let (restored_version, mut restored) =
        EngineState::from_snapshot_bytes(cfg, policy, &checkpoint)?;
    if restored_version != manifest.state_format_version {
        return Err(ServiceError::SelfCheckFailed(
            "restored checkpoint version drifted from compatibility descriptor",
        ));
    }

    let second = restored.apply(Command::Evaluate {
        now: MonoTime(600),
        subject: "self-check".to_owned(),
        cost: 50,
    })?;
    let Effect::Evaluated {
        sequence: 2,
        decision: Decision::Admit {
            remaining_units: 25,
            ..
        },
        ..
    } = second
    else {
        return Err(ServiceError::SelfCheckFailed(
            "post-restore budget or sequence state drifted",
        ));
    };

    Ok(())
}

pub fn run_batch_stream<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    config: ServiceConfig,
    policy: CodecPolicy,
    initial_snapshot: Option<&[u8]>,
) -> Result<Vec<u8>, ServiceError> {
    let engine_cfg: EngineConfig = config.into();
    let mut engine = match initial_snapshot {
        Some(bytes) => EngineState::from_snapshot_bytes(engine_cfg, policy, bytes)?.1,
        None => EngineState::with_policy(engine_cfg, policy)?,
    };

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let wire_cmd: WireCommand = serde_json::from_str(trimmed)?;
        let effect = engine.apply(wire_cmd.into())?;
        let response = WireResponse::from(&effect);
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;

    let final_checkpoint = engine.checkpoint(None)?;
    Ok(final_checkpoint)
}

pub fn write_checkpoint_atomic(path: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => {
            fs::create_dir_all(p)?;
            p.to_path_buf()
        }
        _ => PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("checkpoint");
    let tmp_path = parent.join(format!("{file_name}.tmp.{}", std::process::id()));

    let write_res = (|| -> std::io::Result<()> {
        let mut tmp_file = fs::File::create(&tmp_path)?;
        tmp_file.write_all(bytes)?;
        tmp_file.sync_all()?;
        drop(tmp_file);
        fs::rename(&tmp_path, path)?;
        if let Ok(dir) = fs::File::open(&parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(ServiceError::Io(err));
    }
    Ok(())
}

pub fn run_stdio_batch_files(
    config_path: Option<&Path>,
    state_in: Option<&Path>,
    state_out: Option<&Path>,
) -> Result<(), ServiceError> {
    let config = match config_path {
        Some(path) => load_config_file(path)?,
        None => ServiceConfig::default(),
    };
    let initial_bytes = match state_in {
        Some(path) => Some(fs::read(path)?),
        None => None,
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let final_checkpoint = run_batch_stream(
        stdin.lock(),
        stdout.lock(),
        config,
        CodecPolicy::default(),
        initial_bytes.as_deref(),
    )?;

    if let Some(path) = state_out {
        write_checkpoint_atomic(path, &final_checkpoint)?;
    }
    Ok(())
}
