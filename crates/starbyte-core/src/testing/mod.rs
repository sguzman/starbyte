//! Compliance and regression harness scaffolding.

pub mod cpu_65816;
pub mod spc700;

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
    pub const fn is_configured(&self) -> bool {
        self.cpu_65816_dir.is_some() || self.spc700_dir.is_some() || self.rom_suite_dir.is_some()
    }

    /// Resolve a suite path if present.
    #[must_use]
    pub fn suite_path<'a>(&self, path: &'a Option<PathBuf>) -> Option<&'a Path> {
        path.as_deref()
    }
}

/// Coarse summary of a discovered suite directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteSummary {
    /// Human-readable suite name.
    pub suite_name: &'static str,
    /// Number of JSON files discovered.
    pub file_count: usize,
    /// Number of vectors parsed successfully.
    pub vector_count: usize,
}

/// Aggregate outcome from executing a batch of compliance vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    /// Human-readable suite name.
    pub suite_name: &'static str,
    /// Number of executed vectors.
    pub total: usize,
    /// Number of vectors with no mismatches.
    pub passed: usize,
    /// Number of vectors with one or more mismatches.
    pub failed: usize,
    /// Example failures retained for diagnostics.
    pub failures: Vec<VectorFailure>,
}

/// One failing vector with human-readable mismatch reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorFailure {
    /// Human-readable vector label.
    pub label: String,
    /// Reasons the vector failed.
    pub reasons: Vec<String>,
}
