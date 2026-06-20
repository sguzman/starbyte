//! Audio processing unit scaffolding.

pub mod spc700;

use serde::{Deserialize, Serialize};

/// Buffered audio samples returned to a frontend.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    /// Interleaved stereo 16-bit samples.
    pub samples: Vec<i16>,
}
