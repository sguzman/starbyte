use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;
use crate::cartridge::{Cartridge, Mapper};
use crate::ppu::FrameBuffer;

const SUPERFX_WIDTH: usize = 256;
const SUPERFX_HEIGHT: usize = 224;
const SFR_IRQ: u16 = 1 << 15;
const SFR_G: u16 = 1 << 5;

/// Minimal SuperFX address-map classification used for register routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuperFxMap {
    /// SuperFX-1 style board layout.
    SuperFx1,
    /// SuperFX-2 style board layout.
    SuperFx2,
}

impl SuperFxMap {
    fn for_cartridge(cartridge: &Cartridge) -> Self {
        if cartridge.header().mapper == Mapper::HiRom || cartridge.header().rom_size_bytes() > (2 * 1024 * 1024) {
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
    rom: Vec<u8>,
    ram: Vec<u8>,
    overlay: Vec<u8>,
    rom_buffer: u8,
    color_register: u8,
    alt1: bool,
    alt2: bool,
}

impl SuperFxCoprocessor {
    pub(crate) fn new(cartridge: &Cartridge) -> Self {
        let map = SuperFxMap::for_cartridge(cartridge);
        trace!(
            target: "starbyte_core::coprocessor::superfx",
            title = %cartridge.header().title,
            mapper = ?cartridge.header().mapper,
            map = %map,
            rom_type = cartridge.header().rom_type,
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
            rom: cartridge.rom().to_vec(),
            ram: vec![0x00; 128 * 1024],
            overlay: vec![0x00; SUPERFX_WIDTH * SUPERFX_HEIGHT * 4],
            rom_buffer: 0,
            color_register: 0,
            alt1: false,
            alt2: false,
        }
    }

    pub(crate) fn reset(&mut self) {
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
        self.ram.fill(0);
        self.overlay.fill(0);
        self.rom_buffer = 0;
        self.color_register = 0;
        self.alt1 = false;
        self.alt2 = false;
    }

    pub(crate) fn step(&mut self, clocks: u64) {
        self.cycles = self.cycles.saturating_add(clocks);
        if self.running {
            let mut remaining = clocks;
            while self.running && remaining > 0 {
                let cost = self.step_instruction();
                if cost == 0 {
                    break;
                }
                if cost > remaining {
                    break;
                }
                remaining -= cost;
            }
            trace!(
                target: "starbyte_core::coprocessor::superfx",
                cycles = self.cycles,
                "advanced SuperFX runtime"
            );
        }
    }

    pub(crate) fn render_overlay(&self, framebuffer: &mut FrameBuffer) {
        let width = framebuffer.width().min(SUPERFX_WIDTH);
        let height = framebuffer.height().min(SUPERFX_HEIGHT);
        let pixels = framebuffer.pixels_mut();
        for y in 0..height {
            for x in 0..width {
                let index = (y * SUPERFX_WIDTH + x) * 4;
                let alpha = self.overlay[index + 3];
                if alpha == 0 {
                    continue;
                }
                pixels[index..index + 4].copy_from_slice(&self.overlay[index..index + 4]);
            }
        }
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        self.decode(mapper, address).map(|register| match register {
            SuperFxRegister::R(index, half) => {
                let value = self.regs[index];
                if half {
                    (value >> 8) as u8
                } else {
                    value as u8
                }
            }
            SuperFxRegister::SfrLow => (self.sfr & 0x00FF) as u8,
            SuperFxRegister::SfrHigh => (self.sfr >> 8) as u8,
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

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
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
                    self.reload_rom_buffer();
                }
                if index == 15 && half {
                    self.running = true;
                    self.sfr |= SFR_G;
                }
            }
            SuperFxRegister::SfrLow => {
                let prior = self.sfr;
                self.sfr = (self.sfr & 0xFF00) | u16::from(value);
                if (prior & 0x0001) != 0 && (self.sfr & 0x0001) == 0 {
                    self.cbr = 0;
                    self.flush_cache();
                }
            }
            SuperFxRegister::SfrHigh => self.sfr = (u16::from(value) << 8) | (self.sfr & 0x00FF),
            SuperFxRegister::Bramr => self.bramr = value & 0x01,
            SuperFxRegister::Pbr => {
                self.pbr = value & 0x7F;
                self.flush_cache();
            }
            SuperFxRegister::Rombr => {
                self.rombr = value & 0x7F;
                self.reload_rom_buffer();
            }
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

    fn step_instruction(&mut self) -> u64 {
        let opcode = self.fetch();
        match opcode {
            0x00 => {
                self.running = false;
                self.sfr &= !SFR_G;
                self.sfr |= SFR_IRQ;
                2
            }
            0x01 => 2,
            0x02 => {
                self.cbr = self.regs[15] & 0xFFF0;
                self.flush_cache();
                2
            }
            0x3D => {
                self.alt1 = true;
                2
            }
            0x3E => {
                self.alt2 = true;
                2
            }
            0x3F => {
                self.alt1 = true;
                self.alt2 = true;
                2
            }
            0x4C => {
                if self.alt1 {
                    let sample = self.read_pixel(self.regs[1] as u8, self.regs[2] as u8);
                    self.regs[0] = u16::from(sample);
                } else {
                    self.plot_pixel(self.regs[1] as u8, self.regs[2] as u8, self.color_register);
                    self.regs[1] = self.regs[1].wrapping_add(1);
                }
                self.clear_prefixes();
                4
            }
            0xDF => {
                match (self.alt1, self.alt2) {
                    (false, false) => self.color_register = self.rom_buffer,
                    (true, true) => {
                        self.rombr = (self.regs[0] & 0x7F) as u8;
                        self.reload_rom_buffer();
                    }
                    (false, true) => self.rambr = (self.regs[0] & 0x01) as u8,
                    (true, false) => self.color_register = (self.rom_buffer >> 4) & 0x0F,
                }
                self.clear_prefixes();
                2
            }
            0xD0..=0xDE => {
                let reg = usize::from(opcode & 0x0F);
                self.regs[reg] = self.regs[reg].wrapping_add(1);
                self.clear_prefixes();
                2
            }
            0xE0..=0xEE => {
                let reg = usize::from(opcode & 0x0F);
                self.regs[reg] = self.regs[reg].wrapping_sub(1);
                self.clear_prefixes();
                2
            }
            0xF0..=0xFF => {
                let reg = usize::from(opcode & 0x0F);
                let lo = self.fetch();
                let hi = self.fetch();
                self.regs[reg] = u16::from_le_bytes([lo, hi]);
                if reg == 14 {
                    self.reload_rom_buffer();
                }
                self.clear_prefixes();
                6
            }
            _ => {
                self.running = false;
                self.sfr &= !SFR_G;
                self.sfr |= SFR_IRQ;
                self.clear_prefixes();
                2
            }
        }
    }

    fn clear_prefixes(&mut self) {
        self.alt1 = false;
        self.alt2 = false;
    }

    fn fetch(&mut self) -> u8 {
        let address = (u32::from(self.pbr) << 16) | u32::from(self.regs[15]);
        self.regs[15] = self.regs[15].wrapping_add(1);
        if let Some(cache_offset) = self.cache_offset(address) {
            if !self.cache_valid[cache_offset / 16] {
                let line_base = cache_offset & !0x0F;
                for offset in 0..16 {
                    self.cache[line_base + offset] = self.read_rom_byte(self.cbr_bank_address(line_base as u16 + offset as u16));
                }
                self.cache_valid[line_base / 16] = true;
            }
            self.cache[cache_offset]
        } else {
            self.read_rom_byte(address)
        }
    }

    fn cbr_bank_address(&self, offset: u16) -> u32 {
        (u32::from(self.pbr) << 16) | u32::from(self.cbr.wrapping_add(offset))
    }

    fn cache_offset(&self, address: u32) -> Option<usize> {
        let cache_base = (u32::from(self.pbr) << 16) | u32::from(self.cbr);
        if (cache_base..cache_base + 0x200).contains(&address) {
            Some((address - cache_base) as usize)
        } else {
            None
        }
    }

    fn reload_rom_buffer(&mut self) {
        let address = (u32::from(self.rombr) << 16) | u32::from(self.regs[14]);
        self.rom_buffer = self.read_rom_byte(address);
    }

    fn read_rom_byte(&self, address: u32) -> u8 {
        if self.rom.is_empty() {
            return 0xFF;
        }
        let bank = ((address >> 16) & 0x7F) as usize;
        let offset = (address & 0xFFFF) as usize;
        let index = match self.map {
            SuperFxMap::SuperFx1 | SuperFxMap::SuperFx2 => match (bank, offset) {
                (0x00..=0x3F, 0x0000..=0x7FFF) => bank * 0x8000 + offset,
                (0x00..=0x3F, 0x8000..=0xFFFF) => bank * 0x8000 + (offset - 0x8000),
                (0x40..=0x5F, _) => (bank - 0x40) * 0x10000 + offset,
                _ => offset,
            },
        };
        self.rom[index % self.rom.len()]
    }

    fn plot_pixel(&mut self, x: u8, y: u8, color: u8) {
        let x = usize::from(x);
        let y = usize::from(y);
        if x >= SUPERFX_WIDTH || y >= SUPERFX_HEIGHT {
            return;
        }

        let index = (y * SUPERFX_WIDTH + x) * 4;
        let [r, g, b] = decode_color(color);
        self.overlay[index] = r;
        self.overlay[index + 1] = g;
        self.overlay[index + 2] = b;
        self.overlay[index + 3] = 0xFF;
    }

    fn read_pixel(&self, x: u8, y: u8) -> u8 {
        let x = usize::from(x);
        let y = usize::from(y);
        if x >= SUPERFX_WIDTH || y >= SUPERFX_HEIGHT {
            return 0;
        }

        let index = (y * SUPERFX_WIDTH + x) * 4;
        let rgb = [self.overlay[index], self.overlay[index + 1], self.overlay[index + 2]];
        encode_color(rgb)
    }

    fn decode(&self, mapper: Mapper, address: Address) -> Option<SuperFxRegister> {
        let _ = mapper;
        let _ = self.rom.len();
        let _ = self.ram.len();
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
            0x3100..=0x32FF => Some(SuperFxRegister::Cache(usize::from(offset - 0x3100))),
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

fn decode_color(color: u8) -> [u8; 3] {
    let r = (color & 0x03) * 85;
    let g = ((color >> 2) & 0x03) * 85;
    let b = ((color >> 4) & 0x03) * 85;
    [r, g, b]
}

fn encode_color(rgb: [u8; 3]) -> u8 {
    ((rgb[2] / 85) << 4) | ((rgb[1] / 85) << 2) | (rgb[0] / 85)
}

#[cfg(test)]
mod tests {
    use crate::cartridge::Cartridge;

    use super::*;

    fn make_superfx_cart(program: &[(usize, u8)]) -> Cartridge {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE SUPERFX     ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x13;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        for (offset, value) in program {
            rom[*offset] = *value;
        }
        Cartridge::from_bytes(rom, None).unwrap()
    }

    #[test]
    fn cache_window_roundtrips_bytes() {
        let cartridge = make_superfx_cart(&[]);
        let mut superfx = SuperFxCoprocessor::new(&cartridge);
        assert!(superfx.write(Mapper::LoRom, 0x003100, 0xAA));
        assert!(superfx.write(Mapper::LoRom, 0x003101, 0x55));
        assert_eq!(superfx.read(Mapper::LoRom, 0x003100), Some(0xAA));
        assert_eq!(superfx.read(Mapper::LoRom, 0x003101), Some(0x55));
    }

    #[test]
    fn can_execute_tiny_plot_program_and_render_overlay() {
        let cartridge = make_superfx_cart(&[
            (0x0000, 0xF1), (0x0001, 0x02), (0x0002, 0x00), // IWT R1,2
            (0x0003, 0xF2), (0x0004, 0x01), (0x0005, 0x00), // IWT R2,1
            (0x0006, 0xFE), (0x0007, 0x20), (0x0008, 0x00), // IWT R14,0x20
            (0x0009, 0xDF), // GETC
            (0x000A, 0x4C), // PLOT
            (0x000B, 0x00), // STOP
            (0x0020, 0x33), // color source
        ]);
        let mut superfx = SuperFxCoprocessor::new(&cartridge);

        assert!(superfx.write(Mapper::LoRom, 0x00301E, 0x00));
        assert!(superfx.write(Mapper::LoRom, 0x00301F, 0x00));
        superfx.step(64);

        let mut frame = FrameBuffer::default();
        superfx.render_overlay(&mut frame);

        let pixel = (SUPERFX_WIDTH + 2) * 4;
        assert_eq!(&frame.pixels()[pixel..pixel + 4], &[255, 0, 255, 255]);
        assert_eq!(superfx.read(Mapper::LoRom, 0x003031), Some((SFR_IRQ >> 8) as u8));
    }
}
