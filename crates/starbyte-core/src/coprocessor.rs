//! Cartridge coprocessor metadata and bootstrap runtime models.

use serde::{Deserialize, Serialize};

use crate::bus::Address;
use crate::cartridge::{CartridgeHeader, Mapper};

const DSP_STATUS_READY: u8 = 0x80;

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
    data_register: u16,
    command_register: u16,
    status_register: u8,
    low_byte_latched: bool,
}

impl DspCoprocessor {
    fn new(header: &CartridgeHeader) -> Self {
        Self {
            map: DspAddressMap::for_header(header),
            data_register: 0,
            command_register: 0,
            status_register: DSP_STATUS_READY,
            low_byte_latched: false,
        }
    }

    fn reset(&mut self) {
        self.data_register = 0;
        self.command_register = 0;
        self.status_register = DSP_STATUS_READY;
        self.low_byte_latched = false;
    }

    fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let register = self.map.decode(mapper, address)?;
        let value = match register {
            DspRegister::DataLow => self.data_register.to_le_bytes()[0],
            DspRegister::DataHigh => self.data_register.to_le_bytes()[1],
            DspRegister::Status => self.status_register,
        };

        if matches!(register, DspRegister::DataHigh) {
            self.status_register = DSP_STATUS_READY;
            self.low_byte_latched = false;
        }

        Some(value)
    }

    fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(register) = self.map.decode(mapper, address) else {
            return false;
        };

        match register {
            DspRegister::DataLow => {
                self.data_register = (self.data_register & 0xFF00) | u16::from(value);
                self.low_byte_latched = true;
            }
            DspRegister::DataHigh => {
                self.data_register = (self.data_register & 0x00FF) | (u16::from(value) << 8);
                if self.low_byte_latched {
                    self.command_register = self.data_register;
                    self.execute_bootstrap_command();
                }
                self.low_byte_latched = false;
            }
            DspRegister::Status => {
                self.status_register = value;
            }
        }

        true
    }

    fn execute_bootstrap_command(&mut self) {
        self.status_register = DSP_STATUS_READY;
        self.data_register = match self.command_register {
            0x0000 => 0,
            0x0001 => 0x8000,
            value => value,
        };
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
            Mapper::LoRom if header.rom_size_bytes() > (1024 * 1024) || header.ram_size_bytes() > 0 => {
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

    fn header(mapper: Mapper, rom_type: u8, rom_size_code: u8, ram_size_code: u8) -> CartridgeHeader {
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
        assert_eq!(CoprocessorKind::detect(&header(Mapper::LoRom, 0x00, 0x09, 0)), None);
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
}
