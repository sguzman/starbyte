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
    /// Total bytes actually copied into SPC audio RAM.
    bytes_written: u64,
    /// Number of completed upload blocks.
    blocks_finished: u64,
    /// Last requested sound-program entrypoint.
    entrypoint: Option<u16>,
}

impl Default for IplUpload {
    fn default() -> Self {
        Self {
            phase: UploadPhase::Ready,
            destination: 0,
            expected_index: 0,
            last_port0: None,
            bytes_written: 0,
            blocks_finished: 0,
            entrypoint: None,
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

    /// Number of successfully copied bytes.
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
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
        if ram.len() != 65_536 {
            return UploadEvent::None;
        }
        let token = cpu_ports[0];
        if self.last_port0 == Some(token) {
            return UploadEvent::None;
        }
        self.last_port0 = Some(token);

        match self.phase {
            UploadPhase::Ready | UploadPhase::Executable => {
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
                    return UploadEvent::None;
                }
                self.phase = UploadPhase::Receiving;
                self.write_byte(token, cpu_ports[1], spc_ports, ram)
            }
            UploadPhase::Receiving => {
                if token == self.expected_index {
                    return self.write_byte(token, cpu_ports[1], spc_ports, ram);
                }
                // A different index is the IPL's block-end marker. The
                // expected index is consumed by data, not by this command.
                // Repeated acknowledgements are ignored by last_port0.
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
                    UploadEvent::NextBlock {
                        destination: address,
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
    ) -> UploadEvent {
        let address = self.destination;
        ram[usize::from(address)] = value;
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
