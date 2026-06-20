//! Compliance and regression harness scaffolding.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Disk locations for locally stored compliance corpora.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComplianceSuiteConfig {
    /// Directory containing 65816 single-step JSON vectors.
    pub cpu_65816_dir: Option<PathBuf>,
    /// Directory containing SPC700 single-step JSON vectors.
    pub spc700_dir: Option<PathBuf>,
    /// Directory containing ROM-based regression suites.
    pub rom_suite_dir: Option<PathBuf>,
}

impl ComplianceSuiteConfig {
    /// Return true when any compliance suite path has been configured.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.cpu_65816_dir.is_some() || self.spc700_dir.is_some() || self.rom_suite_dir.is_some()
    }

    /// Resolve a suite path if present.
    #[must_use]
    pub fn suite_path<'a>(&self, path: &'a Option<PathBuf>) -> Option<&'a Path> {
        path.as_deref()
    }
}
