//! Compatibility contract types and Expand/Contract state-inertia invariants.

pub const COMPATIBILITY_SCHEMA: &str = "service.compatibility/v1";
pub const SERVICE_NAME: &str = "admission-engine";
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
pub const STATE_FORMAT_VERSION: u32 = 2;
pub const MIN_READABLE_STATE_FORMAT: u32 = 1;
pub const OLDEST_WRITABLE_STATE_FORMAT: u32 = 1;
pub const PRESERVED_STATE_FIELDS: [&str; 3] =
    ["window_epoch_ms", "consumed_units", "last_sequence"];
pub const V1_TO_V2_AUDIT_EPOCH: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionMode {
    Reversible,
    Expand,
    Contract,
    ForwardOnly,
}

impl TransitionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reversible => "reversible",
            Self::Expand => "expand",
            Self::Contract => "contract",
            Self::ForwardOnly => "forward_only",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CompatError> {
        match raw {
            "reversible" => Ok(Self::Reversible),
            "expand" => Ok(Self::Expand),
            "contract" => Ok(Self::Contract),
            "forward_only" => Ok(Self::ForwardOnly),
            _ => Err(CompatError::UnknownTransitionMode),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatError {
    InvalidSchemaHeader,
    EmptyServiceName,
    ZeroVersionNumber,
    MinReadableExceedsCurrent,
    OldestWritableExceedsCurrent,
    MinReadableExceedsOldestWritable,
    ReversibleModeRequiresCurrentWritable,
    ExpandModeRequiresMixedVersionBridge,
    ContractModeRequiresCurrentWritable,
    ForwardOnlyRequiresCutoverBounds,
    MissingPreservedFields,
    ZeroAuditEpochDefault,
    UnknownTransitionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilitySpec<'a> {
    pub schema: &'a str,
    pub service_name: &'a str,
    pub config_schema_version: u32,
    pub state_format_version: u32,
    pub min_readable_state_format: u32,
    pub oldest_writable_state_format: u32,
    pub transition_mode: TransitionMode,
    pub preserved_state_fields: &'a [&'a str],
    pub v1_to_v2_audit_epoch: u64,
}

pub const CURRENT_COMPATIBILITY: CompatibilitySpec<'static> = CompatibilitySpec {
    schema: COMPATIBILITY_SCHEMA,
    service_name: SERVICE_NAME,
    config_schema_version: CONFIG_SCHEMA_VERSION,
    state_format_version: STATE_FORMAT_VERSION,
    min_readable_state_format: MIN_READABLE_STATE_FORMAT,
    oldest_writable_state_format: OLDEST_WRITABLE_STATE_FORMAT,
    transition_mode: TransitionMode::Expand,
    preserved_state_fields: &PRESERVED_STATE_FIELDS,
    v1_to_v2_audit_epoch: V1_TO_V2_AUDIT_EPOCH,
};

impl<'a> CompatibilitySpec<'a> {
    pub fn validate(&self) -> Result<(), CompatError> {
        if self.schema != COMPATIBILITY_SCHEMA {
            return Err(CompatError::InvalidSchemaHeader);
        }
        if self.service_name.is_empty() {
            return Err(CompatError::EmptyServiceName);
        }
        if self.config_schema_version == 0
            || self.state_format_version == 0
            || self.min_readable_state_format == 0
            || self.oldest_writable_state_format == 0
        {
            return Err(CompatError::ZeroVersionNumber);
        }
        if self.min_readable_state_format > self.state_format_version {
            return Err(CompatError::MinReadableExceedsCurrent);
        }
        if self.oldest_writable_state_format > self.state_format_version {
            return Err(CompatError::OldestWritableExceedsCurrent);
        }
        if self.min_readable_state_format > self.oldest_writable_state_format {
            return Err(CompatError::MinReadableExceedsOldestWritable);
        }
        if self.preserved_state_fields != PRESERVED_STATE_FIELDS {
            return Err(CompatError::MissingPreservedFields);
        }
        if self.v1_to_v2_audit_epoch == 0 {
            return Err(CompatError::ZeroAuditEpochDefault);
        }

        match self.transition_mode {
            TransitionMode::Reversible => {
                if self.oldest_writable_state_format != self.state_format_version {
                    return Err(CompatError::ReversibleModeRequiresCurrentWritable);
                }
            }
            TransitionMode::Expand => {
                if self.state_format_version < 2
                    || self.min_readable_state_format >= self.state_format_version
                    || self.oldest_writable_state_format >= self.state_format_version
                {
                    return Err(CompatError::ExpandModeRequiresMixedVersionBridge);
                }
            }
            TransitionMode::Contract => {
                if self.oldest_writable_state_format != self.state_format_version {
                    return Err(CompatError::ContractModeRequiresCurrentWritable);
                }
            }
            TransitionMode::ForwardOnly => {
                if self.oldest_writable_state_format != self.state_format_version
                    || self.min_readable_state_format != self.state_format_version
                {
                    return Err(CompatError::ForwardOnlyRequiresCutoverBounds);
                }
            }
        }
        Ok(())
    }

    pub const fn can_read_state_format(&self, version: u32) -> bool {
        version >= self.min_readable_state_format && version <= self.state_format_version
    }

    pub const fn can_write_state_format(&self, version: u32) -> bool {
        version >= self.oldest_writable_state_format && version <= self.state_format_version
    }
}
