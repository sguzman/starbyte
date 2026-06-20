//! DMA/HDMA scaffolding.

use serde::{Deserialize, Serialize};

/// DMA controller placeholder.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmaController {
    /// Number of transfers observed in the placeholder model.
    pub transfer_count: u64,
}
