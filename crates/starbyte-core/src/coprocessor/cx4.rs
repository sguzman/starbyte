use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;
use crate::cartridge::{CartridgeHeader, Mapper};

const CX4_STATUS_READY: u8 = 0x80;
const CX4_STATUS_RESULT: u8 = 0x01;
const CX4_COMMAND_OFFSET: usize = 0x1F40;
const CX4_STATUS_OFFSET: usize = 0x1F41;

/// Bounded Cx4 math-command model backed by a register window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cx4Coprocessor {
    ram: Vec<u8>,
    busy_cycles: u64,
    pending_command: Option<u8>,
}

impl Cx4Coprocessor {
    pub(crate) fn new(header: &CartridgeHeader) -> Self {
        trace!(
            target: "starbyte_core::coprocessor::cx4",
            title = %header.title,
            "initializing Cx4 coprocessor"
        );
        let mut ram = vec![0; 0x2000];
        ram[CX4_STATUS_OFFSET] = CX4_STATUS_READY;
        Self {
            ram,
            busy_cycles: 0,
            pending_command: None,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.ram.fill(0);
        self.ram[CX4_STATUS_OFFSET] = CX4_STATUS_READY;
        self.busy_cycles = 0;
        self.pending_command = None;
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        let index = decode_cx4(mapper, address)?;
        Some(self.ram[index])
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(index) = decode_cx4(mapper, address) else {
            return false;
        };

        self.ram[index] = value;
        if index == CX4_COMMAND_OFFSET {
            self.pending_command = Some(value);
            self.busy_cycles = command_latency(value);
            self.ram[CX4_STATUS_OFFSET] = 0;
        }
        true
    }

    pub(crate) fn step(&mut self, clocks: u64) {
        if self.busy_cycles == 0 {
            return;
        }
        self.busy_cycles = self.busy_cycles.saturating_sub(clocks);
        if self.busy_cycles != 0 {
            return;
        }

        if let Some(command) = self.pending_command.take() {
            self.execute(command);
        }
        self.ram[CX4_STATUS_OFFSET] = CX4_STATUS_READY | CX4_STATUS_RESULT;
    }

    fn execute(&mut self, command: u8) {
        match command {
            0x10 => {
                let x = read_i16(&self.ram, 0x0000);
                let y = read_i16(&self.ram, 0x0002);
                let z = read_i16(&self.ram, 0x0004);
                let length = (((i32::from(x) * i32::from(x))
                    + (i32::from(y) * i32::from(y))
                    + (i32::from(z) * i32::from(z))) as f64)
                    .sqrt()
                    .round() as i16;
                write_i16(&mut self.ram, 0x0010, length);
            }
            0x13 => {
                let angle = read_i16(&self.ram, 0x0000);
                let x = read_i16(&self.ram, 0x0002);
                let y = read_i16(&self.ram, 0x0004);
                let sin = dsp_sin(angle);
                let cos = dsp_cos(angle);
                let out_x = q15_mul(x, cos).saturating_sub(q15_mul(y, sin));
                let out_y = q15_mul(x, sin).saturating_add(q15_mul(y, cos));
                write_i16(&mut self.ram, 0x0010, out_x);
                write_i16(&mut self.ram, 0x0012, out_y);
            }
            0x1F => {
                let x = read_i16(&self.ram, 0x0000);
                let y = read_i16(&self.ram, 0x0002);
                let z = read_i16(&self.ram, 0x0004).max(1);
                let focal = read_i16(&self.ram, 0x0006).max(1);
                let screen_x = ((i32::from(x) * i32::from(focal)) / i32::from(z))
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
                    as i16;
                let screen_y = ((i32::from(y) * i32::from(focal)) / i32::from(z))
                    .clamp(i32::from(i16::MIN), i32::from(i16::MAX))
                    as i16;
                write_i16(&mut self.ram, 0x0010, screen_x);
                write_i16(&mut self.ram, 0x0012, screen_y);
                write_i16(&mut self.ram, 0x0014, z);
            }
            _ => self.ram[CX4_STATUS_OFFSET] = CX4_STATUS_READY,
        }
    }
}

fn decode_cx4(mapper: Mapper, address: Address) -> Option<usize> {
    if mapper != Mapper::LoRom {
        return None;
    }
    let bank = ((address >> 16) & 0xFF) as u8 & 0x7F;
    let offset = (address & 0xFFFF) as u16;
    if !(0x00..=0x3F).contains(&bank) || !(0x6000..=0x7FFF).contains(&offset) {
        return None;
    }
    Some(usize::from(offset - 0x6000))
}

fn command_latency(command: u8) -> u64 {
    match command {
        0x10 => 10,
        0x13 | 0x1F => 18,
        _ => 4,
    }
}

fn read_i16(ram: &[u8], index: usize) -> i16 {
    i16::from_le_bytes([ram[index], ram[index + 1]])
}

fn write_i16(ram: &mut [u8], index: usize, value: i16) {
    let [lo, hi] = value.to_le_bytes();
    ram[index] = lo;
    ram[index + 1] = hi;
}

fn q15_mul(left: i16, right: i16) -> i16 {
    ((i32::from(left) * i32::from(right)) >> 15) as i16
}

fn dsp_sin(angle: i16) -> i16 {
    let radians = f64::from(angle) * std::f64::consts::PI / 32768.0;
    (radians.sin() * 32767.0).round().clamp(-32767.0, 32767.0) as i16
}

fn dsp_cos(angle: i16) -> i16 {
    let radians = f64::from(angle) * std::f64::consts::PI / 32768.0;
    (radians.cos() * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{CartridgeHeader, Mapper, Region};

    use super::Cx4Coprocessor;

    fn header() -> CartridgeHeader {
        CartridgeHeader {
            title: "STARBYTE CX4 TEST".to_owned(),
            mapper: Mapper::LoRom,
            map_mode: 0x20,
            rom_type: 0xF3,
            rom_size_code: 0x09,
            ram_size_code: 0x01,
            destination_code: 0x01,
            region: Region::Ntsc,
            complement: 0xFFFF,
            checksum: 0x0000,
        }
    }

    #[test]
    fn vector_length_command_produces_expected_result() {
        let mut cx4 = Cx4Coprocessor::new(&header());
        for (address, value) in [
            (0x006000, 3),
            (0x006001, 0),
            (0x006002, 4),
            (0x006003, 0),
            (0x006004, 12),
            (0x006005, 0),
            (0x007F40, 0x10),
        ] {
            assert!(cx4.write(Mapper::LoRom, address, value));
        }
        cx4.step(10);
        assert_eq!(cx4.read(Mapper::LoRom, 0x006010), Some(13));
        assert_eq!(cx4.read(Mapper::LoRom, 0x006011), Some(0));
        assert_eq!(cx4.read(Mapper::LoRom, 0x007F41), Some(0x81));
    }

    #[test]
    fn rotate_command_updates_output_registers() {
        let mut cx4 = Cx4Coprocessor::new(&header());
        for (address, value) in [
            (0x006000, 0),
            (0x006001, 0x40),
            (0x006002, 0),
            (0x006003, 0x40),
            (0x006004, 0),
            (0x006005, 0),
            (0x007F40, 0x13),
        ] {
            assert!(cx4.write(Mapper::LoRom, address, value));
        }
        cx4.step(18);
        assert_eq!(cx4.read(Mapper::LoRom, 0x006010), Some(0));
        assert_eq!(cx4.read(Mapper::LoRom, 0x006011), Some(0));
        assert!(
            cx4.read(Mapper::LoRom, 0x006013)
                .is_some_and(|value| value > 0x3E)
        );
    }
}
