use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::bus::Address;
use crate::cartridge::Mapper;

/// Bounded S-DD1 register and decompression stream model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sdd1Coprocessor {
    enabled: bool,
    mode: u8,
    input: Vec<u8>,
    output: VecDeque<u8>,
    busy_cycles: u64,
}

impl Sdd1Coprocessor {
    pub(crate) fn new() -> Self {
        Self {
            enabled: false,
            mode: 0,
            input: Vec::new(),
            output: VecDeque::new(),
            busy_cycles: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.enabled = false;
        self.mode = 0;
        self.input.clear();
        self.output.clear();
        self.busy_cycles = 0;
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let offset = decode_sdd1(mapper, address)?;
        match offset {
            0x00 => Some(self.enabled as u8),
            0x01 => Some(self.mode),
            0x05 => Some(self.output.pop_front().unwrap_or(0)),
            0x06 => Some(((self.enabled as u8) << 0) | (u8::from(!self.output.is_empty()) << 7)),
            0x07 => Some(self.output.len().min(0xFF) as u8),
            _ => Some(0),
        }
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(offset) = decode_sdd1(mapper, address) else {
            return false;
        };
        match offset {
            0x00 => self.enabled = value & 0x01 != 0,
            0x01 => self.mode = value,
            0x04 => {
                self.input.push(value);
                self.busy_cycles = self.busy_cycles.saturating_add(2);
            }
            0x07 => {
                self.input.clear();
                self.output.clear();
            }
            _ => {}
        }
        true
    }

    pub(crate) fn step(&mut self, clocks: u64) {
        if !self.enabled || self.busy_cycles == 0 {
            return;
        }
        self.busy_cycles = self.busy_cycles.saturating_sub(clocks);
        if self.busy_cycles == 0 && self.output.is_empty() && !self.input.is_empty() {
            self.output = decompress_stream(self.mode, &self.input)
                .into_iter()
                .collect();
            self.input.clear();
        }
    }
}

/// OBC1 object-window controller backed by 8 KiB of internal RAM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Obc1Coprocessor {
    ram: Vec<u8>,
    baseptr: u16,
    address: u8,
    shift: u8,
}

impl Obc1Coprocessor {
    pub(crate) fn new() -> Self {
        let mut obc1 = Self {
            ram: vec![0; 0x2000],
            baseptr: 0x1C00,
            address: 0,
            shift: 0,
        };
        obc1.reset();
        obc1
    }

    pub(crate) fn reset(&mut self) {
        self.ram.fill(0);
        self.baseptr = 0x1C00;
        self.address = 0;
        self.shift = 0;
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let offset = decode_obc1(mapper, address)?;
        Some(match offset {
            0x1FF0 => {
                self.ram[(usize::from(self.baseptr) + (usize::from(self.address) << 2)) & 0x1FFF]
            }
            0x1FF1 => {
                self.ram
                    [(usize::from(self.baseptr) + (usize::from(self.address) << 2) + 1) & 0x1FFF]
            }
            0x1FF2 => {
                self.ram
                    [(usize::from(self.baseptr) + (usize::from(self.address) << 2) + 2) & 0x1FFF]
            }
            0x1FF3 => {
                self.ram
                    [(usize::from(self.baseptr) + (usize::from(self.address) << 2) + 3) & 0x1FFF]
            }
            0x1FF4 => {
                self.ram
                    [(usize::from(self.baseptr) + usize::from(self.address >> 2) + 0x200) & 0x1FFF]
            }
            _ => self.ram[usize::from(offset) & 0x1FFF],
        })
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(offset) = decode_obc1(mapper, address) else {
            return false;
        };
        match offset {
            0x1FF0..=0x1FF3 => {
                let slot = usize::from(offset - 0x1FF0);
                let index =
                    (usize::from(self.baseptr) + (usize::from(self.address) << 2) + slot) & 0x1FFF;
                self.ram[index] = value;
            }
            0x1FF4 => {
                let index =
                    (usize::from(self.baseptr) + usize::from(self.address >> 2) + 0x200) & 0x1FFF;
                let mask = !(0x03 << self.shift);
                self.ram[index] = (self.ram[index] & mask) | ((value & 0x03) << self.shift);
            }
            0x1FF5 => {
                self.baseptr = if value & 0x01 != 0 { 0x1800 } else { 0x1C00 };
                self.ram[usize::from(offset) & 0x1FFF] = value;
            }
            0x1FF6 => {
                self.address = value & 0x7F;
                self.shift = (value & 0x03) << 1;
                self.ram[usize::from(offset) & 0x1FFF] = value;
            }
            _ => self.ram[usize::from(offset) & 0x1FFF] = value,
        }
        true
    }
}

/// Deterministic S-RTC serial time source for regression coverage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SRtcCoprocessor {
    control: u8,
    index: usize,
    latched: [u8; 13],
    seconds: u32,
}

impl SRtcCoprocessor {
    pub(crate) fn new() -> Self {
        let mut rtc = Self {
            control: 0,
            index: 0,
            latched: [0; 13],
            seconds: 0,
        };
        rtc.reset();
        rtc
    }

    pub(crate) fn reset(&mut self) {
        self.control = 0;
        self.index = 0;
        self.seconds = 0;
        self.latch_time();
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let offset = decode_srtc(mapper, address)?;
        match offset {
            0x00 => {
                let value = self.latched[self.index.min(self.latched.len() - 1)];
                self.index = (self.index + 1).min(self.latched.len() - 1);
                Some(value)
            }
            0x01 => Some(self.control),
            _ => Some(0),
        }
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(offset) = decode_srtc(mapper, address) else {
            return false;
        };
        match offset {
            0x00 => {}
            0x01 => {
                self.control = value;
                match value {
                    0x0D => self.latch_time(),
                    0x0E => {
                        self.seconds = self.seconds.saturating_add(1);
                        self.latch_time();
                    }
                    0x0F => self.index = 0,
                    _ => {}
                }
            }
            _ => {}
        }
        true
    }

    fn latch_time(&mut self) {
        let seconds = self.seconds % 60;
        let minutes = (self.seconds / 60) % 60;
        let hours = (self.seconds / 3600) % 24;
        self.latched = [
            digit_ones(seconds),
            digit_tens(seconds),
            digit_ones(minutes),
            digit_tens(minutes),
            digit_ones(hours),
            digit_tens(hours),
            4,
            2,
            0,
            2,
            4,
            6,
            0x0F,
        ];
        self.index = 0;
    }
}

fn decode_sdd1(mapper: Mapper, address: Address) -> Option<u16> {
    if mapper != Mapper::LoRom {
        return None;
    }
    let bank = ((address >> 16) & 0xFF) as u8 & 0x7F;
    let offset = (address & 0xFFFF) as u16;
    if (0x00..=0x3F).contains(&bank) && (0x4800..=0x4807).contains(&offset) {
        return Some(offset - 0x4800);
    }
    None
}

fn decode_obc1(mapper: Mapper, address: Address) -> Option<u16> {
    if mapper != Mapper::LoRom {
        return None;
    }
    let bank = ((address >> 16) & 0xFF) as u8 & 0x7F;
    let offset = (address & 0xFFFF) as u16;
    if (0x00..=0x3F).contains(&bank) && (0x6000..=0x7FFF).contains(&offset) {
        return Some(offset - 0x6000);
    }
    None
}

fn decode_srtc(mapper: Mapper, address: Address) -> Option<u16> {
    if mapper != Mapper::LoRom {
        return None;
    }
    let bank = ((address >> 16) & 0xFF) as u8 & 0x7F;
    let offset = (address & 0xFFFF) as u16;
    if (0x00..=0x3F).contains(&bank) && (0x2800..=0x2801).contains(&offset) {
        return Some(offset - 0x2800);
    }
    None
}

fn decompress_stream(mode: u8, input: &[u8]) -> Vec<u8> {
    if mode & 0x01 == 0 {
        return input.to_vec();
    }

    let mut output = Vec::new();
    let mut index = 0;
    while index < input.len() {
        let token = input[index];
        index += 1;
        if token & 0x80 != 0 {
            let count = usize::from((token & 0x7F) + 1);
            let value = input.get(index).copied().unwrap_or(0);
            index += 1;
            output.extend(std::iter::repeat_n(value, count));
        } else {
            let count = usize::from(token + 1);
            for _ in 0..count {
                if let Some(value) = input.get(index).copied() {
                    output.push(value);
                }
                index += 1;
            }
        }
    }
    output
}

fn digit_ones(value: u32) -> u8 {
    (value % 10) as u8
}

fn digit_tens(value: u32) -> u8 {
    ((value / 10) % 10) as u8
}

#[cfg(test)]
mod tests {
    use crate::cartridge::Mapper;

    use super::{Obc1Coprocessor, SRtcCoprocessor, Sdd1Coprocessor};

    #[test]
    fn sdd1_rle_mode_expands_data() {
        let mut sdd1 = Sdd1Coprocessor::new();
        sdd1.write(Mapper::LoRom, 0x004800, 0x01);
        sdd1.write(Mapper::LoRom, 0x004801, 0x01);
        sdd1.write(Mapper::LoRom, 0x004804, 0x82);
        sdd1.write(Mapper::LoRom, 0x004804, 0x41);
        sdd1.step(4);

        assert_eq!(sdd1.read(Mapper::LoRom, 0x004806), Some(0x81));
        assert_eq!(sdd1.read(Mapper::LoRom, 0x004805), Some(0x41));
        assert_eq!(sdd1.read(Mapper::LoRom, 0x004805), Some(0x41));
        assert_eq!(sdd1.read(Mapper::LoRom, 0x004805), Some(0x41));
    }

    #[test]
    fn obc1_window_tracks_selected_object() {
        let mut obc1 = Obc1Coprocessor::new();
        obc1.write(Mapper::LoRom, 0x007FF6, 0x05);
        obc1.write(Mapper::LoRom, 0x007FF0, 0x12);
        obc1.write(Mapper::LoRom, 0x007FF1, 0x34);
        assert_eq!(obc1.read(Mapper::LoRom, 0x007FF0), Some(0x12));
        assert_eq!(obc1.read(Mapper::LoRom, 0x007FF1), Some(0x34));
    }

    #[test]
    fn srtc_latches_deterministic_time_digits() {
        let mut srtc = SRtcCoprocessor::new();
        srtc.write(Mapper::LoRom, 0x002801, 0x0E);
        srtc.write(Mapper::LoRom, 0x002801, 0x0F);
        assert_eq!(srtc.read(Mapper::LoRom, 0x002800), Some(1));
        assert_eq!(srtc.read(Mapper::LoRom, 0x002800), Some(0));
    }
}
