//! Bootstrap SNES system bus, WRAM, and timing-visible MMIO model.

use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::Result;
use crate::bus::{Address, Bus};
use crate::cartridge::Cartridge;
use crate::coprocessor::Coprocessor;
use crate::dma::DmaController;
use crate::input::ControllerState;
use crate::ppu::{FrameBuffer, Ppu};
use crate::timing::TimingState;

const WRAM_SIZE: usize = 128 * 1024;
const LOW_WRAM_MIRROR_SIZE: usize = 0x2000;
const APU_IO_PORT_COUNT: usize = 4;

/// CPU-visible system bus state needed for bootstrap correctness work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBus {
    cartridge: Option<Cartridge>,
    coprocessor: Option<Coprocessor>,
    save_ram: Vec<u8>,
    wram: Vec<u8>,
    ppu: Ppu,
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
            coprocessor: None,
            save_ram: Vec::new(),
            wram: vec![0; WRAM_SIZE],
            ppu: Ppu::default(),
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
        let coprocessor_kind = cartridge.coprocessor_kind();
        debug!(
            title = %cartridge.header().title,
            mapper = ?cartridge.mapper(),
            coprocessor = ?coprocessor_kind,
            "installing cartridge"
        );
        self.save_ram = vec![0; cartridge.header().ram_size_bytes()];
        self.coprocessor = Coprocessor::for_cartridge(&cartridge);
        self.cartridge = Some(cartridge);
    }

    /// Clear transient system state while keeping the current cartridge installed.
    pub fn reset(&mut self) {
        self.wram.fill(0);
        if let Some(coprocessor) = &mut self.coprocessor {
            trace!(coprocessor = ?coprocessor.kind(), "resetting coprocessor runtime");
            coprocessor.reset();
        }
        self.ppu = Ppu::default();
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
        if let Some(coprocessor) = &mut self.coprocessor {
            coprocessor.step_master_cycles(clocks);
        }
        if events.started_new_frame {
            self.initialize_hdma_channels();
        }
        if events.crossed_scanline {
            self.step_hdma_for_scanline();
        }
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

    /// Borrow the bootstrap PPU model.
    #[must_use]
    pub const fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    /// Render the current frame through the PPU model.
    pub fn render_frame(&self, framebuffer: &mut FrameBuffer) {
        self.ppu.render_frame(framebuffer);
        if let Some(coprocessor) = &self.coprocessor {
            coprocessor.render_overlay(framebuffer);
        }
    }

    /// Return the installed cartridge if present.
    #[must_use]
    pub const fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    /// Return the installed coprocessor if the cartridge exposes one.
    #[must_use]
    pub const fn coprocessor(&self) -> Option<&Coprocessor> {
        self.coprocessor.as_ref()
    }

    /// Save-RAM byte count advertised by the installed cartridge, if any.
    #[must_use]
    pub fn save_ram_len(&self) -> Option<usize> {
        self.cartridge
            .as_ref()
            .map(|cart| cart.header().ram_size_bytes())
    }

    /// Replace the current battery-backed RAM image.
    pub fn load_save_ram(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() != self.save_ram.len() {
            return Err(crate::Error::InvalidSaveRam {
                expected: self.save_ram.len(),
                actual: bytes.len(),
            });
        }

        self.save_ram.copy_from_slice(bytes);
        Ok(())
    }

    /// Borrow the current battery-backed RAM image.
    #[must_use]
    pub fn save_ram(&self) -> &[u8] {
        &self.save_ram
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
        } else if let Some(index) = self.save_ram_index(address) {
            self.save_ram[index]
        } else if let Some(value) = self.read_coprocessor(address) {
            value
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

        if let Some(index) = self.save_ram_index(address) {
            self.save_ram[index] = value;
            return;
        }

        if self.write_coprocessor(address, value) {
            return;
        }

        let _ = self.write_mmio(address, value);
    }
}

impl SystemBus {
    fn read_coprocessor(&mut self, address: Address) -> Option<u8> {
        let mapper = self.cartridge.as_ref()?.mapper();
        self.coprocessor.as_mut()?.read(mapper, address)
    }

    fn write_coprocessor(&mut self, address: Address, value: u8) -> bool {
        let Some(cartridge) = self.cartridge.as_ref() else {
            return false;
        };
        self.coprocessor
            .as_mut()
            .is_some_and(|coprocessor| coprocessor.write(cartridge.mapper(), address, value))
    }

    fn save_ram_index(&self, address: Address) -> Option<usize> {
        let len = self.save_ram.len();
        if len == 0 {
            return None;
        }

        match self.cartridge.as_ref()?.mapper() {
            crate::cartridge::Mapper::LoRom => map_lorom_save_ram(address, len),
            crate::cartridge::Mapper::HiRom => map_hirom_save_ram(address, len),
        }
    }

    fn execute_dma(&mut self, mask: u8) {
        for channel_index in 0..8 {
            if mask & (1 << channel_index) == 0 {
                continue;
            }

            let mut channel = self.dma.channel(channel_index).copied().unwrap_or_default();
            let pattern = DmaController::b_bus_offsets_for_mode(channel.transfer_mode());
            let mut a_bus_address = channel.a_bus_address;
            let transfer_length = channel.dma_length();

            for offset_index in 0..transfer_length {
                let a_full = (u32::from(channel.a_bus_bank) << 16) | u32::from(a_bus_address);
                let b_full = 0x002100_u32
                    + u32::from(channel.b_bus_address)
                    + u32::from(pattern[offset_index % pattern.len()]);

                if channel.reverse_transfer() {
                    let value = self.read(b_full);
                    self.write(a_full, value);
                } else {
                    let value = self.read(a_full);
                    self.write(b_full, value);
                }

                if !channel.fixed_transfer() {
                    a_bus_address = if channel.decrement_transfer() {
                        a_bus_address.wrapping_sub(1)
                    } else {
                        a_bus_address.wrapping_add(1)
                    };
                }
            }

            channel.a_bus_address = a_bus_address;
            channel.byte_count = 0;
            self.dma.transfer_count = self
                .dma
                .transfer_count
                .saturating_add(transfer_length as u64);
            self.dma.set_channel(channel_index, channel);
        }
    }

    fn initialize_hdma_channels(&mut self) {
        let mask = self.dma.hdma_enable_mask();
        for channel_index in 0..8 {
            let Some(existing) = self.dma.channel(channel_index).copied() else {
                continue;
            };
            let mut channel = existing;
            if mask & (1 << channel_index) == 0 {
                channel.hdma_active = false;
                self.dma.set_channel(channel_index, channel);
                continue;
            }

            channel.hdma_table_address = channel.a_bus_address;
            channel.hdma_active = true;
            self.reload_hdma_block(channel_index, &mut channel);
            self.dma.set_channel(channel_index, channel);
        }
    }

    fn step_hdma_for_scanline(&mut self) {
        if self.timing.in_vblank() {
            return;
        }

        let mask = self.dma.hdma_enable_mask();
        for channel_index in 0..8 {
            if mask & (1 << channel_index) == 0 {
                continue;
            }
            let Some(existing) = self.dma.channel(channel_index).copied() else {
                continue;
            };
            let mut channel = existing;
            if !channel.hdma_active || channel.hdma_line_counter == 0 {
                continue;
            }

            if !channel.hdma_repeat {
                self.perform_hdma_transfer(&mut channel);
            }

            channel.hdma_line_counter = channel.hdma_line_counter.saturating_sub(1);
            if channel.hdma_line_counter == 0 {
                self.reload_hdma_block(channel_index, &mut channel);
            } else {
                channel.hdma_repeat = false;
            }
            self.dma.set_channel(channel_index, channel);
        }
    }

    fn reload_hdma_block(&mut self, channel_index: usize, channel: &mut crate::dma::DmaChannel) {
        let table_full =
            (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_table_address);
        let line_descriptor = self.read(table_full);
        channel.hdma_table_address = channel.hdma_table_address.wrapping_add(1);

        if line_descriptor == 0 {
            channel.hdma_active = false;
            channel.hdma_line_counter = 0;
            channel.hdma_repeat = false;
            return;
        }

        channel.hdma_active = true;
        channel.hdma_line_counter = line_descriptor & 0x7F;
        channel.hdma_repeat = line_descriptor & 0x80 != 0;

        if channel.hdma_indirect() {
            let low = self.read(
                (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_table_address),
            );
            channel.hdma_table_address = channel.hdma_table_address.wrapping_add(1);
            let high = self.read(
                (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_table_address),
            );
            channel.hdma_table_address = channel.hdma_table_address.wrapping_add(1);
            channel.indirect_address = u16::from_le_bytes([low, high]);
            channel.hdma_data_address = channel.indirect_address;
        } else {
            channel.hdma_data_address = channel.hdma_table_address;
        }

        let _ = channel_index;
    }

    fn perform_hdma_transfer(&mut self, channel: &mut crate::dma::DmaChannel) {
        let pattern = DmaController::b_bus_offsets_for_mode(channel.transfer_mode());
        for pattern_offset in pattern {
            let source_address =
                (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_data_address);
            let target_address =
                0x002100_u32 + u32::from(channel.b_bus_address) + u32::from(*pattern_offset);
            let value = self.read(source_address);
            self.write(target_address, value);
            channel.hdma_data_address = channel.hdma_data_address.wrapping_add(1);
            self.dma.transfer_count = self.dma.transfer_count.saturating_add(1);
        }

        if !channel.hdma_indirect() {
            channel.hdma_table_address = channel.hdma_data_address;
        }
    }

    fn read_mmio(&mut self, address: Address) -> Option<u8> {
        let register = (address & 0xFFFF) as u16;

        match register {
            0x2100..=0x213F => Some(self.ppu.read_register(register)),
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
                self.ppu.write_register(register, value);
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
            0x420B => {
                self.dma.set_dma_enable_mask(value);
                self.execute_dma(value);
                self.dma.set_dma_enable_mask(0);
                Some(())
            }
            0x420C => {
                self.dma.set_hdma_enable_mask(value);
                if self.timing.frame == 0 && self.timing.scanline == 0 && self.timing.dot == 0 {
                    self.initialize_hdma_channels();
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

fn map_lorom_save_ram(address: Address, len: usize) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as usize;
    let bank_index = usize::from(bank & 0x7F);

    match bank {
        0x70..=0x7D | 0xF0..=0xFF if offset <= 0x7FFF => {
            Some((bank_index - 0x70) * 0x8000 + offset)
        }
        _ => None,
    }
    .map(|index| index % len)
}

fn map_hirom_save_ram(address: Address, len: usize) -> Option<usize> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as usize;

    if !(0x6000..=0x7FFF).contains(&offset) {
        return None;
    }

    let bank_slot = match bank {
        0x20..=0x3F => usize::from(bank - 0x20),
        0xA0..=0xBF => usize::from((bank - 0xA0) + 0x20),
        _ => return None,
    };

    Some((bank_slot * 0x2000 + (offset - 0x6000)) % len)
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{Cartridge, Mapper};
    use crate::ppu::FrameBuffer;
    use crate::timing::{DOTS_PER_SCANLINE, NTSC_SCANLINES_PER_FRAME, VBLANK_START_SCANLINE};

    use super::*;

    fn make_cart(mapper: Mapper) -> Cartridge {
        make_cart_with_rom_type(mapper, 0x00)
    }

    fn make_cart_with_rom_type(mapper: Mapper, rom_type: u8) -> Cartridge {
        make_cart_with_title_and_rom_type(mapper, rom_type, b"STARBYTE SYSTEM BUS  ")
    }

    fn make_cart_with_title_and_rom_type(mapper: Mapper, rom_type: u8, title: &[u8; 21]) -> Cartridge {
        let mut rom = match mapper {
            Mapper::LoRom => vec![0_u8; 0x10000],
            Mapper::HiRom => vec![0_u8; 0x20000],
        };
        let base = match mapper {
            Mapper::LoRom => 0x7FC0,
            Mapper::HiRom => 0xFFC0,
        };
        rom[base..base + 21].copy_from_slice(title);
        rom[base + 0x15] = match mapper {
            Mapper::LoRom => 0x20,
            Mapper::HiRom => 0x21,
        };
        rom[base + 0x16] = rom_type;
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

    fn make_dsp_cart(mapper: Mapper) -> Cartridge {
        let mut rom = make_cart(mapper).rom().to_vec();
        let base = match mapper {
            Mapper::LoRom => 0x7FC0,
            Mapper::HiRom => 0xFFC0,
        };
        rom[base + 0x16] = 0x03;
        if matches!(mapper, Mapper::LoRom) {
            rom[base + 0x17] = 0x08;
            rom[base + 0x18] = 0x00;
        }
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
    fn maps_lorom_save_ram_reads_and_writes() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x700010, 0xA5);
        bus.write(0xF00020, 0x5A);

        assert_eq!(bus.read(0x700010), 0xA5);
        assert_eq!(bus.read(0xF00020), 0x5A);
        assert!(!bus.save_ram().is_empty());
    }

    #[test]
    fn maps_hirom_save_ram_reads_and_writes() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::HiRom));
        bus.write(0x206000, 0x11);
        bus.write(0xA06001, 0x22);

        assert_eq!(bus.read(0x206000), 0x11);
        assert_eq!(bus.read(0xA06001), 0x22);
        assert!(!bus.save_ram().is_empty());
    }

    #[test]
    fn routes_dsp_register_reads_and_writes_before_rom() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_dsp_cart(Mapper::LoRom));

        bus.write(0x308000, 0x78);
        bus.write(0x308001, 0x56);

        assert_eq!(bus.read(0x308000), 0x78);
        assert_eq!(bus.read(0x308001), 0x56);
        assert_eq!(bus.read(0x30C000), 0x80);
        assert!(matches!(
            bus.coprocessor().map(crate::coprocessor::Coprocessor::kind),
            Some(crate::coprocessor::CoprocessorKind::Dsp)
        ));
    }

    #[test]
    fn routes_superfx_register_reads_and_writes_before_rom() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart_with_rom_type(Mapper::LoRom, 0x13));

        bus.write(0x003000, 0x78);
        bus.write(0x003001, 0x56);
        bus.write(0x003030, 0x01);
        bus.write(0x003031, 0x80);
        bus.write(0x003034, 0x12);
        bus.write(0x00303E, 0x34);
        bus.write(0x00303F, 0x12);

        assert_eq!(bus.read(0x003000), 0x78);
        assert_eq!(bus.read(0x003001), 0x56);
        assert_eq!(bus.read(0x003030), 0x01);
        assert_eq!(bus.read(0x003031), 0x80);
        assert_eq!(bus.read(0x003034), 0x12);
        assert_eq!(bus.read(0x00303E), 0x34);
        assert_eq!(bus.read(0x00303F), 0x12);
        assert!(matches!(
            bus.coprocessor().map(crate::coprocessor::Coprocessor::kind),
            Some(crate::coprocessor::CoprocessorKind::SuperFx)
        ));
    }

    #[test]
    fn routes_sa1_mmio_and_shared_ram_before_rom() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart_with_rom_type(Mapper::LoRom, 0x34));

        bus.write(0x002202, 0x78);
        bus.write(0x002203, 0x56);
        bus.write(0x002206, 0x08);
        bus.write(0x002208, 0x55);
        bus.write(0x00220A, 0x01);
        bus.write(0x002200, 0x80);
        bus.write(0x003000, 0xAA);
        bus.write(0x406000, 0xCC);
        bus.advance_master_clocks(8);

        assert_eq!(bus.read(0x002201), 0xC0);
        assert_eq!(bus.read(0x002209), 0x88);
        assert_eq!(bus.read(0x003000), 0xAA);
        assert_eq!(bus.read(0x406000), 0xCC);
        assert!(matches!(
            bus.coprocessor().map(crate::coprocessor::Coprocessor::kind),
            Some(crate::coprocessor::CoprocessorKind::Sa1)
        ));
    }

    #[test]
    fn routes_cx4_command_window_before_rom() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart_with_title_and_rom_type(Mapper::LoRom, 0xF3, b"STARBYTE CX4 TEST    "));

        for (address, value) in [
            (0x006000, 3),
            (0x006001, 0),
            (0x006002, 4),
            (0x006003, 0),
            (0x006004, 12),
            (0x006005, 0),
            (0x007F40, 0x10),
        ] {
            bus.write(address, value);
        }
        bus.advance_master_clocks(10);

        assert_eq!(bus.read(0x006010), 13);
        assert_eq!(bus.read(0x006011), 0);
        assert_eq!(bus.read(0x007F41), 0x81);
        assert!(matches!(
            bus.coprocessor().map(crate::coprocessor::Coprocessor::kind),
            Some(crate::coprocessor::CoprocessorKind::Cx4)
        ));
    }

    #[test]
    fn routes_secondary_chip_register_windows_before_rom() {
        let mut sdd1 = SystemBus::default();
        sdd1.install_cartridge(make_cart_with_rom_type(Mapper::LoRom, 0x43));
        sdd1.write(0x004800, 0x01);
        sdd1.write(0x004801, 0x01);
        sdd1.write(0x004804, 0x82);
        sdd1.write(0x004804, 0x41);
        sdd1.advance_master_clocks(4);
        assert_eq!(sdd1.read(0x004806), 0x81);
        assert_eq!(sdd1.read(0x004805), 0x41);

        let mut obc1 = SystemBus::default();
        obc1.install_cartridge(make_cart_with_rom_type(Mapper::LoRom, 0x23));
        obc1.write(0x007FF6, 0x05);
        obc1.write(0x007FF0, 0x12);
        assert_eq!(obc1.read(0x007FF0), 0x12);

        let mut srtc = SystemBus::default();
        srtc.install_cartridge(make_cart_with_rom_type(Mapper::LoRom, 0x53));
        srtc.write(0x002801, 0x0E);
        srtc.write(0x002801, 0x0F);
        assert_eq!(srtc.read(0x002800), 0x01);
        assert!(matches!(
            srtc.coprocessor().map(crate::coprocessor::Coprocessor::kind),
            Some(crate::coprocessor::CoprocessorKind::SRtc)
        ));
    }

    #[test]
    fn dsp_bus_path_supports_buffered_command_execution() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_dsp_cart(Mapper::LoRom));

        for word in [0x0000_u16, 0x4000, 0x4000] {
            bus.write(0x308000, (word & 0x00FF) as u8);
            bus.write(0x308001, (word >> 8) as u8);
        }

        assert_eq!(bus.read(0x30C000), 0x20);
        bus.advance_master_clocks(6);
        assert_eq!(bus.read(0x30C000), 0xC0);
        assert_eq!(bus.read(0x308000), 0x00);
        assert_eq!(bus.read(0x308001), 0x20);
        assert_eq!(bus.read(0x30C000), 0x80);
    }

    #[test]
    fn validates_external_save_ram_size() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));

        let error = bus.load_save_ram(&[0xAA]).unwrap_err();
        assert!(matches!(
            error,
            crate::Error::InvalidSaveRam {
                expected: _,
                actual: 1
            }
        ));
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
    fn dma_can_stream_bytes_into_ppu_cgram() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x7E0000, 0x1F);
        bus.write(0x7E0001, 0x00);
        bus.write(0x002121, 0x00);
        bus.write(0x004300, 0x02);
        bus.write(0x004301, 0x22);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x00);
        bus.write(0x004304, 0x7E);
        bus.write(0x004305, 0x02);
        bus.write(0x004306, 0x00);
        bus.write(0x00420B, 0x01);

        assert_eq!(&bus.ppu().cgram()[..2], &[0x1F, 0x00]);
    }

    #[test]
    fn hdma_applies_one_line_of_direct_data() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x002121, 0x00);
        bus.write(0x7E0100, 0x01);
        bus.write(0x7E0101, 0x00);
        bus.write(0x7E0102, 0x7C);
        bus.write(0x7E0103, 0x00);
        bus.write(0x004300, 0x02);
        bus.write(0x004301, 0x22);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x00420C, 0x01);
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE));

        assert_eq!(&bus.ppu().cgram()[..2], &[0x00, 0x7C]);
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

    #[test]
    fn ppu_register_writes_drive_rendered_frame() {
        let mut bus = SystemBus::default();
        let mut frame = FrameBuffer::default();
        bus.write(0x002121, 0x00);
        bus.write(0x002122, 0x00);
        bus.write(0x002122, 0x7C);
        bus.write(0x00212C, 0x01);
        bus.render_frame(&mut frame);

        assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 0xFF]);
    }
}
