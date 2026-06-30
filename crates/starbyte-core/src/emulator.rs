//! Emulator facade exposed to CLI and future frontends.

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::apu::{Apu, ApuStatus, AudioFrame};
use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu_65816::Cpu65816;
use crate::cpu_65816::registers::Registers;
use crate::error::{Error, Result};
use crate::manifest::AssetConfig;
use crate::ppu::FrameBuffer;
use crate::system::SystemBus;
use crate::timing::TimingState;

const CPU_BUS_CYCLE_MASTER_CYCLES: u64 = 6;
const SAVE_STATE_VERSION: u32 = 1;

/// Serializable emulator state snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveState {
    /// Save-state format version.
    pub version: u32,
    /// CPU state.
    pub cpu: Cpu65816,
    /// APU boundary state.
    pub apu: Apu,
    /// CPU-visible memory, MMIO, cartridge, and timing state.
    pub system: SystemBus,
}

/// Builder for the emulator facade.
#[derive(Debug, Clone, Default)]
pub struct EmulatorBuilder {
    assets: AssetConfig,
}

impl EmulatorBuilder {
    /// Create a fresh builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure asset paths.
    #[must_use]
    pub fn assets(mut self, assets: AssetConfig) -> Self {
        self.assets = assets;
        self
    }

    /// Build an emulator with no cartridge loaded yet.
    #[must_use]
    pub fn build(self) -> Emulator {
        Emulator {
            cpu: Cpu65816::default(),
            apu: Apu::with_ipl_path(self.assets.spc700_ipl.clone()),
            frame_buffer: FrameBuffer::default(),
            pending_audio: AudioFrame::default(),
            system: SystemBus::default(),
            assets: self.assets,
        }
    }
}

/// Bootstrap emulator facade. The internal subsystem behavior is intentionally skeletal.
#[derive(Debug, Clone)]
pub struct Emulator {
    cpu: Cpu65816,
    apu: Apu,
    frame_buffer: FrameBuffer,
    pending_audio: AudioFrame,
    system: SystemBus,
    assets: AssetConfig,
}

impl Default for Emulator {
    fn default() -> Self {
        EmulatorBuilder::default().build()
    }
}

impl Emulator {
    /// Load a ROM into the emulator.
    #[instrument(skip_all)]
    pub fn load_rom(&mut self, rom: Cartridge) {
        debug!(title = rom.header().title, mapper = ?rom.mapper(), "installing cartridge");
        self.system.install_cartridge(rom);
        self.reset();
    }

    /// Reset subsystem state.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.apu.reset();
        self.system.reset();
        self.system.sync_apu_ports_from_runtime(&self.apu);
        if let Some(vector) = self.system.reset_vector() {
            self.cpu.registers.pc = vector;
        }
        self.pending_audio = AudioFrame::default();
        self.frame_buffer = FrameBuffer::default();
    }

    /// Execute placeholder work until one frame boundary.
    pub fn run_until_frame(&mut self) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        self.pending_audio.samples.clear();
        let start_frame = self.system.timing().frame;
        while self.system.timing().frame == start_frame {
            self.step_instruction()?;
        }
        self.system.render_frame(&mut self.frame_buffer);
        debug!(frame = self.system.timing().frame, "advanced to next frame");
        Ok(())
    }

    /// Step one instruction in the placeholder model.
    pub fn step_instruction(&mut self) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        self.system.sync_apu_ports_from_runtime(&self.apu);
        let trace = self.cpu.step_with_bus(&mut self.system)?;
        self.system.sync_apu_ports_to_runtime(&mut self.apu);
        let master_cycles = (trace.len() as u64).saturating_mul(CPU_BUS_CYCLE_MASTER_CYCLES);
        self.apu.step_master_cycles(master_cycles);
        self.system.sync_apu_ports_from_runtime(&self.apu);
        self.system.advance_master_clocks(master_cycles);
        self.append_audio_samples(master_cycles);
        Ok(())
    }

    /// Borrow the current framebuffer.
    #[must_use]
    pub const fn framebuffer(&self) -> &FrameBuffer {
        &self.frame_buffer
    }

    /// Borrow buffered audio samples.
    #[must_use]
    pub const fn audio_samples(&self) -> &AudioFrame {
        &self.pending_audio
    }

    /// Save-RAM bytes if present.
    #[must_use]
    pub fn save_ram(&self) -> Option<Vec<u8>> {
        self.system
            .save_ram_len()
            .map(|_| self.system.save_ram().to_vec())
    }

    /// Install externally persisted save RAM for the loaded cartridge.
    pub fn load_save_ram(&mut self, bytes: &[u8]) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        self.system.load_save_ram(bytes)
    }

    /// Serialize a save-state snapshot.
    pub fn save_state(&self) -> Result<String> {
        serde_json::to_string_pretty(&SaveState {
            version: SAVE_STATE_VERSION,
            cpu: self.cpu.clone(),
            apu: self.apu.clone(),
            system: self.system.clone(),
        })
        .map_err(Into::into)
    }

    /// Restore a save-state snapshot.
    pub fn load_state(&mut self, state: &str) -> Result<()> {
        let state: SaveState = serde_json::from_str(state)?;
        if state.version != SAVE_STATE_VERSION {
            return Err(Error::InvalidRom(format!(
                "unsupported save-state version {}",
                state.version
            )));
        }
        self.cpu = state.cpu;
        self.apu = state.apu;
        self.system = state.system;
        self.system.sync_apu_ports_from_runtime(&self.apu);
        Ok(())
    }

    /// Return configured asset paths.
    #[must_use]
    pub const fn assets(&self) -> &AssetConfig {
        &self.assets
    }

    /// Return a high-level APU bootstrap status snapshot.
    #[must_use]
    pub fn apu_status(&self) -> ApuStatus {
        self.apu.status()
    }

    /// Load the configured user-supplied SPC700 IPL ROM if present.
    pub fn load_apu_ipl_rom(&mut self) -> Result<bool> {
        self.apu.load_configured_ipl_rom()
    }

    /// Host-side write access used by bootstrap tests and regression fixtures.
    pub fn host_write_u8(&mut self, address: u32, value: u8) {
        self.system.write(address, value);
    }

    /// Host-side read access used by bootstrap tests and regression fixtures.
    #[must_use]
    pub fn host_read_u8(&mut self, address: u32) -> u8 {
        self.system.read(address)
    }

    /// Borrow the current CPU register file.
    #[must_use]
    pub const fn cpu_registers(&self) -> &Registers {
        &self.cpu.registers
    }

    /// Borrow the current timing state.
    #[must_use]
    pub const fn timing(&self) -> &TimingState {
        self.system.timing()
    }

    /// Set controller-1 state from a host/frontend.
    pub fn set_controller1(&mut self, state: crate::input::ControllerState) {
        self.system.set_controller1(state);
    }

    fn append_audio_samples(&mut self, master_cycles: u64) {
        let sample_pairs = (master_cycles / CPU_BUS_CYCLE_MASTER_CYCLES).max(1) as usize;
        let phase = self.apu_status().spc700_steps as i16;
        let amplitude = ((phase & 0x1F) + 1) * 192;
        for index in 0..sample_pairs {
            let sample = if index % 2 == 0 {
                amplitude
            } else {
                -amplitude
            };
            self.pending_audio.samples.push(sample);
            self.pending_audio.samples.push(sample);
        }
    }

    /// Return the loaded cartridge if any.
    #[must_use]
    pub const fn cartridge(&self) -> Option<&Cartridge> {
        self.system.cartridge()
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{Cartridge, Mapper};
    use crate::input::ControllerState;

    use super::Emulator;

    fn rom_bytes() -> Vec<u8> {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE EMULATOR    ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0xFF;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0x00;
        rom[base + 0x1F] = 0x00;
        rom
    }

    #[test]
    fn state_roundtrip_preserves_timing() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        let cart = Cartridge::from_bytes(rom, None).unwrap();
        assert_eq!(cart.mapper(), Mapper::LoRom);

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);
        emulator.step_instruction().unwrap();
        let state = emulator.save_state().unwrap();

        let mut restored = Emulator::default();
        restored.load_state(&state).unwrap();
        assert_eq!(restored.save_state().unwrap(), state);
    }

    #[test]
    fn step_instruction_uses_reset_vector_and_bus_timing() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);
        emulator.step_instruction().unwrap();

        assert_eq!(emulator.cpu.registers.pc, 0x8001);
        assert_eq!(emulator.system.timing().master_clock, 12);
    }

    #[test]
    fn run_until_frame_renders_framebuffer() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);
        emulator.host_write_u8(0x002121, 0x00);
        emulator.host_write_u8(0x002122, 0x00);
        emulator.host_write_u8(0x002122, 0x7C);
        emulator.host_write_u8(0x00212C, 0x01);
        emulator.run_until_frame().unwrap();

        assert_eq!(emulator.framebuffer().pixels()[..4], [248, 0, 0, 0xFF]);
    }

    #[test]
    fn run_until_frame_buffers_audio_and_accepts_input() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);
        emulator.set_controller1(ControllerState {
            start: true,
            a: true,
            ..ControllerState::default()
        });
        emulator.host_write_u8(0x004016, 0x01);
        emulator.host_write_u8(0x004016, 0x00);
        emulator.run_until_frame().unwrap();

        assert!(!emulator.audio_samples().samples.is_empty());
        assert_eq!(emulator.host_read_u8(0x004218), 0x08);
        assert_eq!(emulator.host_read_u8(0x004219), 0x01);
    }

    #[test]
    fn rejects_unknown_save_state_version() {
        let state = r#"{"version":999,"cpu":{"registers":{"a":0,"x":0,"y":0,"s":0,"d":0,"pc":0,"pbr":0,"dbr":0,"p":0,"emulation":false},"cycles":0},"apu":{"spc700":{"pc":0,"a":0,"x":0,"y":0,"sp":0,"psw":0,"cycles":0},"cpu_to_apu_ports":[0,0,0,0],"apu_to_cpu_ports":[0,0,0,0],"ipl_rom":null,"configured_ipl_path":null,"spc700_steps":0},"system":{"cartridge":null,"save_ram":[],"wram":[],"ppu":{"registers":[],"cgram":[],"cgram_address":0,"cgram_high_byte":false},"cpu_to_apu_io":[0,0,0,0],"apu_to_cpu_io":[0,0,0,0],"dma":{"channels":[{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false}],"dma_enable_mask":0,"hdma_enable_mask":0,"transfer_count":0},"timing":{"master_clock":0,"scanline":0,"dot":0,"frame":0},"open_bus":0,"nmitimen":0,"rdnmi":false,"timeup":false,"joypad":{"controller1":{"b":false,"y":false,"select":false,"start":false,"up":false,"down":false,"left":false,"right":false,"a":false,"x":false,"l":false,"r":false},"latch_line":false,"latched1":0,"shift1":0},"wram_address":0}}"#;
        let mut emulator = Emulator::default();
        assert!(emulator.load_state(state).is_err());
    }
}
