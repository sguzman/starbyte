//! Emulator facade exposed to CLI and future frontends.

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::apu::{Apu, ApuStatus, AudioFrame};
use crate::cartridge::Cartridge;
use crate::cpu_65816::Cpu65816;
use crate::error::{Error, Result};
use crate::manifest::AssetConfig;
use crate::ppu::FrameBuffer;
use crate::system::SystemBus;

const CPU_BUS_CYCLE_MASTER_CYCLES: u64 = 6;

/// Serializable emulator state snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveState {
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
        self.apu.set_ipl_path(self.assets.spc700_ipl.clone());
        self.system.reset();
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

        let start_frame = self.system.timing().frame;
        while self.system.timing().frame == start_frame {
            self.step_instruction()?;
        }
        debug!(frame = self.system.timing().frame, "advanced to next frame");
        Ok(())
    }

    /// Step one instruction in the placeholder model.
    pub fn step_instruction(&mut self) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        let trace = self.cpu.step_with_bus(&mut self.system)?;
        let master_cycles = (trace.len() as u64).saturating_mul(CPU_BUS_CYCLE_MASTER_CYCLES);
        self.apu.step_master_cycles(master_cycles);
        self.system.advance_master_clocks(master_cycles);
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
        self.system.save_ram_len().map(|len| vec![0; len])
    }

    /// Serialize a save-state snapshot.
    pub fn save_state(&self) -> Result<String> {
        serde_json::to_string_pretty(&SaveState {
            cpu: self.cpu.clone(),
            apu: self.apu.clone(),
            system: self.system.clone(),
        })
        .map_err(Into::into)
    }

    /// Restore a save-state snapshot.
    pub fn load_state(&mut self, state: &str) -> Result<()> {
        let state: SaveState = serde_json::from_str(state)?;
        self.cpu = state.cpu;
        self.apu = state.apu;
        self.system = state.system;
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

    /// Return the loaded cartridge if any.
    #[must_use]
    pub const fn cartridge(&self) -> Option<&Cartridge> {
        self.system.cartridge()
    }
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{Cartridge, Mapper};

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
}
