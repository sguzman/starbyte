//! Cartridge coprocessor metadata and bootstrap runtime models.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;
use crate::cartridge::{CartridgeHeader, Mapper};

const DSP_STATUS_READY: u8 = 0x80;
const DSP_STATUS_DATA_AVAILABLE: u8 = 0x40;
const DSP_STATUS_COMMAND_WAITING: u8 = 0x20;

/// Coarse coprocessor family derived from the cartridge header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoprocessorKind {
    /// NEC uPD77C25-based DSP family (`DSP-1/2/3/4`).
    Dsp,
    /// GSU / SuperFX family.
    SuperFx,
    /// OBC1 object controller.
    Obc1,
    /// SA-1 companion CPU.
    Sa1,
    /// S-DD1 decompression chip.
    Sdd1,
    /// S-RTC real-time clock.
    SRtc,
    /// Custom coprocessor identified by the expanded header.
    Custom(u8),
}

impl CoprocessorKind {
    /// Detect coprocessor class from the cartridge header chipset field.
    #[must_use]
    pub fn detect(header: &CartridgeHeader) -> Option<Self> {
        let chipset = header.rom_type;
        if chipset < 0x03 {
            return None;
        }

        match (chipset >> 4) & 0x0F {
            0x0 => Some(Self::Dsp),
            0x1 => Some(Self::SuperFx),
            0x2 => Some(Self::Obc1),
            0x3 => Some(Self::Sa1),
            0x4 => Some(Self::Sdd1),
            0x5 => Some(Self::SRtc),
            0xF => Some(Self::Custom(0)),
            _ => None,
        }
    }
}

impl std::fmt::Display for CoprocessorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp => f.write_str("DSP"),
            Self::SuperFx => f.write_str("SuperFX"),
            Self::Obc1 => f.write_str("OBC1"),
            Self::Sa1 => f.write_str("SA-1"),
            Self::Sdd1 => f.write_str("S-DD1"),
            Self::SRtc => f.write_str("S-RTC"),
            Self::Custom(value) => write!(f, "Custom(0x{value:02X})"),
        }
    }
}

/// Runtime coprocessor slot attached to the system bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Coprocessor {
    /// Bootstrap DSP register model with minimal command plumbing.
    Dsp(DspCoprocessor),
}

impl Coprocessor {
    /// Build the runtime coprocessor state for a cartridge, if any.
    #[must_use]
    pub fn for_cartridge(header: &CartridgeHeader) -> Option<Self> {
        match CoprocessorKind::detect(header)? {
            CoprocessorKind::Dsp => Some(Self::Dsp(DspCoprocessor::new(header))),
            _ => None,
        }
    }

    /// Return the coprocessor family.
    #[must_use]
    pub const fn kind(&self) -> CoprocessorKind {
        match self {
            Self::Dsp(_) => CoprocessorKind::Dsp,
        }
    }

    /// Reset transient runtime state.
    pub fn reset(&mut self) {
        match self {
            Self::Dsp(dsp) => dsp.reset(),
        }
    }

    /// Attempt to service a CPU read from a coprocessor-mapped address.
    #[must_use]
    pub fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        match self {
            Self::Dsp(dsp) => dsp.read(mapper, address),
        }
    }

    /// Attempt to service a CPU write to a coprocessor-mapped address.
    pub fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        match self {
            Self::Dsp(dsp) => dsp.write(mapper, address, value),
        }
    }
}

/// Bootstrap DSP-family runtime model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspCoprocessor {
    map: DspAddressMap,
    status_register: u8,
    write_latch: u16,
    read_latch: u16,
    low_byte_latched: bool,
    read_high_pending: bool,
    active_command: Option<DspCommand>,
    operand_words: Vec<i16>,
    result_words: VecDeque<i16>,
}

impl DspCoprocessor {
    fn new(header: &CartridgeHeader) -> Self {
        let map = DspAddressMap::for_header(header);
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            title = %header.title,
            mapper = ?header.mapper,
            map = ?map,
            rom_type = header.rom_type,
            "initializing DSP coprocessor"
        );
        Self {
            map,
            status_register: DSP_STATUS_READY,
            write_latch: 0,
            read_latch: 0,
            low_byte_latched: false,
            read_high_pending: false,
            active_command: None,
            operand_words: Vec::new(),
            result_words: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            "resetting DSP coprocessor"
        );
        self.status_register = DSP_STATUS_READY;
        self.write_latch = 0;
        self.read_latch = 0;
        self.low_byte_latched = false;
        self.read_high_pending = false;
        self.active_command = None;
        self.operand_words.clear();
        self.result_words.clear();
    }

    fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let register = self.map.decode(mapper, address)?;
        let value = match register {
            DspRegister::DataLow => {
                self.refresh_read_latch();
                self.read_latch.to_le_bytes()[0]
            }
            DspRegister::DataHigh => {
                self.refresh_read_latch();
                self.read_latch.to_le_bytes()[1]
            }
            DspRegister::Status => self.status_register,
        };

        match register {
            DspRegister::DataLow => {
                self.read_high_pending = true;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "read DSP data low"
                );
            }
            DspRegister::DataHigh => {
                if self.read_high_pending {
                    let _ = self.result_words.pop_front();
                    self.read_high_pending = false;
                }
                self.refresh_status();
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "read DSP data high"
                );
            }
            DspRegister::Status => {
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    value = value,
                    "read DSP status"
                );
            }
        }

        Some(value)
    }

    fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(register) = self.map.decode(mapper, address) else {
            return false;
        };

        match register {
            DspRegister::DataLow => {
                self.write_latch = (self.write_latch & 0xFF00) | u16::from(value);
                self.low_byte_latched = true;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "latched DSP data low byte"
                );
            }
            DspRegister::DataHigh => {
                self.write_latch = (self.write_latch & 0x00FF) | (u16::from(value) << 8);
                if self.low_byte_latched {
                    self.accept_word(self.write_latch as i16);
                }
                self.low_byte_latched = false;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    word = self.write_latch,
                    "latched DSP data high byte"
                );
            }
            DspRegister::Status => {
                self.status_register = value;
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    address = %format_args!("{address:#08X}"),
                    value = value,
                    "wrote DSP status"
                );
            }
        }

        true
    }

    fn accept_word(&mut self, word: i16) {
        if let Some(command) = self.active_command {
            self.operand_words.push(word);
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                command = ?command,
                operand = word,
                collected = self.operand_words.len(),
                required = command.operand_count(),
                "accepted DSP operand"
            );
            if self.operand_words.len() >= command.operand_count() {
                let operands = std::mem::take(&mut self.operand_words);
                let results = command.execute(&operands);
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    command = ?command,
                    operands = ?operands,
                    results = ?results,
                    "executed DSP command"
                );
                self.result_words = results.into();
                self.active_command = None;
                self.read_high_pending = false;
                self.refresh_status();
            } else {
                self.refresh_status();
            }
            return;
        }

        let command = DspCommand::from_opcode(word as u16);
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            opcode = word as u16,
            command = ?command,
            operand_count = command.operand_count(),
            "accepted DSP command opcode"
        );
        if command.operand_count() == 0 {
            let results = command.execute(&[]);
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                opcode = word as u16,
                command = ?command,
                results = ?results,
                "executed DSP command"
            );
            self.result_words = results.into();
        } else {
            self.active_command = Some(command);
            self.operand_words.clear();
            self.result_words.clear();
        }
        self.read_high_pending = false;
        self.refresh_status();
    }

    fn refresh_read_latch(&mut self) {
        self.read_latch = self.result_words.front().copied().unwrap_or_default() as u16;
    }

    fn refresh_status(&mut self) {
        let previous = self.status_register;
        let mut status = DSP_STATUS_READY;
        if !self.result_words.is_empty() {
            status |= DSP_STATUS_DATA_AVAILABLE;
        }
        if self.active_command.is_some() {
            status |= DSP_STATUS_COMMAND_WAITING;
        }
        self.status_register = status;
        if previous != status {
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                previous = previous,
                current = status,
                has_result = !self.result_words.is_empty(),
                waiting_for_operand = self.active_command.is_some(),
                "updated DSP status"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspCommand {
    Reset,
    Signature,
    Add,
    Subtract,
    Multiply,
    Abs,
    Dot2,
    Unknown(u16),
}

impl DspCommand {
    fn from_opcode(opcode: u16) -> Self {
        match opcode {
            0x0000 => Self::Reset,
            0x0001 => Self::Signature,
            0x0010 => Self::Add,
            0x0011 => Self::Subtract,
            0x0012 => Self::Multiply,
            0x0013 => Self::Abs,
            0x0014 => Self::Dot2,
            value => Self::Unknown(value),
        }
    }

    const fn operand_count(self) -> usize {
        match self {
            Self::Reset | Self::Signature | Self::Unknown(_) => 0,
            Self::Abs => 1,
            Self::Add | Self::Subtract | Self::Multiply => 2,
            Self::Dot2 => 4,
        }
    }

    fn execute(self, operands: &[i16]) -> Vec<i16> {
        match self {
            Self::Reset => vec![0],
            Self::Signature => vec![i16::MIN],
            Self::Add => vec![operands[0].saturating_add(operands[1])],
            Self::Subtract => vec![operands[0].saturating_sub(operands[1])],
            Self::Abs => vec![operands[0].saturating_abs()],
            Self::Multiply => {
                let product = i32::from(operands[0]) * i32::from(operands[1]);
                let bytes = product.to_le_bytes();
                vec![
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                ]
            }
            Self::Dot2 => {
                let left = i32::from(operands[0]) * i32::from(operands[2]);
                let right = i32::from(operands[1]) * i32::from(operands[3]);
                let sum = left.saturating_add(right);
                let bytes = sum.to_le_bytes();
                vec![
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                ]
            }
            Self::Unknown(opcode) => vec![opcode as i16],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspAddressMap {
    LoRom1MiB,
    LoRom2MiB,
    HiRom,
}

impl DspAddressMap {
    fn for_header(header: &CartridgeHeader) -> Self {
        match header.mapper {
            Mapper::HiRom => Self::HiRom,
            Mapper::LoRom
                if header.rom_size_bytes() > (1024 * 1024) || header.ram_size_bytes() > 0 =>
            {
                Self::LoRom2MiB
            }
            Mapper::LoRom => Self::LoRom1MiB,
        }
    }

    fn decode(self, mapper: Mapper, address: Address) -> Option<DspRegister> {
        if mapper == Mapper::HiRom {
            return self.decode_hirom(address);
        }

        match self {
            Self::LoRom1MiB => self.decode_lorom_1mib(address),
            Self::LoRom2MiB => self.decode_lorom_2mib(address),
            Self::HiRom => self.decode_hirom(address),
        }
    }

    fn decode_lorom_1mib(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !(0x30..=0x3F).contains(&bank) {
            return None;
        }

        match offset {
            0x8000..=0xBFFF => Some(byte_register(offset)),
            0xC000..=0xFFFF => Some(DspRegister::Status),
            _ => None,
        }
    }

    fn decode_lorom_2mib(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !(0x60..=0x6F).contains(&bank) {
            return None;
        }

        match offset {
            0x0000..=0x3FFF => Some(byte_register(offset)),
            0x4000..=0x7FFF => Some(DspRegister::Status),
            _ => None,
        }
    }

    fn decode_hirom(self, address: Address) -> Option<DspRegister> {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;
        let bank = bank & 0x7F;
        if !((0x00..=0x1F).contains(&bank) || (0x20..=0x2F).contains(&bank)) {
            return None;
        }

        match offset {
            0x6000..=0x6FFF => Some(byte_register(offset)),
            0x7000..=0x7FFF => Some(DspRegister::Status),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum DspRegister {
    DataLow,
    DataHigh,
    Status,
}

impl std::fmt::Display for DspRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DataLow => f.write_str("data-low"),
            Self::DataHigh => f.write_str("data-high"),
            Self::Status => f.write_str("status"),
        }
    }
}

const fn byte_register(offset: u16) -> DspRegister {
    if offset & 0x0001 == 0 {
        DspRegister::DataLow
    } else {
        DspRegister::DataHigh
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::CartridgeHeader;

    use super::*;

    fn header(
        mapper: Mapper,
        rom_type: u8,
        rom_size_code: u8,
        ram_size_code: u8,
    ) -> CartridgeHeader {
        CartridgeHeader {
            title: "DSP TEST".to_owned(),
            mapper,
            map_mode: match mapper {
                Mapper::LoRom => 0x20,
                Mapper::HiRom => 0x21,
            },
            rom_type,
            rom_size_code,
            ram_size_code,
            destination_code: 0x01,
            region: crate::cartridge::Region::Ntsc,
            complement: 0xFFFF,
            checksum: 0x0000,
        }
    }

    #[test]
    fn detects_dsp_family_from_chipset_byte() {
        assert_eq!(
            CoprocessorKind::detect(&header(Mapper::LoRom, 0x03, 0x09, 0)),
            Some(CoprocessorKind::Dsp)
        );
        assert_eq!(
            CoprocessorKind::detect(&header(Mapper::LoRom, 0x35, 0x09, 0)),
            Some(CoprocessorKind::Sa1)
        );
        assert_eq!(
            CoprocessorKind::detect(&header(Mapper::LoRom, 0x00, 0x09, 0)),
            None
        );
    }

    #[test]
    fn dsp_lorom_1mib_window_roundtrips_data_register() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::LoRom, 0x03, 0x08, 0x00));
        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x34));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x12));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(0x34));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0x12));
        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(DSP_STATUS_READY));
    }

    #[test]
    fn dsp_hirom_window_exposes_status_port() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::HiRom, 0x03, 0x0A, 0x01));
        assert!(dsp.write(Mapper::HiRom, 0x006000, 0x01));
        assert!(dsp.write(Mapper::HiRom, 0x006001, 0x00));
        assert_eq!(dsp.read(Mapper::HiRom, 0x006000), Some(0x00));
        assert_eq!(dsp.read(Mapper::HiRom, 0x006001), Some(0x80));
        assert_eq!(dsp.read(Mapper::HiRom, 0x007000), Some(DSP_STATUS_READY));
        assert_eq!(dsp.read(Mapper::HiRom, 0x108000), None);
    }

    #[test]
    fn dsp_buffers_operands_before_emitting_result_words() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::LoRom, 0x03, 0x08, 0x00));

        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x10));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert_eq!(
            dsp.read(Mapper::LoRom, 0x30C000),
            Some(DSP_STATUS_READY | DSP_STATUS_COMMAND_WAITING)
        );

        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x05));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert_eq!(
            dsp.read(Mapper::LoRom, 0x30C000),
            Some(DSP_STATUS_READY | DSP_STATUS_COMMAND_WAITING)
        );

        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x07));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert_eq!(
            dsp.read(Mapper::LoRom, 0x30C000),
            Some(DSP_STATUS_READY | DSP_STATUS_DATA_AVAILABLE)
        );
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(12));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0));
        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(DSP_STATUS_READY));
    }

    #[test]
    fn dsp_multiply_and_dot_commands_return_two_word_results() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::LoRom, 0x03, 0x08, 0x00));

        for word in [0x0012_u16, 300, 4] {
            assert!(dsp.write(Mapper::LoRom, 0x308000, (word & 0xFF) as u8));
            assert!(dsp.write(Mapper::LoRom, 0x308001, (word >> 8) as u8));
        }
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(0xB0));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0x04));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(0x00));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0x00));

        for word in [0x0014_u16, 3, 4, 5, 6] {
            assert!(dsp.write(Mapper::LoRom, 0x308000, (word & 0xFF) as u8));
            assert!(dsp.write(Mapper::LoRom, 0x308001, (word >> 8) as u8));
        }
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(39));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(0));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0));
    }
}
