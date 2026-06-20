//! Runtime configuration and asset path manifest types.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Host asset locations resolved by the CLI or future frontends.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetConfig {
    /// Optional SPC700 IPL ROM location.
    pub spc700_ipl: Option<PathBuf>,
    /// Optional base directory for saves.
    pub save_dir: Option<PathBuf>,
    /// Optional base directory for save states.
    pub state_dir: Option<PathBuf>,
}

/// User-tunable runtime options that should survive frontend changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Logging filter used by `tracing_subscriber`.
    pub log_filter: String,
    /// Whether the future frontend should start in dark mode.
    pub prefer_dark_mode: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            log_filter: "info,starbyte_core=debug,starbyte_cli=debug".to_owned(),
            prefer_dark_mode: true,
        }
    }
}
