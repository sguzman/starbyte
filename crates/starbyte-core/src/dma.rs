//! DMA/HDMA scaffolding.

use serde::{Deserialize, Serialize};

const DMA_REGISTER_COUNT: usize = 0x80;

/// DMA controller placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmaController {
    /// CPU-visible DMA/HDMA register image.
    registers: Vec<u8>,
    /// Number of transfers observed in the placeholder model.
    pub transfer_count: u64,
}

impl Default for DmaController {
    fn default() -> Self {
        Self {
            registers: vec![0; DMA_REGISTER_COUNT],
            transfer_count: 0,
        }
    }
}

impl DmaController {
    /// Read a DMA register by CPU-visible offset.
    #[must_use]
    pub fn read_register(&self, offset: u16) -> u8 {
        self.registers
            .get(usize::from(offset))
            .copied()
            .unwrap_or_default()
    }

    /// Write a DMA register by CPU-visible offset.
    pub fn write_register(&mut self, offset: u16, value: u8) {
        if let Some(slot) = self.registers.get_mut(usize::from(offset)) {
            *slot = value;
        }
    }
}
