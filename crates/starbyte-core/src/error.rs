//! Shared error types for Starbyte.

use std::path::PathBuf;

use thiserror::Error;

/// Shared result type for the core crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by Starbyte core services.
#[derive(Debug, Error)]
pub enum Error {
    /// Generic host or content IO failure.
    #[error("I/O error while accessing {path:?}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Wrapped source error.
        #[source]
        source: std::io::Error,
    },
    /// A ROM image could not be parsed.
    #[error("invalid ROM: {0}")]
    InvalidRom(String),
    /// A requested firmware blob is missing.
    #[error("missing firmware: {name}")]
    MissingFirmware {
        /// Firmware display name.
        name: &'static str,
    },
    /// A user-supplied firmware blob failed validation.
    #[error("invalid firmware for {name}: {details}")]
    InvalidFirmware {
        /// Firmware display name.
        name: &'static str,
        /// Validation details.
        details: String,
    },
    /// User-supplied save RAM does not match cartridge expectations.
    #[error("invalid save RAM: expected {expected} bytes, got {actual}")]
    InvalidSaveRam {
        /// Cartridge-advertised save RAM byte count.
        expected: usize,
        /// Actual supplied save RAM byte count.
        actual: usize,
    },
    /// Requested functionality is intentionally deferred.
    #[error("feature not implemented yet: {0}")]
    Unimplemented(&'static str),
    /// CPU opcode is not implemented by the current core.
    #[error("unsupported opcode for {cpu}: 0x{opcode:02X}")]
    UnsupportedOpcode {
        /// CPU identifier.
        cpu: &'static str,
        /// Opcode byte.
        opcode: u8,
    },
    /// State or data serialization failure.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Config parsing failure.
    #[error("configuration error: {0}")]
    Config(#[from] toml::de::Error),
}

impl Error {
    /// Create an IO error variant with a path.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
