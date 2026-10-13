//! Emulator facade exposed to CLI and future frontends.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::apu::{Apu, ApuStatus, AudioFrame, IsolatedSpc700Status, upload::IplUpload};
use crate::bus::{Bus, BusEvent};
use crate::cartridge::Cartridge;
use crate::cpu_65816::Cpu65816;
use crate::cpu_65816::registers::Registers;
use crate::error::{Error, Result};
use crate::manifest::AssetConfig;
use crate::ppu::FrameBuffer;
use crate::system::{RomReadPatch, SystemBus, SystemBusObservability};
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
            cpu_trace_scratch: Vec::new(),
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
    cpu_trace_scratch: Vec<BusEvent>,
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
        // A new cartridge is never implicit permission to continue an
        // earlier experimental sound-driver upload or execution session.
        self.apu.disarm_isolated_ipl_upload();
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
        self.cpu_trace_scratch.clear();
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

    /// Run a normal game instruction using reusable bus-trace allocation.
    pub fn step_instruction(&mut self) -> Result<()> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }
        self.system.sync_apu_ports_from_runtime(&self.apu);
        self.cpu
            .step_with_bus_into(&mut self.system, &mut self.cpu_trace_scratch)?;
        self.finish_cpu_step(self.cpu_trace_scratch.len())
    }

    /// Step one instruction and return the captured CPU bus events.
    pub fn step_instruction_with_trace(&mut self) -> Result<Vec<BusEvent>> {
        if self.system.cartridge().is_none() {
            return Err(Error::InvalidRom("no ROM loaded".to_owned()));
        }
        self.system.sync_apu_ports_from_runtime(&self.apu);
        let trace = self.cpu.step_with_bus(&mut self.system)?;
        self.finish_cpu_step(trace.len())?;
        Ok(trace)
    }

    /// Advance all other subsystems identically with or without a CPU trace.
    fn finish_cpu_step(&mut self, bus_events: usize) -> Result<()> {
        self.system.sync_apu_ports_to_runtime(&mut self.apu);
        // WAI and STP idle cycles have no bus events but advance hardware time.
        let master_cycles = (bus_events as u64)
            .max(1)
            .saturating_mul(CPU_BUS_CYCLE_MASTER_CYCLES);
        self.advance_apu_for_cpu_step(master_cycles)?;
        self.system.sync_apu_ports_from_runtime(&self.apu);
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

    /// Capture an in-memory state without encoding the cartridge as JSON.
    ///
    /// Intended for ephemeral F5/F8 quick saves. Disk slots still use the
    /// versioned JSON format returned by `save_state`.
    #[must_use]
    pub fn capture_state(&self) -> SaveState {
        SaveState {
            version: SAVE_STATE_VERSION,
            cpu: self.cpu.clone(),
            apu: self.apu.clone(),
            system: self.system.clone(),
        }
    }

    /// Serialize a save-state snapshot in the existing disk-compatible format.
    pub fn save_state(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.capture_state()).map_err(Into::into)
    }

    /// Restore a serialized disk snapshot.
    pub fn load_state(&mut self, state: &str) -> Result<()> {
        let decoded: SaveState = serde_json::from_str(state)?;
        self.restore_state(decoded)
    }

    /// Restore an in-memory quick snapshot through the same cartridge and
    /// opt-in sound-driver safety checks used for persistent disk states.
    pub fn restore_state(&mut self, state: SaveState) -> Result<()> {
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
        // Restored APU data is not permission to enable experimental host
        // execution in an ordinary GUI or CLI session. Only a caller that
        // explicitly enabled the probe may keep it active through a load.
        let host_driver_was_opted_in = self.apu.isolated_host_clocked_driver_enabled();
        self.cpu = state.cpu;
        self.apu = state.apu;
        if !host_driver_was_opted_in {
            self.apu.disable_isolated_host_clocked_driver();
        }
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

    /// Opt into isolated SPC IPL transfer capture for diagnostic runs.
    ///
    /// Must be enabled after loading a ROM and before executing it. This
    /// replaces the fake bootstrap handshakes only in the current instance;
    /// it does not automatically run an uploaded game sound driver.
    /// Regular desktop and CLI playback must leave this disabled.
    pub fn enable_isolated_apu_upload_probe(&mut self) {
        self.apu.enable_isolated_ipl_upload();
        self.system.sync_apu_ports_from_runtime(&self.apu);
    }

    /// Execute an uploaded sound driver alongside CPU steps for opt-in
    /// diagnostics only. The ordinary upload probe and desktop runtime stay
    /// transport-only and keep the existing compatibility bootstrap.
    pub fn enable_isolated_apu_driver_probe(&mut self) -> Result<()> {
        self.enable_isolated_apu_upload_probe();
        self.apu.enable_isolated_host_clocked_driver()
    }

    /// Whether this emulator instance explicitly opted into sound-driver
    /// clocking. Ordinary gameplay never enables this path implicitly.
    #[must_use]
    pub fn isolated_apu_driver_probe_enabled(&self) -> bool {
        self.apu.isolated_host_clocked_driver_enabled()
    }

    /// Read the explicitly enabled sound-driver upload progress, if any.
    #[must_use]
    pub fn isolated_apu_upload(&self) -> Option<&IplUpload> {
        self.apu.isolated_ipl_upload()
    }

    /// Actual sound-CPU status after a completed opt-in upload handoff.
    #[must_use]
    pub fn isolated_apu_spc700_status(&self) -> Option<IsolatedSpc700Status> {
        self.apu.isolated_spc700_status()
    }

    /// Read one current DSP register in explicitly enabled IPL diagnostics.
    ///
    /// Production play does not expose an experimental sound-driver
    /// snapshot. Reading the partial DSP register file has no side effects.
    #[must_use]
    pub fn isolated_apu_dsp_register(&self, register: u8) -> Option<u8> {
        self.apu.isolated_ipl_upload()?;
        Some(self.apu.read_dsp_register(register))
    }

    /// Expose the internal SMP TEST latch only in opt-in IPL diagnostics.
    ///
    /// TEST is write-only to the SPC700; this does not alter guest MMIO
    /// reads and cannot be used to inspect a production sound session.
    #[must_use]
    pub fn isolated_apu_test_latch(&self) -> Option<u8> {
        self.apu.isolated_ipl_upload()?;
        Some(self.apu.test_latch())
    }

    /// Replace cartridge-ROM substitutions supplied by the host cheat manager.
    ///
    /// These are intentionally not persisted in SNES save states.
    pub fn set_rom_read_patches(&mut self, patches: &[RomReadPatch]) {
        self.system.set_rom_read_patches(patches);
    }

    /// Observe count of cartridge reads served by enabled ROM substitutions.
    #[must_use]
    pub const fn rom_read_patch_hits(&self) -> u64 {
        self.system.rom_read_patch_hits()
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

    /// Side-effect-free read of one byte from canonical WRAM banks $7E/$7F.
    /// Useful for optional per-frame game-state diagnostics.
    #[must_use]
    pub fn peek_wram_u8(&self, address: u32) -> Option<u8> {
        self.system.peek_wram_u8(address)
    }

    /// Borrow the last values written by the 65816 to APU communication ports.
    ///
    /// These are the CPU-side bus mirrors; SPC700 CONTROL port clearing does
    /// not change them. See `apu_input_latch_ports` for what the SPC actually
    /// reads after independent input latch clearing.
    #[must_use]
    pub fn cpu_to_apu_ports(&self) -> &[u8] {
        self.system.cpu_to_apu_ports()
    }

    /// Current incoming mailbox bytes as read by the SPC700 sound CPU.
    ///
    /// Unlike the CPU write mirrors, these values reflect input-latch
    /// clearing by SPC CONTROL, even before the CPU writes another byte.
    #[must_use]
    pub fn apu_input_latch_ports(&self) -> [u8; 4] {
        std::array::from_fn(|port| self.apu.read_cpu_port(port).unwrap_or(0))
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

    /// Advance the optional sound-driver probe alongside DSP/timer clocks.
    ///
    /// A whole host CPU instruction can span several SPC instruction times.
    /// Clocking its timers up front and synthesizing its PCM only at the end
    /// lets uploaded sound drivers observe future timer events and changes
    /// the audible ordering of DSP register writes. Interleave at the existing
    /// six-master-clock bus quantum only for an explicitly enabled probe.
    /// The 65816 bus operations themselves remain instruction-atomic, so this
    /// is not a claim of exact CPU/SPC bus arbitration or DSP phase timing.
    fn advance_apu_for_cpu_step(&mut self, master_cycles: u64) -> Result<()> {
        if !self.apu.isolated_host_clocked_driver_enabled() {
            self.apu.step_master_cycles(master_cycles);
            self.append_audio_samples(master_cycles);
            return Ok(());
        }
        let mut remaining = master_cycles;
        while remaining > 0 {
            let clocks = remaining.min(CPU_BUS_CYCLE_MASTER_CYCLES);
            self.apu.step_master_cycles(clocks);
            self.apu.execute_host_clocked_spc(clocks)?;
            self.append_audio_samples(clocks);
            remaining -= clocks;
        }
        Ok(())
    }

    fn append_audio_samples(&mut self, master_cycles: u64) {
        // The partial BRR voice mixer produces samples at the fractional
        // DSP cadence. Until SPC700 upload/execution is wired to the mixer,
        // commercial-game audio stays silent, not a synthetic test tone.
        self.apu
            .append_dsp_audio(master_cycles, &mut self.pending_audio.samples);
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
    fn in_memory_quick_state_restores_identically_to_disk_json() {
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        rom[0] = 0xea;
        let cart = Cartridge::from_bytes(rom, None).unwrap();
        let mut original = Emulator::default();
        original.load_rom(cart);
        original.step_instruction().unwrap();

        let snapshot = original.capture_state();
        let disk_state = original.save_state().unwrap();
        let mut from_memory = Emulator::default();
        from_memory.restore_state(snapshot).unwrap();
        let mut from_disk = Emulator::default();
        from_disk.load_state(&disk_state).unwrap();

        assert_eq!(from_memory.save_state().unwrap(), disk_state);
        assert_eq!(from_disk.save_state().unwrap(), disk_state);
        assert_eq!(from_memory.cpu_registers(), from_disk.cpu_registers());
        assert_eq!(
            from_memory.timing().master_clock,
            from_disk.timing().master_clock
        );
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
    fn new_cartridge_disarms_previous_isolated_sound_driver_mode() {
        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom_bytes(), None).unwrap());
        emulator.enable_isolated_apu_driver_probe().unwrap();
        assert!(emulator.isolated_apu_driver_probe_enabled());
        assert!(emulator.isolated_apu_upload().is_some());

        emulator.load_rom(Cartridge::from_bytes(rom_bytes(), None).unwrap());
        assert!(!emulator.isolated_apu_driver_probe_enabled());
        assert!(emulator.isolated_apu_upload().is_none());
        assert!(emulator.apu_status().using_builtin_bootstrap);
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

        let samples = &emulator.audio_samples().samples;
        // One NTSC video frame has roughly 533 stereo sample pairs at
        // 32 kHz, not one pair for every CPU bus access.
        assert!(
            (1_050..=1_100).contains(&samples.len()),
            "unexpected sample count: {}",
            samples.len()
        );
        assert!(
            samples.iter().all(|&sample| sample == 0),
            "DSP not synthesized yet"
        );
        assert_eq!(emulator.host_read_u8(0x004218), 0x80);
        assert_eq!(emulator.host_read_u8(0x004219), 0x10);
    }

    #[test]
    fn reusable_cpu_trace_matches_observed_execution_during_gameplay() {
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        // LDA #$2A; STA $20; INC $20; BRA back to LDA.
        rom[..8].copy_from_slice(&[0xa9, 0x2a, 0x85, 0x20, 0xe6, 0x20, 0x80, 0xf8]);
        let cart = Cartridge::from_bytes(rom, None).unwrap();
        let mut normal = Emulator::default();
        normal.load_rom(cart);
        let mut observed = normal.clone();
        for _ in 0..48 {
            normal.step_instruction().unwrap();
            let events = observed.step_instruction_with_trace().unwrap();
            assert_eq!(normal.cpu_trace_scratch, events);
            assert_eq!(normal.cpu.registers, observed.cpu.registers);
            assert_eq!(normal.timing().master_clock, observed.timing().master_clock);
        }
        assert!(normal.cpu_trace_scratch.capacity() > 0);
        assert_eq!(normal.save_state().unwrap(), observed.save_state().unwrap());
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
    fn production_apu_timing_still_uses_original_single_host_quantum() {
        let mut emulator = Emulator::default();
        let mut reference = emulator.apu.clone();
        let mut expected_pcm = Vec::new();
        let clocks = 21_477_u64;
        reference.step_master_cycles(clocks);
        reference.append_dsp_audio(clocks, &mut expected_pcm);

        assert!(!emulator.apu.isolated_host_clocked_driver_enabled());
        emulator.advance_apu_for_cpu_step(clocks).unwrap();
        assert_eq!(emulator.pending_audio.samples, expected_pcm);
        assert_eq!(
            serde_json::to_value(&emulator.apu).unwrap(),
            serde_json::to_value(&reference).unwrap()
        );
    }

    #[test]
    fn opt_in_spc_timer_poll_receives_time_before_end_of_cpu_quantum() {
        let mut emulator = Emulator::default();
        emulator.apu.reset();
        emulator.apu.enable_isolated_ipl_upload();
        emulator.apu.enable_isolated_host_clocked_driver().unwrap();

        // Timer2 target one, enable timer2, poll its read-and-clear $FF,
        // and acknowledge the first nonzero tick in SPC output port $F4.
        // If all 2,000 master cycles advance *before* the SPC instructions,
        // the timer is still disabled at that time and no tick can arrive.
        let sound_program = [
            0x8f, 1, 0xfc, 0x8f, 4, 0xf1, 0xe4, 0xff, 0xf0, 0xfc, 0xc4, 0xf4, 0x2f, 0xfe,
        ];
        for (port, value) in [(1, 1), (2, 0), (3, 4), (0, 0xcc)] {
            emulator.apu.write_cpu_port(port, value).unwrap();
        }
        emulator.apu.step_master_cycles(6);
        for (index, byte) in sound_program.into_iter().enumerate() {
            emulator.apu.write_cpu_port(1, byte).unwrap();
            emulator.apu.write_cpu_port(0, index as u8).unwrap();
            emulator.apu.step_master_cycles(6);
        }
        for (port, value) in [(1, 0), (2, 0), (3, 4), (0, 15)] {
            emulator.apu.write_cpu_port(port, value).unwrap();
        }
        emulator.apu.step_master_cycles(6);
        assert_eq!(
            emulator.isolated_apu_upload().unwrap().entrypoint(),
            Some(0x0400)
        );

        // Independent reference performs the six-clock APU sequence by
        // hand. The emulator helper must agree on exact serialized state,
        // stereo sample bytes, timer output, and instruction count.
        let initial_apu = emulator.apu.clone();
        let mut reference = initial_apu.clone();
        let mut expected_pcm = Vec::new();
        for _ in 0..333 {
            reference.step_master_cycles(6);
            reference.execute_host_clocked_spc(6).unwrap();
            reference.append_dsp_audio(6, &mut expected_pcm);
        }
        reference.step_master_cycles(2);
        reference.execute_host_clocked_spc(2).unwrap();
        reference.append_dsp_audio(2, &mut expected_pcm);

        emulator.advance_apu_for_cpu_step(2_000).unwrap();
        assert_eq!(emulator.apu.read_apu_port(0).unwrap(), 1);
        assert_eq!(emulator.pending_audio.samples, expected_pcm);
        assert_eq!(
            serde_json::to_value(&emulator.apu).unwrap(),
            serde_json::to_value(&reference).unwrap()
        );
        assert!(emulator.isolated_apu_spc700_status().unwrap().instructions > 4);

        // Splitting at an exact six-master-clock boundary and restoring
        // the sound subsystem midway must keep timer/driver/sample phases.
        let mut partial = Emulator::default();
        partial.apu = initial_apu;
        partial.advance_apu_for_cpu_step(1_002).unwrap();
        let snapshot = serde_json::to_string(&partial.apu).unwrap();
        let mut restored = Emulator::default();
        restored.apu = serde_json::from_str(&snapshot).unwrap();
        restored.advance_apu_for_cpu_step(998).unwrap();
        let mut combined_pcm = partial.pending_audio.samples;
        combined_pcm.extend_from_slice(&restored.pending_audio.samples);
        assert_eq!(combined_pcm, emulator.pending_audio.samples);
        assert_eq!(
            serde_json::to_value(&restored.apu).unwrap(),
            serde_json::to_value(&emulator.apu).unwrap()
        );
    }

    #[test]
    fn pending_cpu_apu_port_write_is_not_lost_across_emulator_save_state() {
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        rom[0] = 0xea; // NOP; the CPU never explicitly writes a port.
        rom[1] = 0xea; // A second idle instruction after the SPC clear.
        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());

        // A frontend or DMA source may write a port between 65816 steps.
        // Saving before the next step must preserve its pending write event.
        emulator.host_write_u8(0x002142, 0x5a);
        let checkpoint = emulator.save_state().unwrap();
        let mut restored = Emulator::default();
        restored.load_state(&checkpoint).unwrap();
        assert_eq!(restored.apu_input_latch_ports()[2], 0);

        emulator.step_instruction().unwrap();
        restored.step_instruction().unwrap();
        assert_eq!(emulator.apu_input_latch_ports()[2], 0x5a);
        assert_eq!(restored.apu_input_latch_ports()[2], 0x5a);

        // On another idle instruction, clearing by SPC must persist; a
        // stale last-written 65816 mirror is not a new write event.
        restored.apu.write_spc_bus(0x00f1, 0x20);
        restored.step_instruction().unwrap();
        assert_eq!(restored.apu_input_latch_ports()[2], 0);
        assert_eq!(restored.cpu_to_apu_ports()[2], 0x5a);
    }

    #[test]
    fn cpu_port_clear_survives_unrelated_65816_instructions_and_repeated_same_value() {
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        // LDA #$66; STA $2140; NOP; LDA #$66; STA $2140; NOP.
        rom[..11].copy_from_slice(&[
            0xa9, 0x66, 0x8d, 0x40, 0x21, 0xea, 0xa9, 0x66, 0x8d, 0x40, 0x21,
        ]);
        rom[11] = 0xea;
        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());
        emulator.step_instruction().unwrap();
        emulator.step_instruction().unwrap();
        assert_eq!(emulator.apu.read_cpu_port(0).unwrap(), 0x66);

        // The sound CPU clears the incoming mailbox. Merely executing a
        // 65816 NOP must never resurrect its earlier $2140 write.
        emulator.apu.write_spc_bus(0x00f1, 0x10);
        assert_eq!(emulator.apu.read_cpu_port(0).unwrap(), 0);
        assert_eq!(emulator.apu_input_latch_ports()[0], 0);
        assert_eq!(emulator.cpu_to_apu_ports()[0], 0x66);
        let checkpoint = emulator.save_state().unwrap();
        let mut restored = Emulator::default();
        restored.load_state(&checkpoint).unwrap();
        emulator.step_instruction().unwrap();
        restored.step_instruction().unwrap();
        assert_eq!(emulator.apu.read_cpu_port(0).unwrap(), 0);
        assert_eq!(restored.apu.read_cpu_port(0).unwrap(), 0);

        // An explicit second CPU write of the same byte *must* be observed.
        for _ in 0..2 {
            emulator.step_instruction().unwrap();
            restored.step_instruction().unwrap();
        }
        assert_eq!(emulator.apu.read_cpu_port(0).unwrap(), 0x66);
        assert_eq!(restored.apu.read_cpu_port(0).unwrap(), 0x66);
        assert_eq!(
            serde_json::to_value(&emulator.apu).unwrap(),
            serde_json::to_value(&restored.apu).unwrap()
        );
        assert_eq!(
            emulator.system_observability().apu_port_write_counts()[0],
            2
        );
    }

    #[test]
    fn cpu_program_uploads_spc_driver_through_real_mailbox_bus_and_runs_it() {
        use crate::apu::spc700::Spc700HaltState;

        fn write_port(code: &mut Vec<u8>, instructions: &mut usize, port: u8, value: u8) {
            // LDA #imm; STA $214x
            code.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
            *instructions += 2;
        }

        fn capture_ack(code: &mut Vec<u8>, instructions: &mut usize, slot: u8) {
            // LDA $2140; STA $0010+slot, using canonical low WRAM.
            code.extend_from_slice(&[0xad, 0x40, 0x21, 0x8d, 0x10 + slot, 0x00]);
            *instructions += 2;
        }

        let mut rom = rom_bytes();
        rom[0x7ffc] = 0x00;
        rom[0x7ffd] = 0x80;
        // Original 65816 ROM: exercise both directions of the actual
        // CPU -> SystemBus -> APU mailbox, not host-side helper writes.
        let mut program = Vec::new();
        let mut instructions = 0;
        write_port(&mut program, &mut instructions, 1, 1);
        write_port(&mut program, &mut instructions, 2, 0);
        write_port(&mut program, &mut instructions, 3, 4);
        write_port(&mut program, &mut instructions, 0, 0xcc);
        capture_ack(&mut program, &mut instructions, 0);

        // SPC700 program at $0400: MOV $F4,#$42; SLEEP.
        let sound_program = [0x8f, 0x42, 0xf4, 0xef];
        for (index, byte) in sound_program.into_iter().enumerate() {
            write_port(&mut program, &mut instructions, 1, byte);
            write_port(&mut program, &mut instructions, 0, index as u8);
            capture_ack(&mut program, &mut instructions, (index + 1) as u8);
        }
        write_port(&mut program, &mut instructions, 1, 0);
        write_port(&mut program, &mut instructions, 2, 0);
        write_port(&mut program, &mut instructions, 3, 4);
        write_port(&mut program, &mut instructions, 0, 5);
        capture_ack(&mut program, &mut instructions, 5);
        rom[..program.len()].copy_from_slice(&program);
        let cart = Cartridge::from_bytes(rom, None).unwrap();

        let mut emulator = Emulator::default();
        emulator.load_rom(cart);
        emulator.enable_isolated_apu_upload_probe();
        assert_eq!(emulator.apu_to_cpu_ports()[0], 0xaa);
        for _ in 0..instructions {
            emulator.step_instruction().unwrap();
        }
        for (slot, expected) in [0xcc_u8, 0, 1, 2, 3, 5].into_iter().enumerate() {
            assert_eq!(
                emulator.peek_wram_u8(0x7e0010 + slot as u32),
                Some(expected),
                "65816 readback of APU acknowledgement {slot}"
            );
        }
        assert_eq!(emulator.cpu_registers().pc, 0x8000 + program.len() as u16);

        let upload = emulator.apu.isolated_ipl_upload().unwrap();
        assert_eq!(upload.bytes_written(), sound_program.len() as u64);
        assert_eq!(upload.blocks_finished(), 1);
        assert_eq!(upload.entrypoint(), Some(0x0400));
        for (index, byte) in sound_program.into_iter().enumerate() {
            assert_eq!(emulator.apu.read_spc_ram(0x0400 + index as u16), byte);
        }
        assert_eq!(emulator.apu.spc700.pc, 0x0400);
        let status = emulator.isolated_apu_spc700_status().unwrap();
        assert_eq!(status.pc, 0x0400);
        assert_eq!(status.instructions, 0);
        assert_eq!(status.cycles, 0);
        assert_eq!(emulator.host_read_u8(0x002140), 5);

        let mut audio = Vec::new();
        assert_eq!(
            emulator
                .apu
                .run_isolated_uploaded_spc_program(20, &mut audio)
                .unwrap(),
            2
        );
        assert_eq!(emulator.apu.read_apu_port(0).unwrap(), 0x42);
        assert_eq!(emulator.apu.spc700.halt_state(), Spc700HaltState::Sleeping);
        assert_eq!(emulator.apu.spc700.pc, 0x0404);
        let status = emulator.isolated_apu_spc700_status().unwrap();
        assert_eq!(status.pc, 0x0404);
        assert_eq!(status.instructions, 2);
        assert_eq!(status.halt_state, Spc700HaltState::Sleeping);

        // Opt-in capture was never enabled in the default emulator.
        assert!(Emulator::default().apu.isolated_ipl_upload().is_none());
    }

    #[test]
    fn cpu_ipl_upload_to_dsp_registers_updates_apu_and_survives_snapshot() {
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        fn append_port_write(program: &mut Vec<u8>, port: u8, value: u8) {
            program.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
        }

        let mut program = Vec::new();
        // Original 65816 instructions drive the real $2140-$2143 bus,
        // then use the IPL to write SPC DSPADDR followed by DSPDATA.
        append_port_write(&mut program, 1, 1);
        append_port_write(&mut program, 2, 0xf2);
        append_port_write(&mut program, 3, 0);
        append_port_write(&mut program, 0, 0xcc);
        append_port_write(&mut program, 1, 0x0c); // Master volume left register.
        append_port_write(&mut program, 0, 0); // Upload first byte at SPC $F2.
        let before_second_byte = program.len() / 5 * 2;
        append_port_write(&mut program, 1, 0x55);
        append_port_write(&mut program, 0, 1); // Upload second byte at SPC $F3.
        let total_instructions = program.len() / 5 * 2;
        rom[..program.len()].copy_from_slice(&program);

        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());
        emulator.enable_isolated_apu_upload_probe();
        for _ in 0..before_second_byte {
            emulator.step_instruction().unwrap();
        }
        assert_eq!(emulator.apu.read_spc_ram(0x00f2), 0x0c);
        assert_eq!(emulator.apu.read_dsp_register(0x0c), 0);
        assert_eq!(emulator.isolated_apu_dsp_register(0x0c), Some(0));
        assert_eq!(emulator.isolated_apu_upload().unwrap().bytes_written(), 1);

        let state = emulator.save_state().unwrap();
        let mut restored = Emulator::default();
        restored.load_state(&state).unwrap();
        for _ in before_second_byte..total_instructions {
            emulator.step_instruction().unwrap();
            restored.step_instruction().unwrap();
        }
        for machine in [&emulator, &restored] {
            assert_eq!(machine.apu.read_spc_ram(0x00f2), 0x0c);
            assert_eq!(machine.apu.read_spc_ram(0x00f3), 0x55);
            assert_eq!(machine.apu.read_dsp_register(0x0c), 0x55);
            assert_eq!(machine.isolated_apu_dsp_register(0x0c), Some(0x55));
            assert_eq!(machine.isolated_apu_upload().unwrap().bytes_written(), 2);
            assert_eq!(machine.apu_to_cpu_ports()[0], 1);
        }
        assert_eq!(
            serde_json::to_value(&emulator.apu).unwrap(),
            serde_json::to_value(&restored.apu).unwrap()
        );
        assert!(Emulator::default().apu.isolated_ipl_upload().is_none());
        assert_eq!(Emulator::default().isolated_apu_dsp_register(0x0c), None);
        assert_eq!(Emulator::default().isolated_apu_test_latch(), None);
        let mut probe = Emulator::default();
        probe.enable_isolated_apu_upload_probe();
        assert_eq!(probe.isolated_apu_test_latch(), Some(0x0a));
        probe.apu.write_spc_bus(0x00f0, 0);
        assert_eq!(probe.isolated_apu_test_latch(), Some(0));
    }

    #[test]
    fn synthetic_cpu_uploads_three_spc_blocks_and_runs_dsp_audio() {
        let mut sound_driver = Vec::new();
        for (reg, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
            (0x4c, 0x01),
        ] {
            sound_driver.extend_from_slice(&[0x8f, reg, 0xf2, 0x8f, value, 0xf3]);
        }
        sound_driver.extend_from_slice(&[0x2f, 0xfe]); // Bounded BRA loop.

        let mut cpu_code = Vec::new();
        let mut write_port = |port: u8, value: u8| {
            cpu_code.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
        };
        // IPL handshake, first block at $0200: BRR directory entry.
        write_port(1, 1);
        write_port(2, 0);
        write_port(3, 2);
        write_port(0, 0xcc);
        for (index, byte) in [0, 3, 0, 3].into_iter().enumerate() {
            write_port(1, byte);
            write_port(0, index as u8);
        }
        // Next block at $0300: original loopable BRR sample.
        write_port(1, 1);
        write_port(2, 0);
        write_port(3, 3);
        write_port(0, 5);
        for (index, byte) in [0xc3, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77]
            .into_iter()
            .enumerate()
        {
            write_port(1, byte);
            write_port(0, index as u8);
        }
        // Third block at $0400: sound driver using $F2/$F3 DSP MMIO.
        write_port(1, 1);
        write_port(2, 0);
        write_port(3, 4);
        write_port(0, 10);
        for (index, byte) in sound_driver.iter().copied().enumerate() {
            write_port(1, byte);
            write_port(0, index as u8);
        }
        write_port(1, 0);
        write_port(2, 0);
        write_port(3, 4);
        write_port(0, sound_driver.len() as u8 + 1);

        let instruction_count = cpu_code.len() / 5 * 2;
        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        rom[..cpu_code.len()].copy_from_slice(&cpu_code);
        let mut host_clocked_rom = rom.clone();
        // Stable no-op CPU loop after IPL handoff while sound CPU executes.
        host_clocked_rom[cpu_code.len()..cpu_code.len() + 2].copy_from_slice(&[0x80, 0xfe]);
        let mut emulator = Emulator::default();
        emulator.load_rom(Cartridge::from_bytes(rom, None).unwrap());
        emulator.enable_isolated_apu_upload_probe();
        let halfway = instruction_count / 2;
        for _ in 0..halfway {
            emulator.step_instruction().unwrap();
        }
        let partial_upload = emulator.isolated_apu_upload().unwrap();
        assert!(partial_upload.bytes_written() > 0);
        assert!(partial_upload.entrypoint().is_none());

        // Resume a save-state taken while the 65816 is still delivering
        // sound-program bytes, not after the IPL has completed its handoff.
        let snapshot = emulator.save_state().unwrap();
        let mut resumed = Emulator::default();
        resumed.load_state(&snapshot).unwrap();
        for _ in halfway..instruction_count {
            emulator.step_instruction().unwrap();
            resumed.step_instruction().unwrap();
        }
        let upload = emulator.isolated_apu_upload().unwrap();
        assert_eq!(upload.blocks_finished(), 3);
        assert_eq!(upload.bytes_written(), (4 + 9 + sound_driver.len()) as u64);
        assert_eq!(upload.entrypoint(), Some(0x0400));
        assert_eq!(
            resumed.isolated_apu_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        assert_eq!(
            resumed.isolated_apu_upload().unwrap().bytes_written(),
            upload.bytes_written()
        );
        assert_eq!(emulator.apu.spc700.pc, 0x0400);
        assert_eq!(emulator.apu.read_spc_ram(0x0200), 0);
        assert_eq!(emulator.apu.read_spc_ram(0x0201), 3);
        assert_eq!(emulator.apu.read_spc_ram(0x0300), 0xc3);
        for (index, byte) in sound_driver.iter().copied().enumerate() {
            assert_eq!(emulator.apu.read_spc_ram(0x0400 + index as u16), byte);
        }

        let mut samples = Vec::new();
        assert_eq!(
            emulator
                .apu
                .run_isolated_uploaded_spc_program(400, &mut samples)
                .unwrap(),
            400
        );
        let mut resumed_samples = Vec::new();
        assert_eq!(
            resumed
                .apu
                .run_isolated_uploaded_spc_program(400, &mut resumed_samples)
                .unwrap(),
            400
        );
        assert_eq!(resumed_samples, samples);
        assert_eq!(
            serde_json::to_value(&resumed.apu).unwrap(),
            serde_json::to_value(&emulator.apu).unwrap()
        );
        assert_eq!(emulator.apu.read_dsp_register(0x5d), 2);
        assert_eq!(emulator.apu.read_dsp_register(0x4c), 1);
        assert!(samples.len() >= 30);
        assert!(samples.iter().any(|sample| *sample > 1_000));
        assert!(samples.chunks_exact(2).all(|pair| pair[0] == pair[1]));

        // The exact same host-side upload can optionally drive SPC execution
        // on CPU master clocks; production mode never enables this.
        let mut host_clocked = Emulator::default();
        host_clocked.load_rom(Cartridge::from_bytes(host_clocked_rom, None).unwrap());
        host_clocked.enable_isolated_apu_driver_probe().unwrap();
        for _ in 0..instruction_count {
            host_clocked.step_instruction().unwrap();
        }
        assert_eq!(
            host_clocked.isolated_apu_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        for _ in 0..10_000 {
            host_clocked.step_instruction().unwrap();
        }
        let clocked_status = host_clocked.isolated_apu_spc700_status().unwrap();
        assert!(clocked_status.instructions > 100);
        assert_eq!(host_clocked.apu.read_dsp_register(0x4c), 1);
        assert!(
            host_clocked
                .audio_samples()
                .samples
                .iter()
                .any(|v| *v > 1000)
        );
        assert!(
            host_clocked
                .apu
                .run_isolated_uploaded_spc_program(1, &mut Vec::new())
                .is_err()
        );
    }

    #[test]
    fn emulated_65816_can_restart_spc_ipl_and_upload_a_second_sound_program() {
        // Every port transaction below is executed by a synthetic 65816
        // cartridge through $2140-$2143; no host-side mailbox writes.
        fn write_port(code: &mut Vec<u8>, port: u8, value: u8) {
            code.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
        }

        fn upload(code: &mut Vec<u8>, program: &[u8]) {
            write_port(code, 1, 1);
            write_port(code, 2, 0);
            write_port(code, 3, 4);
            write_port(code, 0, 0xcc);
            for (index, &byte) in program.iter().enumerate() {
                write_port(code, 1, byte);
                write_port(code, 0, index as u8);
            }
            write_port(code, 1, 0);
            write_port(code, 2, 0);
            write_port(code, 3, 4);
            write_port(code, 0, program.len() as u8 + 1);
        }

        let first_driver = [0x5f, 0xc0, 0xff]; // SPC700 JMP $FFC0.
        let second_driver = [0x8f, 0x42, 0xf4, 0xef]; // MOV $F4,#$42; SLEEP.
        let mut code = Vec::new();
        upload(&mut code, &first_driver);
        let first_upload_instructions = (code.len() / 5) * 2;
        // Give the host-clocked SPC700 time to reenter the IPL; ordinary
        // 65816 NOPs are intentionally part of the cartridge's program.
        code.extend(std::iter::repeat_n(0xea, 48));
        let before_second = code.len();
        // Capture the second IPL's $AA readiness via a real CPU bus read.
        code.extend_from_slice(&[0xad, 0x40, 0x21, 0x8d, 0x10, 0x00]);
        upload(&mut code, &second_driver);
        let second_upload_instructions = 2 + ((code.len() - before_second - 6) / 5) * 2;
        code.extend_from_slice(&[0x80, 0xfe]); // Stable 65816 idle loop.

        let mut rom = rom_bytes();
        rom[0x7ffc] = 0;
        rom[0x7ffd] = 0x80;
        rom[..code.len()].copy_from_slice(&code);
        let mut original = Emulator::default();
        original.load_rom(Cartridge::from_bytes(rom, None).unwrap());
        original.enable_isolated_apu_driver_probe().unwrap();
        // Pre-existing BRR/audio sample RAM survives a full $FFC0 restart.
        original.apu.write_spc_ram(0x2300, 0x7a);

        for _ in 0..first_upload_instructions {
            original.step_instruction().unwrap();
        }
        for _ in 0..48 {
            original.step_instruction().unwrap();
        }
        assert_eq!(original.cpu_registers().pc, 0x8000 + before_second as u16);
        assert_eq!(original.isolated_apu_upload().unwrap().entrypoint(), None);
        assert_eq!(original.apu_to_cpu_ports()[0], 0xaa);
        assert_eq!(original.apu_to_cpu_ports()[1], 0xbb);
        assert_eq!(original.apu.read_spc_ram(0x2300), 0x7a);

        // Snapshot with the second boot IPL ready but before its host-side
        // upload, then compare deterministic CPU/APU continuation.
        let snapshot = original.save_state().unwrap();
        let mut resumed = Emulator::default();
        resumed.enable_isolated_apu_driver_probe().unwrap();
        resumed.load_state(&snapshot).unwrap();
        assert!(resumed.isolated_apu_driver_probe_enabled());

        for _ in 0..second_upload_instructions {
            original.step_instruction().unwrap();
            resumed.step_instruction().unwrap();
        }
        for instance in [&original, &resumed] {
            assert_eq!(instance.peek_wram_u8(0x7e0010), Some(0xaa));
            assert_eq!(
                instance.isolated_apu_upload().unwrap().entrypoint(),
                Some(0x0400)
            );
            assert_eq!(
                instance.isolated_apu_upload().unwrap().bytes_written(),
                second_driver.len() as u64
            );
            assert_eq!(instance.apu.read_spc_ram(0x2300), 0x7a);
            for (index, &byte) in second_driver.iter().enumerate() {
                assert_eq!(instance.apu.read_spc_ram(0x0400 + index as u16), byte);
            }
        }

        for _ in 0..100 {
            original.step_instruction().unwrap();
            resumed.step_instruction().unwrap();
        }
        assert_eq!(original.apu.read_apu_port(0).unwrap(), 0x42);
        assert_eq!(resumed.apu.read_apu_port(0).unwrap(), 0x42);
        assert!(
            original.isolated_apu_spc700_status().unwrap().instructions >= 2,
            "the second uploaded driver must execute actual instructions"
        );
        assert_eq!(
            serde_json::to_value(&original.apu).unwrap(),
            serde_json::to_value(&resumed.apu).unwrap()
        );
    }

    #[test]
    fn loading_diagnostic_state_cannot_enable_host_sound_without_explicit_opt_in() {
        let mut diagnostic = Emulator::default();
        diagnostic.load_rom(Cartridge::from_bytes(rom_bytes(), None).unwrap());
        diagnostic.enable_isolated_apu_driver_probe().unwrap();
        assert!(diagnostic.apu.isolated_host_clocked_driver_enabled());
        let saved = diagnostic.save_state().unwrap();

        let mut ordinary = Emulator::default();
        ordinary.load_state(&saved).unwrap();
        assert!(!ordinary.apu.isolated_host_clocked_driver_enabled());
        assert!(ordinary.isolated_apu_upload().is_some());

        let mut opted_in = Emulator::default();
        opted_in.enable_isolated_apu_driver_probe().unwrap();
        opted_in.load_state(&saved).unwrap();
        assert!(opted_in.apu.isolated_host_clocked_driver_enabled());
    }

    #[test]
    fn rejects_unknown_save_state_version() {
        let state = r#"{"version":999,"cpu":{"registers":{"a":0,"x":0,"y":0,"s":0,"d":0,"pc":0,"pbr":0,"dbr":0,"p":0,"emulation":false},"cycles":0},"apu":{"spc700":{"pc":0,"a":0,"x":0,"y":0,"sp":0,"psw":0,"cycles":0},"cpu_to_apu_ports":[0,0,0,0],"apu_to_cpu_ports":[0,0,0,0],"ipl_rom":null,"configured_ipl_path":null,"spc700_steps":0},"system":{"cartridge":null,"save_ram":[],"wram":[],"ppu":{"registers":[],"cgram":[],"cgram_address":0,"cgram_high_byte":false},"cpu_to_apu_io":[0,0,0,0],"apu_to_cpu_io":[0,0,0,0],"dma":{"channels":[{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false},{"control":0,"b_bus_address":0,"a_bus_address":0,"a_bus_bank":0,"byte_count":0,"indirect_address":0,"hdma_table_address":0,"hdma_data_address":0,"hdma_line_counter":0,"hdma_active":false,"hdma_repeat":false}],"dma_enable_mask":0,"hdma_enable_mask":0,"transfer_count":0},"timing":{"master_clock":0,"scanline":0,"dot":0,"frame":0},"open_bus":0,"nmitimen":0,"rdnmi":false,"timeup":false,"joypad":{"controller1":{"b":false,"y":false,"select":false,"start":false,"up":false,"down":false,"left":false,"right":false,"a":false,"x":false,"l":false,"r":false},"latch_line":false,"latched1":0,"shift1":0},"wram_address":0}}"#;
        let mut emulator = Emulator::default();
        assert!(emulator.load_state(state).is_err());
    }
}
