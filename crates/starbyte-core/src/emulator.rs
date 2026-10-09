//! Emulator facade exposed to CLI and future frontends.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::apu::{Apu, ApuStatus, AudioFrame};
use crate::bus::{Bus, BusEvent};
use crate::cartridge::Cartridge;
use crate::cpu_65816::Cpu65816;
use crate::cpu_65816::registers::Registers;
use crate::error::{Error, Result};
use crate::manifest::AssetConfig;
use crate::ppu::FrameBuffer;
use crate::system::{SystemBus, SystemBusObservability};
use crate::timing::TimingState;

const CPU_BUS_CYCLE_MASTER_CYCLES: u64 = 6;
const SAVE_STATE_VERSION: u32 = 1;
/// More than the maximum number of one-bus-access CPU steps in a frame.
const MAX_INSTRUCTIONS_PER_FRAME: usize = 100_000;

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

    /// Advance one frame with a deterministic progress guard. Headless
    /// clients are not subject to a host wall-clock deadline.
    pub fn run_until_frame(&mut self) -> Result<()> {
        self.run_until_frame_guarded(None, None)
    }

    /// Advance one frame with an additional wall-clock budget for synchronous
    /// desktop UIs. A stalled commercial ROM returns a diagnostic error
    /// rather than holding the event loop indefinitely.
    pub fn run_until_frame_with_timeout(&mut self, timeout: Duration) -> Result<()> {
        self.run_until_frame_guarded(Some(timeout), None)
    }

    /// Run one guarded frame, reporting CPU register state and bus events for
    /// each successfully completed instruction. This is opt-in diagnostics;
    /// the ordinary frame loop retains its minimal-cost execution path.
    pub fn run_until_frame_observed(
        &mut self,
        observer: &mut dyn FnMut(&Registers, &Registers, &[BusEvent]),
    ) -> Result<()> {
        self.run_until_frame_guarded(None, Some(observer))
    }

    fn run_until_frame_guarded(
        &mut self,
        timeout: Option<Duration>,
        mut observer: Option<&mut dyn FnMut(&Registers, &Registers, &[BusEvent])>,
    ) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        self.pending_audio.samples.clear();
        let start_frame = self.system.timing().frame;
        let started = Instant::now();
        let mut instructions = 0_usize;
        while self.system.timing().frame == start_frame {
            if instructions >= MAX_INSTRUCTIONS_PER_FRAME
                || timeout.is_some_and(|budget| started.elapsed() >= budget)
            {
                return Err(self.frame_stalled(start_frame, instructions, started.elapsed()));
            }
            let before_clock = self.system.timing().master_clock;
            if let Some(record) = observer.as_mut() {
                let before = self.cpu.registers.clone();
                let bus_events = self.step_instruction_with_trace()?;
                record(&before, &self.cpu.registers, &bus_events);
            } else {
                self.step_instruction()?;
            }
            instructions += 1;
            if self.system.timing().master_clock == before_clock
                || timeout.is_some_and(|budget| started.elapsed() >= budget)
            {
                return Err(self.frame_stalled(start_frame, instructions, started.elapsed()));
            }
        }
        self.refresh_framebuffer();
        debug!(
            frame = self.system.timing().frame,
            instructions, "advanced to next frame"
        );
        Ok(())
    }

    fn frame_stalled(&self, frame: u64, instructions: usize, elapsed: Duration) -> Error {
        Error::FrameStalled {
            frame,
            instructions,
            elapsed_ms: elapsed.as_millis(),
            pc: (u32::from(self.cpu.registers.pbr) << 16) | u32::from(self.cpu.registers.pc),
        }
    }

    /// Step one instruction in the placeholder model.
    pub fn step_instruction(&mut self) -> Result<()> {
        let _ = self.step_instruction_with_trace()?;
        Ok(())
    }

    /// Step one instruction and return the captured CPU bus events.
    pub fn step_instruction_with_trace(&mut self) -> Result<Vec<BusEvent>> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }

        self.system.sync_apu_ports_from_runtime(&self.apu);
        let trace = self.cpu.step_with_bus(&mut self.system)?;
        self.system.sync_apu_ports_to_runtime(&mut self.apu);
        // WAI and STP produce no CPU bus accesses while idling, but the
        // console's PPU/APU clocks must continue advancing. Count one
        // internal CPU cycle for each empty instruction-step trace.
        let master_cycles = (trace.len() as u64)
            .max(1)
            .saturating_mul(CPU_BUS_CYCLE_MASTER_CYCLES);
        self.apu.step_master_cycles(master_cycles);
        self.system.sync_apu_ports_from_runtime(&self.apu);
        self.system.advance_master_clocks(master_cycles);
        self.append_audio_samples(master_cycles);
        Ok(trace)
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
        // Save states include the full original cartridge. Never silently
        // replace an already loaded different game with snapshot contents.
        // An empty emulator may still be initialized from a full snapshot.
        if let Some(current) = self.system.cartridge() {
            match state.system.cartridge() {
                Some(saved)
                    if current.mapper() == saved.mapper() && current.rom() == saved.rom() => {}
                _ => {
                    return Err(Error::InvalidRom(
                        "save state belongs to a different cartridge".to_owned(),
                    ));
                }
            }
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

    /// Borrow the current CPU-to-APU communication ports.
    #[must_use]
    pub fn cpu_to_apu_ports(&self) -> &[u8] {
        self.system.cpu_to_apu_ports()
    }

    /// Borrow the current APU-to-CPU communication ports.
    #[must_use]
    pub fn apu_to_cpu_ports(&self) -> &[u8] {
        self.system.apu_to_cpu_ports()
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

    /// Number of bytes transferred by DMA and HDMA in this session.
    #[must_use]
    pub const fn dma_transferred_bytes(&self) -> u64 {
        self.system.dma_transferred_bytes()
    }

    /// Return current NMITIMEN and H/V IRQ timer compare coordinates.
    #[must_use]
    pub const fn irq_timer_configuration(&self) -> (u8, u16, u16) {
        self.system.irq_timer_configuration()
    }

    /// Nonmutating host controller bits, latched automatic/serial bits and busy state.
    #[must_use]
    pub const fn joypad_status(&self) -> (u16, u16, bool) {
        self.system.joypad_status()
    }

    /// Borrow compact bus activity counters for CLI reporting and regressions.
    #[must_use]
    pub fn system_observability(&self) -> &SystemBusObservability {
        self.system.observability()
    }

    /// Read one PPU register without mutating bus-visible side effects.
    #[must_use]
    pub fn peek_ppu_register(&self, register: u16) -> Option<u8> {
        self.system.peek_ppu_register(register)
    }

    /// Re-render the current system state into the framebuffer.
    pub fn refresh_framebuffer(&mut self) {
        self.system.render_frame(&mut self.frame_buffer);
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
    use crate::timing::{DOTS_PER_SCANLINE, MASTER_CLOCKS_PER_DOT, NTSC_SCANLINES_PER_FRAME};

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
    fn loading_wrong_cartridge_state_is_rejected_without_mutating_session() {
        let mut first_rom = rom_bytes();
        first_rom[0x7FFC] = 0x00;
        first_rom[0x7FFD] = 0x80;
        first_rom[0x0000] = 0xEA;

        let mut first = Emulator::default();
        first.load_rom(Cartridge::from_bytes(first_rom.clone(), None).unwrap());
        first.step_instruction().unwrap();
        let state = first.save_state().unwrap();

        let mut second_rom = first_rom.clone();
        second_rom[0x0001] = 0xFF; // Same title/header but a different ROM.
        let mut second = Emulator::default();
        second.load_rom(Cartridge::from_bytes(second_rom, None).unwrap());
        let before = second.save_state().unwrap();
        assert!(second.load_state(&state).is_err());
        assert_eq!(second.save_state().unwrap(), before);

        // Content identity, not filename equality, is what matters.
        let mut same_rom = Emulator::default();
        same_rom.load_rom(
            Cartridge::from_bytes(first_rom, Some(std::path::PathBuf::from("elsewhere.sfc")))
                .unwrap(),
        );
        same_rom.load_state(&state).unwrap();
        assert_eq!(same_rom.timing().frame, first.timing().frame);
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
    fn bounded_frame_reports_diagnostic_without_hanging_or_advancing() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0;
        rom[0x7FFD] = 0x80;
        rom[0] = 0xEA;
        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());

        let error = emulator
            .run_until_frame_with_timeout(std::time::Duration::ZERO)
            .unwrap_err();
        assert!(matches!(
            error,
            crate::error::Error::FrameStalled {
                frame: 0,
                instructions: 0,
                pc: 0x008000,
                ..
            }
        ));
        assert_eq!(emulator.timing().frame, 0);
        emulator.run_until_frame().unwrap();
        assert_eq!(emulator.timing().frame, 1);
    }

    #[test]
    fn a_waiting_or_stopped_cpu_does_not_halt_ppu_frame_timing() {
        for opcode in [0xCB, 0xDB] {
            let mut rom = rom_bytes();
            rom[0x7FFC] = 0x00;
            rom[0x7FFD] = 0x80;
            rom[0] = opcode;
            rom[1] = 0xEA;

            let mut emulator = Emulator::default();
            emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());
            emulator.run_until_frame().unwrap();
            assert_eq!(emulator.timing().frame, 1, "opcode {opcode:02X}");
            assert_eq!(emulator.cpu_registers().pc, 0x8001);

            emulator.run_until_frame().unwrap();
            assert_eq!(emulator.timing().frame, 2, "opcode {opcode:02X}");
            assert_eq!(emulator.cpu_registers().pc, 0x8001);
        }
    }

    #[test]
    fn a_game_program_receives_one_nmi_at_ntsc_vblank_per_frame() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        // Emulation-mode NMI vector points to the handler at $8010.
        rom[0x7FFA] = 0x10;
        rom[0x7FFB] = 0x80;
        // LDA #$80 / STA $4200 / BRA * (wait forever for NMI).
        rom[..7].copy_from_slice(&[0xA9, 0x80, 0x8D, 0x00, 0x42, 0x80, 0xFE]);
        // INC $00 / RTI: record one NMI without modifying any ROM state.
        rom[0x10..0x13].copy_from_slice(&[0xE6, 0x00, 0x40]);

        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());

        emulator.run_until_frame().unwrap();
        assert_eq!(emulator.host_read_u8(0x000000), 1);
        let clocks_per_frame = u64::from(DOTS_PER_SCANLINE)
            * u64::from(NTSC_SCANLINES_PER_FRAME)
            * MASTER_CLOCKS_PER_DOT;
        assert!(emulator.timing().master_clock >= clocks_per_frame);
        assert!(emulator.timing().master_clock < clocks_per_frame + 128);

        emulator.run_until_frame().unwrap();
        assert_eq!(emulator.host_read_u8(0x000000), 2);
    }

    #[test]
    fn native_nmi_with_full_register_save_restores_stack_and_program_bank() {
        let mut rom = rom_bytes();
        // LoROM NMI vector ($00:FFEA) enters bank zero at $8100.
        rom[0x7FEA] = 0x00;
        rom[0x7FEB] = 0x81;
        rom[0] = 0x80; // BRA -2, a stable native-mode main loop.
        rom[1] = 0xFE;
        // SEI, PHP, REP #$30, PHA, PHX, PHY, PHB, PHK, PLB,
        // SEP #$30, LDA $4210, REP #$30, PLB, PLY, PLX, PLA,
        // PLP, RTI: standard SNES interrupt prologue/epilogue.
        let handler: &[u8] = &[
            0x78, 0x08, 0xC2, 0x30, 0x48, 0xDA, 0x5A, 0x8B, 0x4B, 0xAB, 0xE2, 0x30, 0xAD, 0x10,
            0x42, 0xC2, 0x30, 0xAB, 0x7A, 0xFA, 0x68, 0x28, 0x40,
        ];
        rom[0x100..0x100 + handler.len()].copy_from_slice(handler);

        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());
        emulator.cpu.registers.emulation = false;
        emulator.cpu.registers.p = 0x30; // M and X must survive hardware NMI.
        emulator.cpu.registers.pc = 0x8000;
        emulator.cpu.registers.pbr = 0x80;
        emulator.cpu.registers.dbr = 0x14;
        emulator.cpu.registers.s = 0x01FF;
        emulator.cpu.registers.a = 0xBEEF;
        // X=1 means 8-bit index registers; their upper bytes cannot survive
        // the handler's REP/SEP mode changes and must already be zero.
        emulator.cpu.registers.x = 0x0034;
        emulator.cpu.registers.y = 0x0078;
        emulator.host_write_u8(0x004200, 0x80);

        for _ in 0..3 {
            emulator.run_until_frame().unwrap();
            let regs = &emulator.cpu.registers;
            assert_eq!(regs.pbr, 0x80, "NMI failed to restore program bank");
            assert_eq!(regs.p & 0x30, 0x30, "NMI erased accumulator/index width");
            assert_eq!(regs.s, 0x01FF, "NMI failed to restore stack pointer");
            assert_eq!(regs.a, 0xBEEF, "NMI failed to restore accumulator");
            assert_eq!(regs.x, 0x0034, "NMI failed to restore X");
            assert_eq!(regs.y, 0x0078, "NMI failed to restore Y");
            assert_eq!(regs.dbr, 0x14, "NMI failed to restore data bank");
        }
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
        emulator.host_write_u8(0x002100, 0x0F);
        emulator.host_write_u8(0x002121, 0x00);
        emulator.host_write_u8(0x002122, 0x1F);
        emulator.host_write_u8(0x002122, 0x00);
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
    fn step_instruction_keeps_apu_ports_coherent_across_cpu_write_and_readback() {
        let mut rom = rom_bytes();
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xA9;
        rom[0x0001] = 0xCC;
        rom[0x0002] = 0x8D;
        rom[0x0003] = 0x40;
        rom[0x0004] = 0x21;
        rom[0x0005] = 0xAD;
        rom[0x0006] = 0x40;
        rom[0x0007] = 0x21;
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);

        assert_eq!(emulator.host_read_u8(0x002140), 0xAA);

        emulator.step_instruction().unwrap();
        emulator.step_instruction().unwrap();

        assert_eq!(emulator.cpu_to_apu_ports()[0], 0xCC);
        assert_eq!(emulator.apu_to_cpu_ports()[0], 0xCC);
        assert_eq!(emulator.host_read_u8(0x002140), 0xCC);

        emulator.step_instruction().unwrap();

        assert_eq!(emulator.cpu_registers().a as u8, 0xCC);
        assert_eq!(emulator.host_read_u8(0x002140), 0xCC);
    }

    #[test]
    fn rejects_unknown_save_state_version() {
        let state = r#"{"version":999,"cpu":{"registers":{"a":0,"x":0,"y":0,"s":0,"d":0,"pc":0,"pbr":0,"dbr":0,"p":0,"emulation":false},"cycles":0},"apu":{"spc700":{"pc":0,"a":0,"x":0,"y":0,"sp":0,"psw":0,"cycles":0},"cpu_to_apu_ports":[0,0,0,0],"apu_to_cpu_ports":[0,0,0,0],"ipl_rom":null,"configured_ipl_path":null,"spc700_steps":0},"system":{"cartridge":null,"save_ram":[],"wram":[],"ppu":{"registers":[],"cgram":[],"cgram_address":0,"cgram_high_byte":false},"cpu_to_apu_io":[0,0,0,0],"apu_to_cpu_io":[0,0,0,0],"dma":{"channels":[{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false}],"dma_enable_mask":0,"hdma_enable_mask":0,"transfer_count":0},"timing":{"master_clock":0,"scanline":0,"dot":0,"frame":0},"open_bus":0,"nmitimen":0,"rdnmi":false,"timeup":false,"joypad":{"controller1":{"b":false,"y":false,"select":false,"start":false,"up":false,"down":false,"left":false,"right":false,"a":false,"x":false,"l":false,"r":false},"latch_line":false,"latched1":0,"shift1":0},"wram_address":0}}"#;
        let mut emulator = Emulator::default();
        assert!(emulator.load_state(state).is_err());
    }
}
