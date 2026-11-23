//! Serde wire contracts for `admission-engine` stdio batches, compatibility descriptors,
//! and `platform.domain-config/v1` configuration envelopes.

use engine_core::{
    CURRENT_COMPATIBILITY, Command, CompatError, CompatibilitySpec, Decision, Effect, EngineConfig,
    MonoTime, RejectReason, SERVICE_NAME, TransitionMode,
};
use serde::{Deserialize, Serialize};

pub const PLATFORM_DOMAIN_CONFIG_SCHEMA: &str = "platform.domain-config/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionSpecificDefaults {
    pub v1_to_v2_audit_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityManifest {
    pub schema: String,
    pub service_name: String,
    pub config_schema_version: u32,
    pub state_format_version: u32,
    pub min_readable_state_format: u32,
    pub oldest_writable_state_format: u32,
    pub transition_mode: String,
    pub preserved_state_fields: Vec<String>,
    pub version_specific_defaults: VersionSpecificDefaults,
}

impl CompatibilityManifest {
    pub fn current() -> Self {
        Self {
            schema: CURRENT_COMPATIBILITY.schema.to_owned(),
            service_name: CURRENT_COMPATIBILITY.service_name.to_owned(),
            config_schema_version: CURRENT_COMPATIBILITY.config_schema_version,
            state_format_version: CURRENT_COMPATIBILITY.state_format_version,
            min_readable_state_format: CURRENT_COMPATIBILITY.min_readable_state_format,
            oldest_writable_state_format: CURRENT_COMPATIBILITY.oldest_writable_state_format,
            transition_mode: CURRENT_COMPATIBILITY.transition_mode.as_str().to_owned(),
            preserved_state_fields: CURRENT_COMPATIBILITY
                .preserved_state_fields
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            version_specific_defaults: VersionSpecificDefaults {
                v1_to_v2_audit_epoch: CURRENT_COMPATIBILITY.v1_to_v2_audit_epoch,
            },
        }
    }

    pub fn validate(&self) -> Result<(), CompatError> {
        let transition_mode = TransitionMode::parse(&self.transition_mode)?;
        let field_refs: Vec<&str> = self
            .preserved_state_fields
            .iter()
            .map(String::as_str)
            .collect();
        let spec = CompatibilitySpec {
            schema: &self.schema,
            service_name: &self.service_name,
            config_schema_version: self.config_schema_version,
            state_format_version: self.state_format_version,
            min_readable_state_format: self.min_readable_state_format,
            oldest_writable_state_format: self.oldest_writable_state_format,
            transition_mode,
            preserved_state_fields: &field_refs,
            v1_to_v2_audit_epoch: self.version_specific_defaults.v1_to_v2_audit_epoch,
        };
        spec.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub config_schema_version: u32,
    pub window_ms: u64,
    pub max_units_per_window: u64,
    pub max_burst_units: u64,
    pub state_write_version: u32,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self::from(EngineConfig::default())
    }
}

impl From<EngineConfig> for ServiceConfig {
    fn from(cfg: EngineConfig) -> Self {
        Self {
            config_schema_version: cfg.config_schema_version,
            window_ms: cfg.window_ms,
            max_units_per_window: cfg.max_units_per_window,
            max_burst_units: cfg.max_burst_units,
            state_write_version: cfg.state_write_version,
        }
    }
}

impl From<ServiceConfig> for EngineConfig {
    fn from(cfg: ServiceConfig) -> Self {
        Self {
            config_schema_version: cfg.config_schema_version,
            window_ms: cfg.window_ms,
            max_units_per_window: cfg.max_units_per_window,
            max_burst_units: cfg.max_burst_units,
            state_write_version: cfg.state_write_version,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformDomainEnvelope {
    pub schema: String,
    #[serde(default, alias = "service")]
    pub service_name: Option<String>,
    pub engine_config: ServiceConfig,
}

impl PlatformDomainEnvelope {
    pub fn into_service_config(self) -> Result<ServiceConfig, String> {
        if self.schema != PLATFORM_DOMAIN_CONFIG_SCHEMA {
            return Err(format!("unsupported envelope schema `{}`", self.schema));
        }
        if let Some(svc) = &self.service_name
            && svc != SERVICE_NAME
        {
            return Err(format!(
                "domain envelope targets service `{svc}`, expected `{SERVICE_NAME}`"
            ));
        }
        Ok(self.engine_config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireCommand {
    Evaluate {
        now_ms: u64,
        #[serde(alias = "tenant_id")]
        subject: String,
        cost: u64,
    },
    Snapshot {
        now_ms: u64,
        #[serde(default)]
        target_format_version: Option<u32>,
    },
}

impl From<WireCommand> for Command {
    fn from(cmd: WireCommand) -> Self {
        match cmd {
            WireCommand::Evaluate {
                now_ms,
                subject,
                cost,
            } => Self::Evaluate {
                now: MonoTime(now_ms),
                subject,
                cost,
            },
            WireCommand::Snapshot {
                now_ms,
                target_format_version,
            } => Self::Snapshot {
                now: MonoTime(now_ms),
                target_format_version,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WireResponse {
    Evaluated {
        subject: String,
        sequence: u64,
        admitted: bool,
        remaining_units: u64,
        window_epoch_ms: Option<u64>,
        reject_reason: Option<String>,
        retry_after_ms: Option<u64>,
    },
    SnapshotReady {
        format_version: u32,
        sequence: u64,
        payload_bytes: usize,
    },
}

impl From<&Effect> for WireResponse {
    fn from(effect: &Effect) -> Self {
        match effect {
            Effect::Evaluated {
                subject,
                sequence,
                decision,
            } => match decision {
                Decision::Admit {
                    remaining_units,
                    window_epoch_ms,
                } => Self::Evaluated {
                    subject: subject.clone(),
                    sequence: *sequence,
                    admitted: true,
                    remaining_units: *remaining_units,
                    window_epoch_ms: Some(*window_epoch_ms),
                    reject_reason: None,
                    retry_after_ms: None,
                },
                Decision::Reject {
                    reason,
                    remaining_units,
                    retry_after_ms,
                } => Self::Evaluated {
                    subject: subject.clone(),
                    sequence: *sequence,
                    admitted: false,
                    remaining_units: *remaining_units,
                    window_epoch_ms: None,
                    reject_reason: Some(format_reject(reason)),
                    retry_after_ms: Some(*retry_after_ms),
                },
            },
            Effect::SnapshotReady {
                format_version,
                sequence,
                payload,
            } => Self::SnapshotReady {
                format_version: *format_version,
                sequence: *sequence,
                payload_bytes: payload.len(),
            },
        }
    }
}

fn format_reject(reason: &RejectReason) -> String {
    match reason {
        RejectReason::EmptySubject => "empty_subject".to_owned(),
        RejectReason::InvalidCost => "invalid_cost".to_owned(),
        RejectReason::BurstExceeded => "burst_exceeded".to_owned(),
        RejectReason::RateLimited => "rate_limited".to_owned(),
        RejectReason::ClockRegression { .. } => "clock_regression".to_owned(),
    }
}
