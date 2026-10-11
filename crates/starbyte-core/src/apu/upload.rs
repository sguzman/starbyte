//! SNES IPL sound-program transfer protocol.
//!
//! Implements the SPC-side loader exchange over APUIO0..3, including the
//! $CC initial handshake, byte-index acknowledgements, block boundaries,
//! eight-bit index wrap and an entry-point command. It is intentionally an
//! isolated, opt-in alternative to the existing production bootstrap shim.
//! It does not execute the uploaded sound program or ship an IPL ROM.
//!
//! Protocol reference: https://snes.nesdev.org/wiki/S-SMP#IPL_Boot_ROM

use serde::{Deserialize, Serialize};

/// Last independently verified step of an IPL upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UploadPhase {
    /// Initialized and waiting for the $CC command.
    Ready,
    /// Block address accepted; waiting for data byte with index zero.
    AwaitFirstByte,
    /// Copying indexed bytes to audio RAM.
    Receiving,
    /// A block has finished; waiting for another block or entry command.
    Executable,
}

/// Result of processing one complete CPU-to-APU mailbox snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadEvent {
    /// Nothing changed or no valid transfer action was observed.
    None,
    /// The CPU's $CC command was acknowledged.
    KickAccepted,
    /// One new byte was written to audio RAM.
    ByteWritten { address: u16, value: u8 },
    /// A block ended and a new upload destination was selected.
    NextBlock { destination: u16 },
    /// A zero-valued block marker immediately consumed the nonzero command
    /// byte as byte 0 at the next block address (real IPL fall-through).
    NextBlockWithByte { destination: u16, value: u8 },
    /// CPU requested that the uploaded program begin executing.
    EntryPoint { address: u16 },
}

/// Host-independent SPC IPL transfer decoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IplUpload {
    phase: UploadPhase,
    destination: u16,
    expected_index: u8,
    last_port0: Option<u8>,
    /// Accepted IPL data bytes, including writes blocked by SMP TEST.
    bytes_written: u64,
    /// Bytes actually written to SPC RAM, excluding TEST-gated ordinary RAM.
    #[serde(default)]
    bytes_stored: u64,
    /// Number of completed upload blocks.
    blocks_finished: u64,
    /// Last requested sound-program entrypoint.
    entrypoint: Option<u16>,
    /// Number of distinct out-of-sequence port-0 tokens ignored by IPL.
    #[serde(default)]
    ignored_tokens: u64,
}

impl Default for IplUpload {
    fn default() -> Self {
        Self {
            phase: UploadPhase::Ready,
            destination: 0,
            expected_index: 0,
            last_port0: None,
            bytes_written: 0,
            bytes_stored: 0,
            blocks_finished: 0,
            entrypoint: None,
            ignored_tokens: 0,
        }
    }
}

impl IplUpload {
    /// Reset the transfer state, without modifying uploaded RAM.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Return the current transfer phase.
    #[must_use]
    pub const fn phase(&self) -> UploadPhase {
        self.phase
    }

    /// Number of accepted transfer bytes (not necessarily stored in RAM).
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Accepted payload bytes that were actually stored in SPC RAM. TEST
    /// write protection may make this smaller than bytes_written().
    #[must_use]
    pub const fn bytes_stored(&self) -> u64 {
        self.bytes_stored
    }

    /// Number of completed blocks, including the last block before entry.
    #[must_use]
    pub const fn blocks_finished(&self) -> u64 {
        self.blocks_finished
    }

    /// Entry point selected by a completed zero-command handoff.
    #[must_use]
    pub const fn entrypoint(&self) -> Option<u16> {
        self.entrypoint
    }

    /// Next SPC RAM address awaiting data, or none before a block begins
    /// and after sound-program handoff. No uploaded content is exposed.
    #[must_use]
    pub const fn next_ram_address(&self) -> Option<u16> {
        match self.phase {
            UploadPhase::AwaitFirstByte | UploadPhase::Receiving => Some(self.destination),
            UploadPhase::Ready | UploadPhase::Executable => None,
        }
    }

    /// Number of distinct invalid port-0 tokens, excluding unchanged polls.
    #[must_use]
    pub const fn ignored_tokens(&self) -> u64 {
        self.ignored_tokens
    }

    /// Expected byte index at the next stage of the IPL transfer.
    #[must_use]
    pub const fn expected_index(&self) -> u8 {
        self.expected_index
    }

    /// Process one CPU mailbox snapshot, updating the SPC reply and RAM.
    ///
    /// The caller must supply each CPU port-0 write *after* port 1/2/3 have
    /// been updated. Changes to the other ports alone do not transfer data.
    /// The full 64 KiB audio RAM must be supplied. Incorrect lengths are
    /// rejected before any mutation.
    pub fn observe(
        &mut self,
        cpu_ports: [u8; 4],
        spc_ports: &mut [u8; 4],
        ram: &mut [u8],
    ) -> UploadEvent {
        self.observe_with_ram_writes(cpu_ports, spc_ports, ram, true)
    }

    /// Apply TEST bit 1 write gating while preserving mailbox acks and
    /// consumed byte indices. The IPL interpreter uses the same SRAM bus
    /// as any other SPC700 instruction, including the $F0-$FF exemption.
    pub(super) fn observe_with_ram_writes(
        &mut self,
        cpu_ports: [u8; 4],
        spc_ports: &mut [u8; 4],
        ram: &mut [u8],
        ram_writable: bool,
    ) -> UploadEvent {
        if ram.len() != 65_536 {
            return UploadEvent::None;
        }
        let token = cpu_ports[0];
        if self.last_port0 == Some(token) {
            return UploadEvent::None;
        }
        self.last_port0 = Some(token);

        match self.phase {
            UploadPhase::Executable => UploadEvent::None,
            UploadPhase::Ready => {
                if token != 0xcc {
                    return UploadEvent::None;
                }
                self.destination = u16::from_le_bytes([cpu_ports[2], cpu_ports[3]]);
                self.expected_index = 0;
                self.entrypoint = None;
                spc_ports[0] = token;
                // The IPL also supports starting an already resident program
                // directly: command zero and the entry address in ports 2/3.
                if cpu_ports[1] == 0 {
                    self.entrypoint = Some(self.destination);
                    self.phase = UploadPhase::Executable;
                    UploadEvent::EntryPoint {
                        address: self.destination,
                    }
                } else {
                    self.phase = UploadPhase::AwaitFirstByte;
                    UploadEvent::KickAccepted
                }
            }
            UploadPhase::AwaitFirstByte => {
                if token != 0 {
                    self.ignored_tokens = self.ignored_tokens.saturating_add(1);
                    return UploadEvent::None;
                }
                self.phase = UploadPhase::Receiving;
                self.write_byte(token, cpu_ports[1], spc_ports, ram, ram_writable)
            }
            UploadPhase::Receiving => {
                if token == self.expected_index {
                    return self.write_byte(token, cpu_ports[1], spc_ports, ram, ram_writable);
                }
                // The real IPL compares Y (the expected byte index) to
                // the CPU's port-0 token and takes the block-end path only
                // when the 8-bit subtraction has its sign bit set. A stale
                // token, or one too far ahead, is not a block boundary.
                // This comparison wraps across $FF->$00 like SPC700 CMP.
                if self.expected_index.wrapping_sub(token) & 0x80 == 0 {
                    self.ignored_tokens = self.ignored_tokens.saturating_add(1);
                    return UploadEvent::None;
                }
                // The expected index is consumed by data, not this
                // command. Repeated port-0 values are already ignored.
                self.blocks_finished = self.blocks_finished.saturating_add(1);
                let address = u16::from_le_bytes([cpu_ports[2], cpu_ports[3]]);
                spc_ports[0] = token;
                if cpu_ports[1] == 0 {
                    self.entrypoint = Some(address);
                    self.phase = UploadPhase::Executable;
                    UploadEvent::EntryPoint { address }
                } else {
                    self.destination = address;
                    self.expected_index = 0;
                    self.phase = UploadPhase::AwaitFirstByte;
                    if token == 0 {
                        // After an end marker $00, IPL's MOV Y,$F4/BNE
                        // falls straight through: it reads the still-live
                        // nonzero mode byte from port 1 as byte index 0.
                        // No second port-0 edge is required (or possible).
                        self.phase = UploadPhase::Receiving;
                        let _ = self.write_byte(0, cpu_ports[1], spc_ports, ram, ram_writable);
                        UploadEvent::NextBlockWithByte {
                            destination: address,
                            value: cpu_ports[1],
                        }
                    } else {
                        UploadEvent::NextBlock {
                            destination: address,
                        }
                    }
                }
            }
        }
    }

    fn write_byte(
        &mut self,
        token: u8,
        value: u8,
        spc_ports: &mut [u8; 4],
        ram: &mut [u8],
        ram_writable: bool,
    ) -> UploadEvent {
        let address = self.destination;
        if ram_writable || (0x00f0..=0x00ff).contains(&address) {
            ram[usize::from(address)] = value;
            self.bytes_stored = self.bytes_stored.saturating_add(1);
        }
        self.destination = address.wrapping_add(1);
        self.expected_index = token.wrapping_add(1);
        self.bytes_written = self.bytes_written.saturating_add(1);
        spc_ports[0] = token;
        UploadEvent::ByteWritten { address, value }
    }
}

#[cfg(test)]
mod tests {
    use super::{IplUpload, UploadEvent, UploadPhase};

    struct Harness {
        transfer: IplUpload,
        cpu: [u8; 4],
        spc: [u8; 4],
        ram: Vec<u8>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                transfer: IplUpload::default(),
                cpu: [0; 4],
                spc: [0xaa, 0xbb, 0, 0],
                ram: vec![0; 65_536],
            }
        }

        fn send(&mut self, token: u8) -> UploadEvent {
            self.cpu[0] = token;
            self.transfer
                .observe(self.cpu, &mut self.spc, &mut self.ram)
        }

        fn kick(&mut self, destination: u16) {
            let [lo, hi] = destination.to_le_bytes();
            self.cpu[1] = 1;
            self.cpu[2] = lo;
            self.cpu[3] = hi;
            assert_eq!(self.send(0xcc), UploadEvent::KickAccepted);
            assert_eq!(self.spc[0], 0xcc);
            assert_eq!(self.transfer.next_ram_address(), Some(destination));
        }

        fn send_byte(&mut self, token: u8, value: u8, address: u16) {
            self.cpu[1] = value;
            assert_eq!(
                self.send(token),
                UploadEvent::ByteWritten { address, value }
            );
            assert_eq!(self.spc[0], token);
        }
    }

    #[test]
    fn handoff_disables_loader_until_explicit_reset() {
        let mut h = Harness::new();
        h.cpu = [0, 0, 0x00, 0x04];
        assert_eq!(h.send(0xcc), UploadEvent::EntryPoint { address: 0x0400 });
        h.cpu[1] = 1;
        assert_eq!(h.send(0), UploadEvent::None);
        assert_eq!(h.send(0xcc), UploadEvent::None);
        assert_eq!(h.transfer.entrypoint(), Some(0x0400));
        assert_eq!(h.transfer.phase(), UploadPhase::Executable);

        h.transfer.reset();
        h.cpu[1] = 1;
        assert_eq!(h.send(0xcc), UploadEvent::KickAccepted);
        assert_eq!(h.transfer.phase(), UploadPhase::AwaitFirstByte);
    }

    #[test]
    fn initial_zero_command_jumps_to_resident_code_without_upload() {
        let mut h = Harness::new();
        h.cpu = [0, 0, 0x00, 0x04];
        assert_eq!(h.send(0xcc), UploadEvent::EntryPoint { address: 0x0400 });
        assert_eq!(h.spc[0], 0xcc);
        assert_eq!(h.transfer.entrypoint(), Some(0x0400));
        assert_eq!(h.transfer.bytes_written(), 0);
        assert_eq!(h.transfer.blocks_finished(), 0);
        assert_eq!(h.transfer.phase(), UploadPhase::Executable);
    }

    #[test]
    fn single_block_upload_and_entrypoint_preserve_byte_contents() {
        let mut h = Harness::new();
        h.kick(0x0200);
        h.send_byte(0, 0xe8, 0x0200);
        h.send_byte(1, 0x42, 0x0201);
        h.send_byte(2, 0x6f, 0x0202);
        h.cpu[1] = 0;
        h.cpu[2] = 0;
        h.cpu[3] = 2;
        assert_eq!(h.send(4), UploadEvent::EntryPoint { address: 0x0200 });
        assert_eq!(&h.ram[0x200..0x203], &[0xe8, 0x42, 0x6f]);
        assert_eq!(h.transfer.phase(), UploadPhase::Executable);
        assert_eq!(h.transfer.next_ram_address(), None);
        assert_eq!(h.transfer.entrypoint(), Some(0x0200));
        assert_eq!(h.transfer.bytes_written(), 3);
        assert_eq!(h.transfer.blocks_finished(), 1);
        assert_eq!(h.send(4), UploadEvent::None);
    }

    #[test]
    fn multi_block_upload_preserves_ram_and_acks_only_port0_changes() {
        let mut h = Harness::new();
        h.kick(0x10fe);
        h.send_byte(0, 0xaa, 0x10fe);
        h.send_byte(1, 0xbb, 0x10ff);
        h.cpu[1] = 0x44;
        assert_eq!(h.send(1), UploadEvent::None);
        assert_eq!(h.ram[0x1100], 0);
        h.cpu[1] = 1;
        h.cpu[2] = 0;
        h.cpu[3] = 0x40;
        assert_eq!(
            h.send(3),
            UploadEvent::NextBlock {
                destination: 0x4000
            }
        );
        h.send_byte(0, 0xcc, 0x4000);
        h.send_byte(1, 0xdd, 0x4001);
        h.cpu[1] = 0;
        h.cpu[2] = 0xfe;
        h.cpu[3] = 0x10;
        assert_eq!(h.send(4), UploadEvent::EntryPoint { address: 0x10fe });
        assert_eq!(&h.ram[0x10fe..0x1100], &[0xaa, 0xbb]);
        assert_eq!(&h.ram[0x4000..0x4002], &[0xcc, 0xdd]);
        assert_eq!(h.transfer.blocks_finished(), 2);
        assert_eq!(h.transfer.bytes_written(), 4);
    }

    #[test]
    fn stale_and_too_far_ahead_indices_do_not_terminate_an_ipl_block() {
        let mut h = Harness::new();
        h.kick(0x1200);
        h.send_byte(0, 0x11, 0x1200);
        h.send_byte(1, 0x22, 0x1201);
        h.send_byte(2, 0x33, 0x1202);
        // Expected index is now 3; a stale index and a forward token
        // 129 ahead are both rejected by the IPL's signed CMP.
        h.cpu[1] = 1;
        h.cpu[2] = 0;
        h.cpu[3] = 0x40;
        assert_eq!(h.send(1), UploadEvent::None);
        assert_eq!(h.send(132), UploadEvent::None);
        assert_eq!(h.transfer.phase(), UploadPhase::Receiving);
        assert_eq!(h.transfer.blocks_finished(), 0);
        assert_eq!(h.transfer.ignored_tokens(), 2);
        assert_eq!(h.transfer.expected_index(), 3);
        assert_eq!(h.spc[0], 2);

        // The next expected byte remains acceptable after invalid tokens.
        h.send_byte(3, 0x44, 0x1203);
        assert_eq!(h.transfer.bytes_written(), 4);
        assert_eq!(&h.ram[0x1200..0x1204], &[0x11, 0x22, 0x33, 0x44]);
        // A valid forward marker (expected=4, token=6) starts a new block.
        assert_eq!(
            h.send(6),
            UploadEvent::NextBlock {
                destination: 0x4000
            }
        );
        assert_eq!(h.transfer.blocks_finished(), 1);
        h.send_byte(0, 0x55, 0x4000);
    }

    #[test]
    fn ipl_block_marker_signed_comparison_wraps_at_index_ff() {
        let mut h = Harness::new();
        h.kick(0x3000);
        for n in 0..=253_u16 {
            h.send_byte(n as u8, n as u8, 0x3000 + n);
        }
        // Expected index is $FE. Token $FD is stale, while $00 is
        // forward by two with wrapped subtraction ($FE-$00) negative.
        h.cpu[1] = 1;
        h.cpu[2] = 0;
        h.cpu[3] = 0x40;
        assert_eq!(h.send(252), UploadEvent::None);
        assert_eq!(
            h.send(0),
            UploadEvent::NextBlockWithByte {
                destination: 0x4000,
                value: 1,
            }
        );
        assert_eq!(h.transfer.blocks_finished(), 1);
        assert_eq!(h.transfer.bytes_written(), 255);
        assert_eq!(h.transfer.bytes_stored(), 255);
        assert_eq!(h.ram[0x4000], 1);
        // The first byte is already consumed; a repeated index zero is
        // not a second transfer. The next legitimate index is one.
        assert_eq!(h.send(0), UploadEvent::None);
        assert_eq!(h.transfer.bytes_stored(), 255);
        h.send_byte(1, 0x77, 0x4001);
        assert_eq!(h.transfer.bytes_stored(), 256);
    }

    #[test]
    fn ipl_accepts_forward_128_marker_at_wrapped_index_zero() {
        let mut h = Harness::new();
        h.kick(0x2000);
        // After 256 data bytes, the next expected index is $00.
        for index in 0..256_u16 {
            h.send_byte(index as u8, index as u8, 0x2000 + index);
        }
        h.cpu[1] = 1;
        h.cpu[2] = 0;
        h.cpu[3] = 0x40;
        // CMP $00-$80 = $80 sets N, which takes the IPL block end.
        assert_eq!(
            h.send(0x80),
            UploadEvent::NextBlock {
                destination: 0x4000
            }
        );
        assert_eq!(h.transfer.blocks_finished(), 1);
        assert_eq!(h.transfer.bytes_written(), 256);
        h.send_byte(0, 0x55, 0x4000);
    }

    #[test]
    fn ignored_index_retains_upload_state_across_serialization() {
        let mut h = Harness::new();
        h.kick(0x2200);
        h.send_byte(0, 0x41, 0x2200);
        h.send_byte(1, 0x42, 0x2201);
        h.cpu[1] = 1;
        h.cpu[2] = 0;
        h.cpu[3] = 0x40;
        assert_eq!(h.send(0), UploadEvent::None);
        assert_eq!(h.transfer.ignored_tokens(), 1);
        let state = serde_json::to_string(&h.transfer).unwrap();
        h.transfer = serde_json::from_str(&state).unwrap();
        assert_eq!(h.send(0), UploadEvent::None);
        assert_eq!(h.transfer.ignored_tokens(), 1);
        assert_eq!(h.spc[0], 1);
        assert_eq!(
            h.send(3),
            UploadEvent::NextBlock {
                destination: 0x4000
            }
        );
        assert_eq!(&h.ram[0x2200..0x2202], &[0x41, 0x42]);
    }

    #[test]
    fn mid_transfer_serialization_keeps_expected_ack_index() {
        let mut h = Harness::new();
        h.kick(0x5000);
        h.send_byte(0, 0x8f, 0x5000);
        let saved = serde_json::to_string(&h.transfer).unwrap();
        h.transfer = serde_json::from_str(&saved).unwrap();

        // Restoring the decoder must not re-copy the last acknowledged byte.
        assert_eq!(h.send(0), UploadEvent::None);
        h.send_byte(1, 0xaa, 0x5001);
        h.send_byte(2, 0xf4, 0x5002);
        assert_eq!(&h.ram[0x5000..0x5003], &[0x8f, 0xaa, 0xf4]);
        assert_eq!(h.transfer.bytes_written(), 3);
        assert_eq!(h.transfer.phase(), UploadPhase::Receiving);
        assert_eq!(h.transfer.next_ram_address(), Some(0x5003));
    }

    #[test]
    fn disabled_ram_writes_consume_indices_without_storing_ordinary_bytes() {
        let mut h = Harness::new();
        h.kick(0x3000);
        h.cpu[1] = 0x44;
        assert_eq!(
            h.transfer.observe_with_ram_writes(
                [0, h.cpu[1], h.cpu[2], h.cpu[3]],
                &mut h.spc,
                &mut h.ram,
                false,
            ),
            UploadEvent::ByteWritten {
                address: 0x3000,
                value: 0x44,
            }
        );
        assert_eq!(h.ram[0x3000], 0);
        assert_eq!(h.transfer.bytes_written(), 1);
        assert_eq!(h.transfer.bytes_stored(), 0);
        assert_eq!(h.transfer.expected_index(), 1);
        h.cpu[1] = 0x77;
        h.cpu[0] = 1;
        assert_eq!(
            h.transfer.observe_with_ram_writes(h.cpu, &mut h.spc, &mut h.ram, true),
            UploadEvent::ByteWritten {
                address: 0x3001,
                value: 0x77,
            }
        );
        assert_eq!(h.ram[0x3001], 0x77);
        assert_eq!(h.transfer.bytes_written(), 2);
        assert_eq!(h.transfer.bytes_stored(), 1);
    }

    #[test]
    fn stored_bytes_include_mmio_overlay_even_when_test_blocks_ordinary_ram() {
        let mut h = Harness::new();
        h.kick(0x00ef);
        h.cpu[1] = 0x23;
        h.cpu[0] = 0;
        assert_eq!(
            h.transfer
                .observe_with_ram_writes(h.cpu, &mut h.spc, &mut h.ram, false),
            UploadEvent::ByteWritten {
                address: 0x00ef,
                value: 0x23,
            }
        );
        h.cpu[0] = 1;
        h.cpu[1] = 0x56;
        assert_eq!(
            h.transfer
                .observe_with_ram_writes(h.cpu, &mut h.spc, &mut h.ram, false),
            UploadEvent::ByteWritten {
                address: 0x00f0,
                value: 0x56,
            }
        );
        assert_eq!(h.ram[0x00ef], 0);
        assert_eq!(h.ram[0x00f0], 0x56);
        assert_eq!(h.transfer.bytes_written(), 2);
        assert_eq!(h.transfer.bytes_stored(), 1);
        let serialized = serde_json::to_string(&h.transfer).unwrap();
        let restored: IplUpload = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.bytes_stored(), 1);
        assert_eq!(restored.bytes_written(), 2);

        // Existing on-disk snapshots predate this optional counter. They
        // must continue loading without pretending historical writes were
        // observed by the newly introduced field.
        let mut legacy = serde_json::to_value(&h.transfer).unwrap();
        legacy.as_object_mut().unwrap().remove("bytes_stored");
        let legacy: IplUpload = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.bytes_written(), 2);
        assert_eq!(legacy.bytes_stored(), 0);
    }

    #[test]
    fn index_and_address_wrap_without_buffer_overrun() {
        let mut h = Harness::new();
        h.kick(0xfffc);
        for n in 0..=256_u16 {
            h.send_byte(n as u8, (n & 0x7f) as u8, 0xfffc_u16.wrapping_add(n));
        }
        assert_eq!(h.ram[0xfffc], 0);
        assert_eq!(h.ram[0xffff], 3);
        assert_eq!(h.ram[0], 4);
        assert_eq!(h.ram[0x00fc], 0);
        assert_eq!(h.transfer.bytes_written(), 257);
        let json = serde_json::to_string(&h.transfer).unwrap();
        let resumed: IplUpload = serde_json::from_str(&json).unwrap();
        assert_eq!(resumed.bytes_written(), h.transfer.bytes_written());
        assert_eq!(resumed.phase(), UploadPhase::Receiving);
    }

    #[test]
    fn unfinished_first_byte_does_not_write_memory() {
        let mut h = Harness::new();
        h.kick(0x3000);
        h.cpu[1] = 0x80;
        assert_eq!(h.send(7), UploadEvent::None);
        assert_eq!(h.transfer.bytes_written(), 0);
        assert_eq!(h.ram[0x3000], 0);
        let old = h.transfer.clone();
        assert_eq!(
            h.transfer.observe(h.cpu, &mut h.spc, &mut []),
            UploadEvent::None
        );
        assert_eq!(h.transfer.bytes_written(), old.bytes_written());
    }
}
