//! Versioned binary snapshot codec with Expand/Contract semantic projection rules.

use alloc::vec::Vec;

use crate::compat::{
    MIN_READABLE_STATE_FORMAT, OLDEST_WRITABLE_STATE_FORMAT, STATE_FORMAT_VERSION,
    V1_TO_V2_AUDIT_EPOCH,
};

const SNAPSHOT_MAGIC: &[u8; 8] = b"ADMSNAP\0";
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecPolicy {
    pub current_version: u32,
    pub min_readable_version: u32,
    pub oldest_writable_version: u32,
}

impl Default for CodecPolicy {
    fn default() -> Self {
        Self {
            current_version: STATE_FORMAT_VERSION,
            min_readable_version: MIN_READABLE_STATE_FORMAT,
            oldest_writable_version: OLDEST_WRITABLE_STATE_FORMAT,
        }
    }
}

impl CodecPolicy {
    pub const fn v1_only() -> Self {
        Self {
            current_version: 1,
            min_readable_version: 1,
            oldest_writable_version: 1,
        }
    }

    pub const fn v2_expand() -> Self {
        Self {
            current_version: 2,
            min_readable_version: 1,
            oldest_writable_version: 1,
        }
    }

    pub const fn v2_contract() -> Self {
        Self {
            current_version: 2,
            min_readable_version: 2,
            oldest_writable_version: 2,
        }
    }

    pub const fn can_read(&self, version: u32) -> bool {
        version >= self.min_readable_version && version <= self.current_version && version <= 2
    }

    pub const fn can_write(&self, version: u32) -> bool {
        version >= self.oldest_writable_version
            && version <= self.current_version
            && version >= 1
            && version <= 2
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotData {
    pub window_epoch_ms: u64,
    pub consumed_units: u64,
    pub last_sequence: u64,
    pub audit_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    BadMagic,
    TruncatedFrame,
    ChecksumMismatch {
        expected: u64,
        computed: u64,
    },
    UnsupportedReadVersion {
        found: u32,
        min_readable: u32,
        current: u32,
    },
    UnsupportedWriteVersion {
        requested: u32,
        oldest_writable: u32,
        current: u32,
    },
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn encode_snapshot(
    data: &SnapshotData,
    target_version: u32,
    policy: CodecPolicy,
) -> Result<Vec<u8>, SnapshotError> {
    if !policy.can_write(target_version) {
        return Err(SnapshotError::UnsupportedWriteVersion {
            requested: target_version,
            oldest_writable: policy.oldest_writable_version,
            current: policy.current_version,
        });
    }

    let mut buf = Vec::with_capacity(52);
    buf.extend_from_slice(SNAPSHOT_MAGIC);
    buf.extend_from_slice(&target_version.to_be_bytes());
    buf.extend_from_slice(&data.window_epoch_ms.to_be_bytes());
    buf.extend_from_slice(&data.consumed_units.to_be_bytes());
    buf.extend_from_slice(&data.last_sequence.to_be_bytes());
    if target_version >= 2 {
        buf.extend_from_slice(&data.audit_epoch.to_be_bytes());
    }

    let checksum = fnv1a_64(&buf);
    buf.extend_from_slice(&checksum.to_be_bytes());
    Ok(buf)
}

pub fn decode_snapshot(
    bytes: &[u8],
    policy: CodecPolicy,
) -> Result<(u32, SnapshotData), SnapshotError> {
    if bytes.len() < 8 + 4 + 8 + 8 + 8 + 8 {
        return Err(SnapshotError::TruncatedFrame);
    }
    if &bytes[..8] != SNAPSHOT_MAGIC {
        return Err(SnapshotError::BadMagic);
    }

    let payload_end = bytes.len() - 8;
    let expected = u64::from_be_bytes(
        bytes[payload_end..]
            .try_into()
            .map_err(|_| SnapshotError::TruncatedFrame)?,
    );
    let computed = fnv1a_64(&bytes[..payload_end]);
    if expected != computed {
        return Err(SnapshotError::ChecksumMismatch { expected, computed });
    }

    let mut cursor = 8usize;
    let version = read_u32(bytes, &mut cursor, payload_end)?;
    if !policy.can_read(version) {
        return Err(SnapshotError::UnsupportedReadVersion {
            found: version,
            min_readable: policy.min_readable_version,
            current: policy.current_version,
        });
    }

    let window_epoch_ms = read_u64(bytes, &mut cursor, payload_end)?;
    let consumed_units = read_u64(bytes, &mut cursor, payload_end)?;
    let last_sequence = read_u64(bytes, &mut cursor, payload_end)?;
    let audit_epoch = if version >= 2 {
        read_u64(bytes, &mut cursor, payload_end)?
    } else {
        V1_TO_V2_AUDIT_EPOCH
    };

    if cursor != payload_end {
        return Err(SnapshotError::TruncatedFrame);
    }

    Ok((
        version,
        SnapshotData {
            window_epoch_ms,
            consumed_units,
            last_sequence,
            audit_epoch,
        },
    ))
}

fn read_u32(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32, SnapshotError> {
    if *cursor + 4 > end {
        return Err(SnapshotError::TruncatedFrame);
    }
    let val = u32::from_be_bytes(
        bytes[*cursor..*cursor + 4]
            .try_into()
            .map_err(|_| SnapshotError::TruncatedFrame)?,
    );
    *cursor += 4;
    Ok(val)
}

fn read_u64(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u64, SnapshotError> {
    if *cursor + 8 > end {
        return Err(SnapshotError::TruncatedFrame);
    }
    let val = u64::from_be_bytes(
        bytes[*cursor..*cursor + 8]
            .try_into()
            .map_err(|_| SnapshotError::TruncatedFrame)?,
    );
    *cursor += 8;
    Ok(val)
}
