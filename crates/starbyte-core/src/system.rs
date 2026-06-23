//! Bootstrap SNES system bus, WRAM, and timing-visible MMIO model.

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::{Address, Bus};
use crate::cartridge::Cartridge;
use crate::dma::DmaController;
use crate::input::ControllerState;
use crate::timing::TimingState;

const WRAM_SIZE: usize = 128 * 1024;
const LOW_WRAM_MIRROR_SIZE: usize = 0x2000;
const PPU_REGISTER_COUNT: usize = 0x40;
const APU_IO_PORT_COUNT: usize = 4;

/// CPU-visible system bus state needed for bootstrap correctness work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBus {
    cartridge: Option<Cartridge>,
    wram: Vec<u8>,
    ppu_registers: Vec<u8>,
    apu_io: Vec<u8>,
    dma: DmaController,
    timing: TimingState,
    open_bus: u8,
    nmitimen: u8,
    rdnmi: bool,
    timeup: bool,
    joypad: JoypadIo,
    wram_address: u32,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self {
            cartridge: None,
            wram: vec![0; WRAM_SIZE],
            ppu_registers: vec![0; PPU_REGISTER_COUNT],
            apu_io: vec![0; APU_IO_PORT_COUNT],
            dma: DmaController::default(),
            timing: TimingState::default(),
            open_bus: 0,
            nmitimen: 0,
            rdnmi: false,
            timeup: false,
            joypad: JoypadIo::default(),
            wram_address: 0,
        }
    }
}

impl SystemBus {
    /// Install or replace the loaded cartridge.
    pub fn install_cartridge(&mut self, cartridge: Cartridge) {
        self.cartridge = Some(cartridge);
    }

    /// Clear transient system state while keeping the current cartridge installed.
    pub fn reset(&mut self) {
        self.wram.fill(0);
        self.ppu_registers.fill(0);
        self.apu_io.fill(0);
        self.dma = DmaController::default();
        self.timing = TimingState::default();
        self.open_bus = 0;
        self.nmitimen = 0;
        self.rdnmi = false;
        self.timeup = false;
        self.joypad = JoypadIo::default();
        self.wram_address = 0;
    }

    /// Advance global timing and derive pending interrupt state from it.
    pub fn advance_master_clocks(&mut self, clocks: u64) {
        let events = self.timing.advance_master_clocks(clocks);
        if events.entered_vblank {
            trace!(
                frame = self.timing.frame,
                scanline = self.timing.scanline,
                "entered vblank"
            );
            self.rdnmi = true;
        }
        if events.crossed_scanline && self.irq_enabled() {
            self.timeup = true;
        }
    }

    /// Current global timing state.
    #[must_use]
    pub const fn timing(&self) -> &TimingState {
        &self.timing
    }

    /// Return the installed cartridge if present.
    #[must_use]
    pub const fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    /// Save-RAM byte count advertised by the installed cartridge, if any.
    #[must_use]
    pub fn save_ram_len(&self) -> Option<usize> {
        self.cartridge
            .as_ref()
            .map(|cart| cart.header().ram_size_bytes())
    }

    /// Reset vector advertised by the installed cartridge, if any.
    #[must_use]
    pub fn reset_vector(&self) -> Option<u16> {
        self.cartridge.as_ref().and_then(Cartridge::reset_vector)
    }

    /// Update the current controller-1 snapshot used by joypad latching.
    pub fn set_controller1(&mut self, state: ControllerState) {
        self.joypad.controller1 = state;
        if self.joypad.latch_line {
            self.joypad.latch();
        }
    }

    /// Whether NMI should currently be considered enabled.
    #[must_use]
    pub const fn nmi_enabled(&self) -> bool {
        self.nmitimen & 0x80 != 0
    }

    /// Whether one of the placeholder IRQ enable bits is set.
    #[must_use]
    pub const fn irq_enabled(&self) -> bool {
        self.nmitimen & 0x30 != 0
    }
}

impl Bus for SystemBus {
    fn read(&mut self, address: Address) -> u8 {
        let address = address & 0x00FF_FFFF;
        let value = if let Some(index) = low_wram_mirror_index(address) {
            self.wram[index]
        } else if let Some(index) = high_wram_index(address) {
            self.wram[index]
        } else if let Some(value) = self.read_mmio(address) {
            value
        } else if let Some(value) = self
            .cartridge
            .as_ref()
            .and_then(|cartridge| cartridge.read_byte(address))
        {
            value
        } else {
            self.open_bus
        };

        self.open_bus = value;
        value
    }

    fn write(&mut self, address: Address, value: u8) {
        let address = address & 0x00FF_FFFF;
        self.open_bus = value;

        if let Some(index) = low_wram_mirror_index(address) {
            self.wram[index] = value;
            return;
        }

        if let Some(index) = high_wram_index(address) {
            self.wram[index] = value;
            return;
        }

        let _ = self.write_mmio(address, value);
    }
}

impl SystemBus {
    fn read_mmio(&mut self, address: Address) -> Option<u8> {
        let register = (address & 0xFFFF) as u16;

        match register {
            0x2100..=0x213F => {
                let value = self.ppu_registers[usize::from(register - 0x2100)];
                Some(value)
            }
            0x2140..=0x2143 => Some(self.apu_io[usize::from(register - 0x2140)]),
            0x2180 => {
                let value = self.wram[self.wram_address as usize % WRAM_SIZE];
                self.wram_address = (self.wram_address + 1) & 0x1_FFFF;
                Some(value)
            }
            0x4016 => Some((self.open_bus & 0xFC) | self.joypad.read_serial_port1()),
            0x4017 => Some(self.open_bus & 0xFC),
            0x4210 => {
                let value = (self.open_bus & 0x70) | if self.rdnmi { 0x80 } else { 0x00 } | 0x02;
                self.rdnmi = false;
                Some(value)
            }
            0x4211 => {
                let value = (self.open_bus & 0x7F) | if self.timeup { 0x80 } else { 0x00 };
                self.timeup = false;
                Some(value)
            }
            0x4212 => {
                let mut value = self.open_bus & 0x3E;
                if self.timing.in_vblank() {
                    value |= 0x80;
                }
                if self.timing.dot == 0 {
                    value |= 0x40;
                }
                Some(value)
            }
            0x4218 => Some((self.joypad.latched1 & 0x00FF) as u8),
            0x4219 => Some((self.joypad.latched1 >> 8) as u8),
            0x4300..=0x437F => Some(self.dma.read_register(register - 0x4300)),
            _ => None,
        }
    }

    fn write_mmio(&mut self, address: Address, value: u8) -> Option<()> {
        let register = (address & 0xFFFF) as u16;

        match register {
            0x2100..=0x213F => {
                self.ppu_registers[usize::from(register - 0x2100)] = value;
                Some(())
            }
            0x2140..=0x2143 => {
                self.apu_io[usize::from(register - 0x2140)] = value;
                Some(())
            }
            0x2180 => {
                let index = self.wram_address as usize % WRAM_SIZE;
                self.wram[index] = value;
                self.wram_address = (self.wram_address + 1) & 0x1_FFFF;
                Some(())
            }
            0x2181 => {
                self.wram_address = (self.wram_address & !0x0000FF) | u32::from(value);
                Some(())
            }
            0x2182 => {
                self.wram_address = (self.wram_address & !0x00FF00) | (u32::from(value) << 8);
                Some(())
            }
            0x2183 => {
                self.wram_address =
                    (self.wram_address & !0x010000) | (u32::from(value & 0x01) << 16);
                Some(())
            }
            0x4016 => {
                self.joypad.write_latch(value);
                Some(())
            }
            0x4200 => {
                let nmi_was_enabled = self.nmi_enabled();
                self.nmitimen = value;
                if self.nmi_enabled() && !nmi_was_enabled && self.timing.in_vblank() {
                    self.rdnmi = true;
                }
                Some(())
            }
            0x4300..=0x437F => {
                self.dma.write_register(register - 0x4300, value);
                Some(())
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
struct JoypadIo {
    controller1: ControllerState,
    latch_line: bool,
    latched1: u16,
    shift1: u16,
}

impl JoypadIo {
    fn latch(&mut self) {
        self.latched1 = self.controller1.to_bits();
        self.shift1 = self.latched1;
    }

    fn write_latch(&mut self, value: u8) {
        let next = value & 0x01 != 0;
        if next {
            self.latch();
        } else if self.latch_line && !next {
            self.shift1 = self.latched1;
        }
        self.latch_line = next;
    }

    fn read_serial_port1(&mut self) -> u8 {
        if self.latch_line {
            return (self.latched1 & 0x01) as u8;
        }

        let bit = (self.shift1 & 0x01) as u8;
        self.shift1 = (self.shift1 >> 1) | 0x8000;
        bit
    }
}

fn low_wram_mirror_index(address: Address) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    match bank {
        0x00..=0x3F | 0x80..=0xBF if offset < LOW_WRAM_MIRROR_SIZE as u16 => {
            Some(usize::from(offset))
        }
        _ => None,
    }
}

fn high_wram_index(address: Address) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as usize;

    match bank {
        0x7E => Some(offset),
        0x7F => Some(0x10000 + offset),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{Cartridge, Mapper};
    use crate::timing::{DOTS_PER_SCANLINE, NTSC_SCANLINES_PER_FRAME, VBLANK_START_SCANLINE};

    use super::*;

    fn make_cart(mapper: Mapper) -> Cartridge {
        let mut rom = match mapper {
            Mapper::LoRom => vec![0_u8; 0x10000],
            Mapper::HiRom => vec![0_u8; 0x20000],
        };
        let base = match mapper {
            Mapper::LoRom => 0x7FC0,
            Mapper::HiRom => 0xFFC0,
        };
        rom[base..base + 21].copy_from_slice(b"STARBYTE SYSTEM BUS  ");
        rom[base + 0x15] = match mapper {
            Mapper::LoRom => 0x20,
            Mapper::HiRom => 0x21,
        };
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = if matches!(mapper, Mapper::HiRom) {
            0x0A
        } else {
            0x09
        };
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        Cartridge::from_bytes(rom, None).unwrap()
    }

    #[test]
    fn mirrors_low_wram_into_system_banks() {
        let mut bus = SystemBus::default();
        bus.write(0x000123, 0xAB);

        assert_eq!(bus.read(0x7E0123), 0xAB);
        assert_eq!(bus.read(0x800123), 0xAB);
    }

    #[test]
    fn preserves_open_bus_on_unmapped_reads() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x7E0010, 0x5A);

        assert_eq!(bus.read(0x006000), 0x5A);
    }

    #[test]
    fn latches_joypad_state_into_parallel_and_serial_registers() {
        let mut bus = SystemBus::default();
        bus.set_controller1(ControllerState {
            b: true,
            start: true,
            a: true,
            ..ControllerState::default()
        });

        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        assert_eq!(bus.read(0x004218), 0x09);
        assert_eq!(bus.read(0x004219), 0x01);
        assert_eq!(bus.read(0x004016) & 0x01, 1);
        assert_eq!(bus.read(0x004016) & 0x01, 0);
        assert_eq!(bus.read(0x004016) & 0x01, 0);
        assert_eq!(bus.read(0x004016) & 0x01, 1);
    }

    #[test]
    fn exposes_dma_register_image() {
        let mut bus = SystemBus::default();
        bus.write(0x004300, 0x8F);
        bus.write(0x00430A, 0x55);

        assert_eq!(bus.read(0x004300), 0x8F);
        assert_eq!(bus.read(0x00430A), 0x55);
    }

    #[test]
    fn raises_nmi_and_irq_from_timing_progression() {
        let mut bus = SystemBus::default();
        bus.write(0x004200, 0x90);

        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE));
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        assert_eq!(bus.read(0x004211) & 0x80, 0x00);

        let clocks_to_vblank = u64::from(DOTS_PER_SCANLINE) * u64::from(VBLANK_START_SCANLINE - 1);
        bus.advance_master_clocks(clocks_to_vblank);
        assert!(bus.timing().in_vblank());
        assert_eq!(bus.read(0x004210) & 0x80, 0x80);
        assert_eq!(bus.read(0x004210) & 0x80, 0x00);
    }

    #[test]
    fn wraps_frames_through_explicit_timing() {
        let mut bus = SystemBus::default();
        let frame_clocks = u64::from(DOTS_PER_SCANLINE) * u64::from(NTSC_SCANLINES_PER_FRAME);
        bus.advance_master_clocks(frame_clocks);

        assert_eq!(bus.timing().frame, 1);
        assert_eq!(bus.timing().scanline, 0);
        assert_eq!(bus.timing().dot, 0);
    }
}
