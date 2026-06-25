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
    /// Bounded SuperFX register and cache model.
    SuperFx(SuperFxCoprocessor),
}

impl Coprocessor {
    /// Build the runtime coprocessor state for a cartridge, if any.
    #[must_use]
    pub fn for_cartridge(header: &CartridgeHeader) -> Option<Self> {
        match CoprocessorKind::detect(header)? {
            CoprocessorKind::Dsp => Some(Self::Dsp(DspCoprocessor::new(header))),
            CoprocessorKind::SuperFx => Some(Self::SuperFx(SuperFxCoprocessor::new(header))),
            _ => None,
        }
    }

    /// Return the coprocessor family.
    #[must_use]
    pub const fn kind(&self) -> CoprocessorKind {
        match self {
            Self::Dsp(_) => CoprocessorKind::Dsp,
            Self::SuperFx(_) => CoprocessorKind::SuperFx,
        }
    }

    /// Reset transient runtime state.
    pub fn reset(&mut self) {
        match self {
            Self::Dsp(dsp) => dsp.reset(),
            Self::SuperFx(superfx) => superfx.reset(),
        }
    }

    /// Attempt to service a CPU read from a coprocessor-mapped address.
    #[must_use]
    pub fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        match self {
            Self::Dsp(dsp) => dsp.read(mapper, address),
            Self::SuperFx(superfx) => superfx.read(mapper, address),
        }
    }

    /// Attempt to service a CPU write to a coprocessor-mapped address.
    pub fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        match self {
            Self::Dsp(dsp) => dsp.write(mapper, address, value),
            Self::SuperFx(superfx) => superfx.write(mapper, address, value),
        }
    }

    /// Advance coprocessor-internal timing, if the active chip model uses it.
    pub fn step_master_cycles(&mut self, clocks: u64) {
        match self {
            Self::Dsp(_) => {}
            Self::SuperFx(superfx) => superfx.step(clocks),
        }
    }
}

/// Coarse DSP family revision derived from the cartridge metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DspVariant {
    /// DSP-1 baseline behavior.
    Dsp1,
    /// DSP-1B with the alternate data ROM observed in later boards.
    Dsp1B,
    /// DSP-2 family.
    Dsp2,
    /// DSP-3 family.
    Dsp3,
    /// DSP-4 family.
    Dsp4,
    /// We detected a DSP cartridge but could not refine the revision.
    Unknown,
}

impl DspVariant {
    /// Detect a likely DSP revision from the cartridge title and chipset byte.
    #[must_use]
    pub fn detect(header: &CartridgeHeader) -> Self {
        let title = header.title.to_ascii_uppercase();
        if title.contains("DSP-4") {
            return Self::Dsp4;
        }
        if title.contains("DSP-3") {
            return Self::Dsp3;
        }
        if title.contains("DSP-2") {
            return Self::Dsp2;
        }
        if title.contains("DSP-1B") || title.contains("DSP1B") || title.contains("1B") {
            return Self::Dsp1B;
        }
        if header.rom_type >= 0x03 {
            return Self::Dsp1;
        }
        Self::Unknown
    }
}

impl std::fmt::Display for DspVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp1 => f.write_str("DSP-1"),
            Self::Dsp1B => f.write_str("DSP-1B"),
            Self::Dsp2 => f.write_str("DSP-2"),
            Self::Dsp3 => f.write_str("DSP-3"),
            Self::Dsp4 => f.write_str("DSP-4"),
            Self::Unknown => f.write_str("unknown DSP"),
        }
    }
}

/// Bootstrap DSP-family runtime model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DspCoprocessor {
    variant: DspVariant,
    map: DspAddressMap,
    status_register: u8,
    write_latch: u16,
    read_latch: u16,
    low_byte_latched: bool,
    read_high_pending: bool,
    active_command: Option<DspCommand>,
    frozen: bool,
    operand_words: Vec<i16>,
    result_words: VecDeque<i16>,
}

impl DspCoprocessor {
    fn new(header: &CartridgeHeader) -> Self {
        let variant = DspVariant::detect(header);
        let map = DspAddressMap::for_header(header);
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            title = %header.title,
            mapper = ?header.mapper,
            variant = %variant,
            map = ?map,
            rom_type = header.rom_type,
            "initializing DSP coprocessor"
        );
        Self {
            variant,
            map,
            status_register: DSP_STATUS_READY,
            write_latch: 0,
            read_latch: 0,
            low_byte_latched: false,
            read_high_pending: false,
            active_command: None,
            frozen: false,
            operand_words: Vec::new(),
            result_words: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        trace!(
            target: "starbyte_core::coprocessor::dsp",
            variant = %self.variant,
            "resetting DSP coprocessor"
        );
        self.status_register = DSP_STATUS_READY;
        self.write_latch = 0;
        self.read_latch = 0;
        self.low_byte_latched = false;
        self.read_high_pending = false;
        self.active_command = None;
        self.frozen = false;
        self.operand_words.clear();
        self.result_words.clear();
    }

    fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let register = self.map.decode(mapper, address)?;
        if self.frozen && !matches!(register, DspRegister::Status) {
            return None;
        }
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

        if self.frozen && !matches!(register, DspRegister::Status) {
            return true;
        }

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
                let results = command.execute(&operands, self.variant);
                trace!(
                    target: "starbyte_core::coprocessor::dsp",
                    command = ?command,
                    variant = %self.variant,
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
            variant = %self.variant,
            operand_count = command.operand_count(),
            "accepted DSP command opcode"
        );
        if command.operand_count() == 0 {
            let results = command.execute(&[], self.variant);
            trace!(
                target: "starbyte_core::coprocessor::dsp",
                opcode = word as u16,
                command = ?command,
                variant = %self.variant,
                results = ?results,
                "executed DSP command"
            );
            self.result_words = results.into();
            if command.is_freeze() {
                self.frozen = true;
            }
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
        let mut status = if self.frozen { 0 } else { DSP_STATUS_READY };
        if !self.frozen {
            if !self.result_words.is_empty() {
                status |= DSP_STATUS_DATA_AVAILABLE;
            }
            if self.active_command.is_some() {
                status |= DSP_STATUS_COMMAND_WAITING;
            }
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
    MemoryTest,
    Multiply2,
    Add,
    Subtract,
    Multiply,
    Abs,
    Dot2,
    MemoryDump,
    MemorySize,
    Freeze,
    Unknown(u16),
}

impl DspCommand {
    fn from_opcode(opcode: u16) -> Self {
        match opcode {
            0x0000 => Self::Reset,
            0x0001 => Self::Signature,
            0x000F => Self::MemoryTest,
            0x001A | 0x002A | 0x003A => Self::Freeze,
            0x0010 => Self::Add,
            0x0011 => Self::Subtract,
            0x0012 => Self::Multiply,
            0x0013 => Self::Abs,
            0x0014 => Self::Dot2,
            0x001F => Self::MemoryDump,
            0x0020 => Self::Multiply2,
            0x002F => Self::MemorySize,
            value => Self::Unknown(value),
        }
    }

    const fn operand_count(self) -> usize {
        match self {
            Self::Reset | Self::Signature | Self::MemorySize | Self::Freeze | Self::Unknown(_) => 0,
            Self::MemoryTest => 1,
            Self::MemoryDump => 1,
            Self::Abs => 1,
            Self::Add | Self::Subtract | Self::Multiply | Self::Multiply2 => 2,
            Self::Dot2 => 4,
        }
    }

    const fn is_freeze(self) -> bool {
        matches!(self, Self::Freeze)
    }

    fn execute(self, operands: &[i16], variant: DspVariant) -> Vec<i16> {
        match self {
            Self::Reset => vec![0],
            Self::Signature => vec![match variant {
                DspVariant::Dsp1B => i16::MIN + 1,
                DspVariant::Dsp2 => i16::MIN + 2,
                DspVariant::Dsp3 => i16::MIN + 3,
                DspVariant::Dsp4 => i16::MIN + 4,
                DspVariant::Unknown => i16::MIN,
                DspVariant::Dsp1 => i16::MIN,
            }],
            Self::MemoryTest => vec![0],
            Self::MemorySize => vec![0x0100],
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
            Self::Multiply2 => {
                let product = i32::from(operands[0]) * i32::from(operands[1]);
                let adjusted = product.saturating_add(1);
                let bytes = adjusted.to_le_bytes();
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
            Self::MemoryDump => {
                let seed: u16 = match variant {
                    DspVariant::Dsp1B => 0x1B1B,
                    DspVariant::Dsp2 => 0x2D2D,
                    DspVariant::Dsp3 => 0x3D3D,
                    DspVariant::Dsp4 => 0x4D4D,
                    DspVariant::Unknown | DspVariant::Dsp1 => 0x1111,
                };
                let seed = seed.wrapping_add(operands.first().copied().unwrap_or_default() as u16);
                (0..1024)
                    .map(|index| {
                        let value = seed.wrapping_add(index as u16);
                        i16::from_le_bytes(value.to_le_bytes())
                    })
                    .collect()
            }
            Self::Freeze => Vec::new(),
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

/// Minimal SuperFX address-map classification used for register routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SuperFxMap {
    /// SuperFX-1 style board layout.
    SuperFx1,
    /// SuperFX-2 style board layout.
    SuperFx2,
}

impl SuperFxMap {
    fn for_header(header: &CartridgeHeader) -> Self {
        if header.mapper == Mapper::HiRom || header.rom_size_bytes() > (2 * 1024 * 1024) {
            Self::SuperFx2
        } else {
            Self::SuperFx1
        }
    }
}

impl std::fmt::Display for SuperFxMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SuperFx1 => f.write_str("SuperFX-1"),
            Self::SuperFx2 => f.write_str("SuperFX-2"),
        }
    }
}

/// Bounded SuperFX register and cache model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuperFxCoprocessor {
    map: SuperFxMap,
    regs: [u16; 16],
    sfr: u16,
    bramr: u8,
    pbr: u8,
    rombr: u8,
    cfgr: u8,
    scbr: u8,
    clsr: u8,
    scmr: u8,
    vcr: u8,
    rambr: u8,
    cbr: u16,
    cache: Vec<u8>,
    cache_valid: Vec<bool>,
    running: bool,
    cycles: u64,
}

impl SuperFxCoprocessor {
    fn new(header: &CartridgeHeader) -> Self {
        let map = SuperFxMap::for_header(header);
        trace!(
            target: "starbyte_core::coprocessor::superfx",
            title = %header.title,
            mapper = ?header.mapper,
            map = %map,
            rom_type = header.rom_type,
            "initializing SuperFX coprocessor"
        );
        Self {
            map,
            regs: [0; 16],
            sfr: 0,
            bramr: 0,
            pbr: 0,
            rombr: 0,
            cfgr: 0,
            scbr: 0,
            clsr: 0,
            scmr: 0,
            vcr: 0,
            rambr: 0,
            cbr: 0,
            cache: vec![0xFF; 0x200],
            cache_valid: vec![false; 0x20],
            running: false,
            cycles: 0,
        }
    }

    fn reset(&mut self) {
        trace!(
            target: "starbyte_core::coprocessor::superfx",
            map = %self.map,
            "resetting SuperFX coprocessor"
        );
        self.regs = [0; 16];
        self.sfr = 0;
        self.bramr = 0;
        self.pbr = 0;
        self.rombr = 0;
        self.cfgr = 0;
        self.scbr = 0;
        self.clsr = 0;
        self.scmr = 0;
        self.vcr = 0;
        self.rambr = 0;
        self.cbr = 0;
        self.cache.fill(0xFF);
        self.cache_valid.fill(false);
        self.running = false;
        self.cycles = 0;
    }

    fn step(&mut self, clocks: u64) {
        self.cycles = self.cycles.saturating_add(clocks);
        if self.running {
            trace!(
                target: "starbyte_core::coprocessor::superfx",
                cycles = self.cycles,
                "advanced SuperFX runtime"
            );
        }
    }

    fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        self.decode(mapper, address).map(|register| match register {
            SuperFxRegister::R(index, half) => {
                let value = self.regs[index];
                let byte = if half {
                    (value >> 8) as u8
                } else {
                    value as u8
                };
                trace!(
                    target: "starbyte_core::coprocessor::superfx",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = byte,
                    "read SuperFX register"
                );
                byte
            }
            SuperFxRegister::SfrLow => (self.sfr & 0x00FF) as u8,
            SuperFxRegister::SfrHigh => {
                let value = (self.sfr >> 8) as u8;
                trace!(
                    target: "starbyte_core::coprocessor::superfx",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "read SuperFX status"
                );
                value
            }
            SuperFxRegister::Bramr => self.bramr,
            SuperFxRegister::Pbr => self.pbr,
            SuperFxRegister::Rombr => self.rombr,
            SuperFxRegister::Cfgr => self.cfgr,
            SuperFxRegister::Scbr => self.scbr,
            SuperFxRegister::Clsr => self.clsr,
            SuperFxRegister::Scmr => self.scmr,
            SuperFxRegister::Vcr => self.vcr,
            SuperFxRegister::Rambr => self.rambr,
            SuperFxRegister::CbrLow => (self.cbr & 0x00FF) as u8,
            SuperFxRegister::CbrHigh => (self.cbr >> 8) as u8,
            SuperFxRegister::Cache(offset) => self.cache[offset],
        })
    }

    fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(register) = self.decode(mapper, address) else {
            return false;
        };

        match register {
            SuperFxRegister::R(index, half) => {
                let current = self.regs[index];
                self.regs[index] = if half {
                    (current & 0x00FF) | (u16::from(value) << 8)
                } else {
                    (current & 0xFF00) | u16::from(value)
                };
                if index == 14 && half {
                    self.rombr = (self.regs[index] & 0x7F) as u8;
                }
                if index == 15 && half {
                    self.running = true;
                }
                trace!(
                    target: "starbyte_core::coprocessor::superfx",
                    address = %format_args!("{address:#08X}"),
                    register = %register,
                    value = value,
                    "wrote SuperFX register"
                );
            }
            SuperFxRegister::SfrLow => {
                let prior = self.sfr;
                self.sfr = (self.sfr & 0xFF00) | u16::from(value);
                if (prior & 0x0001) != 0 && (self.sfr & 0x0001) == 0 {
                    self.cbr = 0;
                    self.flush_cache();
                }
            }
            SuperFxRegister::SfrHigh => {
                self.sfr = (u16::from(value) << 8) | (self.sfr & 0x00FF);
            }
            SuperFxRegister::Bramr => self.bramr = value & 0x01,
            SuperFxRegister::Pbr => {
                self.pbr = value & 0x7F;
                self.flush_cache();
            }
            SuperFxRegister::Rombr => self.rombr = value & 0x7F,
            SuperFxRegister::Cfgr => self.cfgr = value,
            SuperFxRegister::Scbr => self.scbr = value,
            SuperFxRegister::Clsr => self.clsr = value & 0x01,
            SuperFxRegister::Scmr => self.scmr = value,
            SuperFxRegister::Vcr => self.vcr = value,
            SuperFxRegister::Rambr => self.rambr = value & 0x01,
            SuperFxRegister::CbrLow => self.cbr = (self.cbr & 0xFF00) | u16::from(value),
            SuperFxRegister::CbrHigh => self.cbr = (u16::from(value) << 8) | (self.cbr & 0x00FF),
            SuperFxRegister::Cache(offset) => self.cache[offset] = value,
        }

        true
    }

    fn flush_cache(&mut self) {
        self.cache_valid.fill(false);
    }

    fn decode(&self, mapper: Mapper, address: Address) -> Option<SuperFxRegister> {
        let _ = mapper;
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        match offset {
            0x3000..=0x301F => {
                let index = usize::from((offset & 0x1E) >> 1);
                Some(SuperFxRegister::R(index, offset & 1 == 1))
            }
            0x3030 => Some(SuperFxRegister::SfrLow),
            0x3031 => Some(SuperFxRegister::SfrHigh),
            0x3033 => Some(SuperFxRegister::Bramr),
            0x3034 => Some(SuperFxRegister::Pbr),
            0x3036 => Some(SuperFxRegister::Rombr),
            0x3037 => Some(SuperFxRegister::Cfgr),
            0x3038 => Some(SuperFxRegister::Scbr),
            0x3039 => Some(SuperFxRegister::Clsr),
            0x303A => Some(SuperFxRegister::Scmr),
            0x303B => Some(SuperFxRegister::Vcr),
            0x303C => Some(SuperFxRegister::Rambr),
            0x303E => Some(SuperFxRegister::CbrLow),
            0x303F => Some(SuperFxRegister::CbrHigh),
            0x3100..=0x32FF => {
                let offset = usize::from(offset - 0x3100);
                Some(SuperFxRegister::Cache(offset))
            }
            _ => match self.map {
                SuperFxMap::SuperFx1 if bank <= 0x7F => None,
                SuperFxMap::SuperFx2 if bank <= 0x7F => None,
                _ => None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SuperFxRegister {
    R(usize, bool),
    SfrLow,
    SfrHigh,
    Bramr,
    Pbr,
    Rombr,
    Cfgr,
    Scbr,
    Clsr,
    Scmr,
    Vcr,
    Rambr,
    CbrLow,
    CbrHigh,
    Cache(usize),
}

impl std::fmt::Display for SuperFxRegister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::R(index, half) => write!(f, "r{index}{}", if *half { ".hi" } else { ".lo" }),
            Self::SfrLow => f.write_str("sfr.lo"),
            Self::SfrHigh => f.write_str("sfr.hi"),
            Self::Bramr => f.write_str("bramr"),
            Self::Pbr => f.write_str("pbr"),
            Self::Rombr => f.write_str("rombr"),
            Self::Cfgr => f.write_str("cfgr"),
            Self::Scbr => f.write_str("scbr"),
            Self::Clsr => f.write_str("clsr"),
            Self::Scmr => f.write_str("scmr"),
            Self::Vcr => f.write_str("vcr"),
            Self::Rambr => f.write_str("rambr"),
            Self::CbrLow => f.write_str("cbr.lo"),
            Self::CbrHigh => f.write_str("cbr.hi"),
            Self::Cache(offset) => write!(f, "cache[{offset:#04X}]"),
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

    #[test]
    fn dsp_variant_detection_prefers_specific_titles() {
        let mut dsp1b = header(Mapper::LoRom, 0x03, 0x08, 0x00);
        dsp1b.title = "STARBYTE DSP-1B TEST".to_owned();
        assert_eq!(DspVariant::detect(&dsp1b), DspVariant::Dsp1B);

        let mut dsp4 = header(Mapper::LoRom, 0x03, 0x08, 0x00);
        dsp4.title = "STARBYTE DSP-4 TEST".to_owned();
        assert_eq!(DspVariant::detect(&dsp4), DspVariant::Dsp4);
    }

    #[test]
    fn dsp_memory_dump_returns_a_large_result_set() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::LoRom, 0x03, 0x08, 0x00));
        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x1f));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x34));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x12));

        assert_eq!(dsp.result_words.len(), 1024);
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), Some(0x45));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308001), Some(0x23));
    }

    #[test]
    fn coprocessor_factory_routes_superfx_carts() {
        let header = header(Mapper::LoRom, 0x13, 0x09, 0x00);
        let coprocessor = Coprocessor::for_cartridge(&header).unwrap();
        assert_eq!(coprocessor.kind(), CoprocessorKind::SuperFx);
    }

    #[test]
    fn dsp_freeze_command_blocks_future_bus_accesses() {
        let mut dsp = DspCoprocessor::new(&header(Mapper::LoRom, 0x03, 0x08, 0x00));
        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x1a));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(0x00));
        assert_eq!(dsp.read(Mapper::LoRom, 0x308000), None);
        assert!(dsp.write(Mapper::LoRom, 0x308000, 0x10));
        assert!(dsp.write(Mapper::LoRom, 0x308001, 0x00));
        assert_eq!(dsp.read(Mapper::LoRom, 0x30C000), Some(0x00));
    }

    #[test]
    fn superfx_cache_window_roundtrips_bytes() {
        let mut superfx = SuperFxCoprocessor::new(&header(Mapper::LoRom, 0x13, 0x09, 0x00));
        assert!(superfx.write(Mapper::LoRom, 0x003100, 0xAA));
        assert!(superfx.write(Mapper::LoRom, 0x003101, 0x55));
        assert_eq!(superfx.read(Mapper::LoRom, 0x003100), Some(0xAA));
        assert_eq!(superfx.read(Mapper::LoRom, 0x003101), Some(0x55));
        superfx.step(12);
        assert_eq!(superfx.read(Mapper::LoRom, 0x003030), Some(0x00));
    }
}
