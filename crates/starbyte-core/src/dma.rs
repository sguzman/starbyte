//! DMA/HDMA controller model for bootstrap correctness work.

use serde::{Deserialize, Serialize};

/// Number of CPU-visible DMA/HDMA channels.
pub const DMA_CHANNEL_COUNT: usize = 8;
/// Snapshot of one DMA channel's configured transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DmaChannel {
    /// DMAPx control byte.
    pub control: u8,
    /// BBADx destination register offset.
    pub b_bus_address: u8,
    /// A1Tx current transfer address.
    pub a_bus_address: u16,
    /// A1Bx source bank.
    pub a_bus_bank: u8,
    /// DASx byte count or indirect HDMA pointer.
    pub byte_count: u16,
    /// Resolved indirect HDMA data address.
    pub indirect_address: u16,
    /// A2Ax HDMA table address.
    pub hdma_table_address: u16,
    /// Current HDMA data pointer for direct-mode bytes.
    pub hdma_data_address: u16,
    /// NTRLx HDMA line counter / repeat flags.
    pub hdma_line_counter: u8,
    /// Whether this HDMA channel still has work to do this frame.
    pub hdma_active: bool,
    /// Whether the current HDMA block repeats without consuming new data.
    pub hdma_repeat: bool,
}

impl DmaChannel {
    /// Build a 24-bit A-bus source address.
    #[must_use]
    pub fn a_bus_full_address(self) -> u32 {
        (u32::from(self.a_bus_bank) << 16) | u32::from(self.a_bus_address)
    }

    /// Number of bytes implied by the current DMA byte count register.
    #[must_use]
    pub fn dma_length(self) -> usize {
        if self.byte_count == 0 {
            0x1_0000
        } else {
            usize::from(self.byte_count)
        }
    }

    /// Whether the transfer direction is B-bus to A-bus.
    #[must_use]
    pub const fn reverse_transfer(self) -> bool {
        self.control & 0x80 != 0
    }

    /// Whether the A-bus address remains fixed across the transfer.
    #[must_use]
    pub const fn fixed_transfer(self) -> bool {
        self.control & 0x08 != 0
    }

    /// Whether the A-bus address decrements instead of incrementing.
    #[must_use]
    pub const fn decrement_transfer(self) -> bool {
        self.control & 0x10 != 0
    }

    /// Whether HDMA uses indirect addressing for data bytes.
    #[must_use]
    pub const fn hdma_indirect(self) -> bool {
        self.control & 0x40 != 0
    }

    /// DMA/HDMA transfer mode number.
    #[must_use]
    pub const fn transfer_mode(self) -> u8 {
        self.control & 0x07
    }
}

/// DMA controller register image and execution bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmaController {
    channels: [DmaChannel; DMA_CHANNEL_COUNT],
    dma_enable_mask: u8,
    hdma_enable_mask: u8,
    /// Number of bytes transferred by DMA or HDMA in the bootstrap model.
    pub transfer_count: u64,
}

impl Default for DmaController {
    fn default() -> Self {
        Self {
            channels: [DmaChannel::default(); DMA_CHANNEL_COUNT],
            dma_enable_mask: 0,
            hdma_enable_mask: 0,
            transfer_count: 0,
        }
    }
}

impl DmaController {
    /// Read a CPU-visible DMA/HDMA register by offset from `$4300`.
    #[must_use]
    pub fn read_register(&self, offset: u16) -> u8 {
        let channel = usize::from(offset / 0x10);
        let register = offset % 0x10;
        let Some(channel) = self.channels.get(channel).copied() else {
            return 0;
        };

        match register {
            0x0 => channel.control,
            0x1 => channel.b_bus_address,
            0x2 => channel.a_bus_address as u8,
            0x3 => (channel.a_bus_address >> 8) as u8,
            0x4 => channel.a_bus_bank,
            0x5 => channel.byte_count as u8,
            0x6 => (channel.byte_count >> 8) as u8,
            0x7 => channel.indirect_address as u8,
            0x8 => channel.hdma_table_address as u8,
            0x9 => (channel.hdma_table_address >> 8) as u8,
            0xA => channel.hdma_line_counter,
            _ => 0,
        }
    }

    /// Write a CPU-visible DMA/HDMA register by offset from `$4300`.
    pub fn write_register(&mut self, offset: u16, value: u8) {
        let channel = usize::from(offset / 0x10);
        let register = offset % 0x10;
        let Some(channel) = self.channels.get_mut(channel) else {
            return;
        };

        match register {
            0x0 => channel.control = value,
            0x1 => channel.b_bus_address = value,
            0x2 => channel.a_bus_address = (channel.a_bus_address & 0xFF00) | u16::from(value),
            0x3 => {
                channel.a_bus_address = (channel.a_bus_address & 0x00FF) | (u16::from(value) << 8)
            }
            0x4 => channel.a_bus_bank = value,
            0x5 => channel.byte_count = (channel.byte_count & 0xFF00) | u16::from(value),
            0x6 => channel.byte_count = (channel.byte_count & 0x00FF) | (u16::from(value) << 8),
            0x7 => {
                channel.indirect_address = (channel.indirect_address & 0xFF00) | u16::from(value)
            }
            0x8 => {
                channel.hdma_table_address =
                    (channel.hdma_table_address & 0xFF00) | u16::from(value)
            }
            0x9 => {
                channel.hdma_table_address =
                    (channel.hdma_table_address & 0x00FF) | (u16::from(value) << 8)
            }
            0xA => channel.hdma_line_counter = value,
            _ => {}
        }
    }

    /// Current DMA enable mask written through `$420B`.
    #[must_use]
    pub const fn dma_enable_mask(&self) -> u8 {
        self.dma_enable_mask
    }

    /// Current HDMA enable mask written through `$420C`.
    #[must_use]
    pub const fn hdma_enable_mask(&self) -> u8 {
        self.hdma_enable_mask
    }

    /// Set the currently requested DMA mask.
    pub fn set_dma_enable_mask(&mut self, mask: u8) {
        self.dma_enable_mask = mask;
    }

    /// Set the currently requested HDMA mask.
    pub fn set_hdma_enable_mask(&mut self, mask: u8) {
        self.hdma_enable_mask = mask;
    }

    /// Borrow one configured DMA channel.
    #[must_use]
    pub fn channel(&self, index: usize) -> Option<&DmaChannel> {
        self.channels.get(index)
    }

    /// Replace one configured DMA channel.
    pub fn set_channel(&mut self, index: usize, channel: DmaChannel) {
        if let Some(slot) = self.channels.get_mut(index) {
            *slot = channel;
        }
    }

    /// Mutable access for execution helpers that need to update a channel in place.
    pub fn channel_mut(&mut self, index: usize) -> Option<&mut DmaChannel> {
        self.channels.get_mut(index)
    }

    /// Decode which B-bus register offsets a transfer mode touches.
    #[must_use]
    pub fn b_bus_offsets_for_mode(mode: u8) -> &'static [u8] {
        match mode & 0x07 {
            0 => &[0],
            1 => &[0, 1],
            2 => &[0, 0],
            3 => &[0, 0, 1, 1],
            4 => &[0, 1, 2, 3],
            5 => &[0, 1, 0, 1],
            6 => &[0, 0],
            7 => &[0, 0, 1, 1],
            _ => &[0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DmaChannel, DmaController};

    #[test]
    fn register_roundtrip_preserves_channel_shape() {
        let mut dma = DmaController::default();
        dma.write_register(0x00, 0x81);
        dma.write_register(0x01, 0x22);
        dma.write_register(0x02, 0x34);
        dma.write_register(0x03, 0x12);
        dma.write_register(0x04, 0x7E);
        dma.write_register(0x05, 0x78);
        dma.write_register(0x06, 0x56);
        dma.write_register(0x08, 0xCD);
        dma.write_register(0x09, 0xAB);
        dma.write_register(0x0A, 0xFE);

        let channel = dma.channel(0).copied().unwrap();
        assert_eq!(
            channel,
            DmaChannel {
                control: 0x81,
                b_bus_address: 0x22,
                a_bus_address: 0x1234,
                a_bus_bank: 0x7E,
                byte_count: 0x5678,
                indirect_address: 0,
                hdma_table_address: 0xABCD,
                hdma_data_address: 0,
                hdma_line_counter: 0xFE,
                hdma_active: false,
                hdma_repeat: false,
            }
        );
        assert_eq!(dma.read_register(0x03), 0x12);
        assert_eq!(dma.read_register(0x09), 0xAB);
    }

    #[test]
    fn exposes_transfer_mode_patterns() {
        assert_eq!(DmaController::b_bus_offsets_for_mode(0), &[0]);
        assert_eq!(DmaController::b_bus_offsets_for_mode(1), &[0, 1]);
        assert_eq!(DmaController::b_bus_offsets_for_mode(4), &[0, 1, 2, 3]);
    }
}
