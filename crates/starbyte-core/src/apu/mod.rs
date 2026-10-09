//! Audio processing unit bootstrap boundary.

pub mod dsp;
pub mod spc700;

use std::{cell::RefCell, path::PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::error::{Error, Result};

use self::{dsp::Dsp, spc700::Spc700};

/// Size of the user-supplied SPC700 IPL ROM.
pub const SPC700_IPL_ROM_LEN: usize = 64;

/// NTSC master-clock frequency in Hz. See `TimingState` for frame/dot clocks.
const NTSC_MASTER_CLOCK_HZ: u64 = 21_477_272;
/// SNES DSP produces 32,000 stereo sample pairs per second.
pub const DSP_SAMPLE_RATE_HZ: u64 = 32_000;
const SPC_RAM_BYTES: usize = 65_536;

fn blank_spc_ram() -> Vec<u8> {
    vec![0; SPC_RAM_BYTES]
}

/// Buffered audio samples returned to a frontend.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    /// Interleaved stereo 16-bit samples.
    pub samples: Vec<i16>,
}

/// Snapshot of APU bootstrap status surfaced to the emulator and CLI layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApuStatus {
    /// Whether a user-supplied IPL ROM is currently loaded.
    pub has_ipl_rom: bool,
    /// Whether the emulator is using the in-tree bootstrap fallback instead of external firmware.
    pub using_builtin_bootstrap: bool,
    /// Configured firmware path if any.
    pub configured_ipl_path: Option<PathBuf>,
    /// Total SPC700 bootstrap steps executed through the APU boundary.
    pub spc700_steps: u64,
}

/// Minimal APU wrapper used to establish timing and communication boundaries early.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Apu {
    /// SPC700 core state.
    pub spc700: Spc700,
    cpu_to_apu_ports: [u8; 4],
    apu_to_cpu_ports: [u8; 4],
    ipl_rom: Option<Vec<u8>>,
    configured_ipl_path: Option<PathBuf>,
    spc700_steps: u64,
    #[serde(default)]
    bootstrap_state: BootstrapState,
    /// Fractional numerator of the DSP sample clock; serialized for save-state continuity.
    #[serde(default)]
    dsp_sample_phase: u64,
    /// Independent SPC700 address space (not main CPU WRAM).
    #[serde(default = "blank_spc_ram")]
    spc_ram: Vec<u8>,
    /// Partial SNES DSP synthesis engine and voice/register state.
    #[serde(default)]
    dsp: Dsp,
}

impl Default for Apu {
    fn default() -> Self {
        Self {
            spc700: Spc700::default(),
            cpu_to_apu_ports: [0; 4],
            apu_to_cpu_ports: [0; 4],
            ipl_rom: None,
            configured_ipl_path: None,
            spc700_steps: 0,
            bootstrap_state: BootstrapState::default(),
            dsp_sample_phase: 0,
            spc_ram: blank_spc_ram(),
            dsp: Dsp::default(),
        }
    }
}

impl Apu {
    /// Create an APU boundary configured with an optional firmware path.
    #[must_use]
    pub fn with_ipl_path(path: Option<PathBuf>) -> Self {
        Self {
            configured_ipl_path: path,
            ..Self::default()
        }
    }

    /// Reset APU-visible state while preserving configured firmware.
    pub fn reset(&mut self) {
        self.spc700.reset();
        self.cpu_to_apu_ports = [0; 4];
        self.apu_to_cpu_ports = if self.bootstrap_program_available() {
            [0xAA, 0xBB, 0x00, 0x00]
        } else {
            [0; 4]
        };
        self.spc700_steps = 0;
        self.dsp_sample_phase = 0;
        self.spc_ram.resize(SPC_RAM_BYTES, 0);
        self.spc_ram.fill(0);
        self.dsp.reset();
        self.bootstrap_state = if self.bootstrap_program_available() {
            BootstrapState::WaitingForCpuBootstrapAck
        } else {
            BootstrapState::Idle
        };
    }

    /// Configure or replace the path to a user-supplied IPL ROM.
    pub fn set_ipl_path(&mut self, path: Option<PathBuf>) {
        self.configured_ipl_path = path;
        self.ipl_rom = None;
    }

    /// Load the configured user-supplied IPL ROM if a path is present.
    #[instrument(skip_all)]
    pub fn load_configured_ipl_rom(&mut self) -> Result<bool> {
        let Some(path) = self.configured_ipl_path.clone() else {
            self.ipl_rom = None;
            return Ok(false);
        };

        let data = std::fs::read(&path).map_err(|source| Error::io(&path, source))?;
        self.install_ipl_rom_bytes(data, Some(path))?;
        Ok(true)
    }

    /// Install a user-supplied IPL ROM from owned bytes.
    pub fn install_ipl_rom_bytes(&mut self, data: Vec<u8>, source: Option<PathBuf>) -> Result<()> {
        if data.len() != SPC700_IPL_ROM_LEN {
            return Err(Error::InvalidFirmware {
                name: "SPC700 IPL ROM",
                details: format!(
                    "expected {SPC700_IPL_ROM_LEN} bytes but received {} bytes",
                    data.len()
                ),
            });
        }

        debug!(path = ?source, "loaded user-supplied SPC700 IPL ROM");
        self.ipl_rom = Some(data);
        if source.is_some() {
            self.configured_ipl_path = source;
        }
        Ok(())
    }

    /// Step the SPC700 core once through the APU boundary.
    pub fn step_spc700(&mut self) {
        self.spc700.step();
        self.spc700_steps = self.spc700_steps.saturating_add(1);
    }

    /// Execute a single SPC700 instruction against its own RAM and MMIO bus.
    ///
    /// This path is available for isolated sound-driver regression tests.
    /// Full cartridge runtime still uses the historical bootstrap boundary
    /// until uploaded game sound drivers and missing SPC700 opcodes are ready.
    pub fn execute_spc_program_instruction(&mut self) -> Result<Vec<crate::bus::BusEvent>> {
        // The CPU needs independent read/write callbacks to the same bus.
        // Temporarily move it out so the callbacks can safely borrow the APU.
        let mut cpu = std::mem::take(&mut self.spc700);
        let bus = RefCell::new(&mut *self);
        let result = cpu.step_with_memory(
            |address| bus.borrow().read_spc_bus(address),
            |address, value| bus.borrow_mut().write_spc_bus(address, value),
        );
        let mut apu = bus.borrow_mut();
        apu.spc700 = cpu;
        if result.is_ok() {
            apu.spc700_steps = apu.spc700_steps.saturating_add(1);
        }
        result
    }

    /// Advance placeholder APU work for a number of master cycles.
    pub fn step_master_cycles(&mut self, master_cycles: u64) {
        // The exact divider will be replaced when full system timing is modeled.
        for _ in 0..(master_cycles / 6) {
            self.step_spc700();
        }
        self.advance_bootstrap_handshake();
    }

    /// Return the number of SNES DSP sample pairs due for this many NTSC master clocks.
    ///
    /// The DSP itself is not yet emulated, so the current caller emits silence.
    /// Retaining the fractional remainder avoids drift across CPU instructions,
    /// frames, and save-state restoration. This does not run the SPC700.
    pub fn advance_dsp_sample_clock(&mut self, master_cycles: u64) -> usize {
        let numerator = u128::from(self.dsp_sample_phase)
            + u128::from(master_cycles) * u128::from(DSP_SAMPLE_RATE_HZ);
        let denominator = u128::from(NTSC_MASTER_CLOCK_HZ);
        self.dsp_sample_phase = (numerator % denominator) as u64;
        usize::try_from(numerator / denominator).expect("DSP sample count exceeds platform usize")
    }

    /// Write SPC700 RAM (separate from the SNES main CPU address space).
    pub fn write_spc_ram(&mut self, address: u16, value: u8) {
        self.spc_ram[usize::from(address)] = value;
    }

    /// Read SPC700 RAM, including user-uploaded BRR samples.
    #[must_use]
    pub fn read_spc_ram(&self, address: u16) -> u8 {
        self.spc_ram[usize::from(address)]
    }

    /// Read from the SPC700 address space, including DSP and CPU mailbox ports.
    ///
    /// This exposes the essential sound-CPU data path without enabling
    /// execution of incomplete SPC700 sound drivers. Timers, IPL overlay,
    /// and general SPC700 instruction timing remain separate work.
    #[must_use]
    pub fn read_spc_bus(&self, address: u16) -> u8 {
        match address {
            0x00f3 => self.dsp.read_register(self.spc_ram[0x00f2]),
            0x00f4..=0x00f7 => self.cpu_to_apu_ports[usize::from(address - 0x00f4)],
            _ => self.read_spc_ram(address),
        }
    }

    /// Write the SPC700 RAM/MMIO bus, including indirect DSP register access.
    pub fn write_spc_bus(&mut self, address: u16, value: u8) {
        match address {
            0x00f2 => self.spc_ram[0x00f2] = value & 0x7f,
            0x00f3 => self
                .dsp
                .write_register(self.spc_ram[0x00f2], value, &self.spc_ram),
            0x00f4..=0x00f7 => {
                self.apu_to_cpu_ports[usize::from(address - 0x00f4)] = value;
            }
            _ => self.write_spc_ram(address, value),
        }
    }

    /// Write an S-DSP register directly (normally through SPC700 $F2/$F3).

    pub fn write_dsp_register(&mut self, register: u8, value: u8) {
        self.dsp.write_register(register, value, &self.spc_ram);
    }

    /// Read the current register value from the partial S-DSP model.
    #[must_use]
    pub fn read_dsp_register(&self, register: u8) -> u8 {
        self.dsp.read_register(register)
    }

    /// Append DSP-paced interleaved stereo output into one frame's buffer.
    ///
    /// Current real-game output remains silent until SPC700 sound-program
    /// execution and DSP register writes are connected to the runtime.
    pub fn append_dsp_audio(&mut self, master_cycles: u64, output: &mut Vec<i16>) {
        let pairs = self.advance_dsp_sample_clock(master_cycles);
        for _ in 0..pairs {
            let (left, right) = self.dsp.next_stereo_pair(&self.spc_ram);
            output.push(left);
            output.push(right);
        }
    }

    /// Write one CPU-to-APU communication port byte.
    pub fn write_cpu_port(&mut self, port: usize, value: u8) -> Result<()> {
        let Some(slot) = self.cpu_to_apu_ports.get_mut(port) else {
            return Err(Error::Unimplemented("APU port index out of range"));
        };
        *slot = value;
        Ok(())
    }

    /// Read one CPU-to-APU communication port byte.
    pub fn read_cpu_port(&self, port: usize) -> Result<u8> {
        self.cpu_to_apu_ports
            .get(port)
            .copied()
            .ok_or(Error::Unimplemented("APU port index out of range"))
    }

    /// Write one APU-to-CPU communication port byte.
    pub fn write_apu_port(&mut self, port: usize, value: u8) -> Result<()> {
        let Some(slot) = self.apu_to_cpu_ports.get_mut(port) else {
            return Err(Error::Unimplemented("APU port index out of range"));
        };
        *slot = value;
        Ok(())
    }

    /// Read one APU-to-CPU communication port byte.
    pub fn read_apu_port(&self, port: usize) -> Result<u8> {
        self.apu_to_cpu_ports
            .get(port)
            .copied()
            .ok_or(Error::Unimplemented("APU port index out of range"))
    }

    /// Borrow the current loaded IPL ROM if any.
    #[must_use]
    pub fn ipl_rom(&self) -> Option<&[u8]> {
        self.ipl_rom.as_deref()
    }

    /// Return a high-level status snapshot.
    #[must_use]
    pub fn status(&self) -> ApuStatus {
        ApuStatus {
            has_ipl_rom: self.ipl_rom.is_some(),
            using_builtin_bootstrap: self.ipl_rom.is_none(),
            configured_ipl_path: self.configured_ipl_path.clone(),
            spc700_steps: self.spc700_steps,
        }
    }

    fn advance_bootstrap_handshake(&mut self) {
        if !self.bootstrap_program_available() {
            return;
        }

        match self.bootstrap_state {
            BootstrapState::Idle => {}
            BootstrapState::WaitingForCpuBootstrapAck => {
                self.apu_to_cpu_ports = [0xAA, 0xBB, 0x00, 0x00];
                if self.cpu_to_apu_ports[0] == 0xCC {
                    self.bootstrap_state = BootstrapState::UploadingProgram;
                    self.apu_to_cpu_ports = self.cpu_to_apu_ports;
                }
            }
            BootstrapState::UploadingProgram => {
                // Keep the bootstrap upload handshake moving even while the full SPC700 IPL
                // program is not yet modeled. The CPU-side upload loop expects the APU to
                // acknowledge each transfer through the communication ports it just wrote.
                self.apu_to_cpu_ports = self.cpu_to_apu_ports;
                if self.cpu_to_apu_ports == [0x00, 0x00, 0x00, 0x00] {
                    self.bootstrap_state = BootstrapState::ProgramRunning;
                    self.apu_to_cpu_ports[0] = 0xAA;
                    self.apu_to_cpu_ports[1] = 0xBB;
                }
            }
            BootstrapState::ProgramRunning => {
                self.apu_to_cpu_ports[0] = 0xAA;
                self.apu_to_cpu_ports[1] = 0xBB;
                self.apu_to_cpu_ports[2] = self.cpu_to_apu_ports[2];
                self.apu_to_cpu_ports[3] = self.cpu_to_apu_ports[3];
                if self.cpu_to_apu_ports[0] == 0xCC {
                    self.bootstrap_state = BootstrapState::UploadingProgram;
                    self.apu_to_cpu_ports = self.cpu_to_apu_ports;
                }
            }
        }
    }

    const fn bootstrap_program_available(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum BootstrapState {
    #[default]
    Idle,
    WaitingForCpuBootstrapAck,
    UploadingProgram,
    ProgramRunning,
}

#[cfg(test)]
mod tests {
    use super::{Apu, SPC700_IPL_ROM_LEN};

    #[test]
    fn installs_valid_ipl_rom() {
        let mut apu = Apu::default();
        apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN], None)
            .unwrap();
        assert!(apu.status().has_ipl_rom);
        assert_eq!(apu.ipl_rom().unwrap()[0], 0xAA);
    }

    #[test]
    fn rejects_invalid_ipl_rom_size() {
        let mut apu = Apu::default();
        let error = apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN - 1], None);
        assert!(error.is_err());
    }

    #[test]
    fn port_roundtrip_and_step_accounting_work() {
        let mut apu = Apu::default();
        apu.write_cpu_port(0, 0x12).unwrap();
        apu.write_apu_port(3, 0x34).unwrap();
        apu.step_master_cycles(12);

        assert_eq!(apu.read_cpu_port(0).unwrap(), 0x12);
        assert_eq!(apu.read_apu_port(3).unwrap(), 0x34);
        assert_eq!(apu.status().spc700_steps, 2);
    }

    #[test]
    fn spc700_bus_routes_indirect_dsp_and_bidirectional_mailboxes() {
        let mut apu = Apu::default();
        apu.write_spc_bus(0x00f2, 0x5d);
        apu.write_spc_bus(0x00f3, 0x02);
        assert_eq!(apu.read_spc_bus(0x00f2), 0x5d);
        assert_eq!(apu.read_spc_bus(0x00f3), 0x02);
        assert_eq!(apu.read_dsp_register(0x5d), 0x02);

        apu.write_cpu_port(1, 0x34).unwrap();
        assert_eq!(apu.read_spc_bus(0x00f5), 0x34);
        apu.write_spc_bus(0x00f6, 0x56);
        assert_eq!(apu.read_apu_port(2).unwrap(), 0x56);

        apu.write_spc_bus(0x0300, 0x7b);
        assert_eq!(apu.read_spc_bus(0x0300), 0x7b);
    }

    #[test]
    fn spc700_program_moves_data_through_real_dsp_and_cpu_mailbox_ports() {
        let mut apu = Apu::default();
        // MOV $F2,#$5D; MOV $F3,#$02; MOV A,$F5; MOV $F4,A.
        let program = [0x8f, 0x5d, 0xf2, 0x8f, 0x02, 0xf3, 0xe4, 0xf5, 0xc4, 0xf4];
        for (index, byte) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0200 + index as u16, byte);
        }
        apu.spc700.load_state(0x0200, 0, 0, 0, 0xef, 0);
        apu.write_cpu_port(1, 0x55).unwrap();

        for _ in 0..4 {
            assert!(!apu.execute_spc_program_instruction().unwrap().is_empty());
        }
        assert_eq!(apu.read_dsp_register(0x5d), 2);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0x55);
        assert_eq!(apu.spc700.pc, 0x020a);
    }

    #[test]
    fn spc700_direct_page_flag_redirects_memory_moves_to_page_one() {
        let mut apu = Apu::default();
        // SETP; MOV $82,#$7B; MOV A,$82.
        for (index, value) in [0x40, 0x8f, 0x7b, 0x82, 0xe4, 0x82].into_iter().enumerate() {
            apu.write_spc_ram(0x0200 + index as u16, value);
        }
        apu.spc700.load_state(0x0200, 0, 0, 0, 0xef, 0);
        for _ in 0..3 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x0182), 0x7b);
        assert_eq!(apu.read_spc_ram(0x0082), 0);
        assert_eq!(apu.spc700.a, 0x7b);
    }

    #[test]
    fn can_decode_and_mix_brr_audio_from_spc_ram() {
        let mut apu = Apu::default();
        // Directory at $0200 and a filter-0 looping BRR block at $0300.
        for (address, value) in [
            (0x0200, 0x00),
            (0x0201, 0x03),
            (0x0202, 0x00),
            (0x0203, 0x03),
        ] {
            apu.write_spc_ram(address, value);
        }
        apu.write_spc_ram(0x0300, 0xc3);
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        for (register, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
            (0x4c, 0x01),
        ] {
            apu.write_dsp_register(register, value);
        }
        let mut audio = Vec::new();
        apu.append_dsp_audio(21_477_272 / 100, &mut audio);
        assert!((600..=700).contains(&audio.len()));
        assert!(audio.iter().any(|sample| *sample > 1000));
        assert_eq!(apu.read_spc_ram(0x0300), 0xc3);
        assert_eq!(apu.read_dsp_register(0x5d), 0x02);
    }

    #[test]
    fn sample_clock_produces_exactly_32k_pairs_per_ntsc_second() {
        let mut apu = Apu::default();
        assert_eq!(apu.advance_dsp_sample_clock(21_477_272), 32_000);
        assert_eq!(apu.advance_dsp_sample_clock(21_477_272), 32_000);
    }

    #[test]
    fn sample_clock_is_independent_of_cpu_instruction_chunking() {
        let mut whole = Apu::default();
        let mut chunks = Apu::default();
        let clocks = 357_368_u64; // one nominal NTSC frame
        let expected = whole.advance_dsp_sample_clock(clocks);
        let mut observed = 0;
        for _ in 0..(clocks / 6) {
            observed += chunks.advance_dsp_sample_clock(6);
        }
        observed += chunks.advance_dsp_sample_clock(clocks % 6);
        assert_eq!(expected, observed);
        assert!((530..=534).contains(&observed));
        assert_eq!(
            whole.advance_dsp_sample_clock(clocks),
            chunks.advance_dsp_sample_clock(clocks)
        );
    }

    #[test]
    fn dsp_fraction_survives_save_states_and_reset_clears_it() {
        let mut apu = Apu::default();
        assert_eq!(apu.advance_dsp_sample_clock(300), 0);
        let serialized = serde_json::to_string(&apu).unwrap();
        let mut resumed: Apu = serde_json::from_str(&serialized).unwrap();
        for clocks in [6, 4, 400, 357_368, 1, 123_456] {
            assert_eq!(
                apu.advance_dsp_sample_clock(clocks),
                resumed.advance_dsp_sample_clock(clocks)
            );
        }
        resumed.reset();
        let mut fresh = Apu::default();
        assert_eq!(
            resumed.advance_dsp_sample_clock(300),
            fresh.advance_dsp_sample_clock(300)
        );
    }

    #[test]
    fn builtin_bootstrap_handshake_runs_without_external_ipl_rom() {
        let mut apu = Apu::default();
        apu.reset();

        assert!(!apu.status().has_ipl_rom);
        assert!(apu.status().using_builtin_bootstrap);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xAA);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xBB);

        apu.write_cpu_port(0, 0xCC).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xCC);

        apu.write_cpu_port(0, 0x00).unwrap();
        apu.write_cpu_port(1, 0x00).unwrap();
        apu.write_cpu_port(2, 0x00).unwrap();
        apu.write_cpu_port(3, 0x00).unwrap();
        apu.step_master_cycles(6);

        assert_eq!(apu.read_apu_port(0).unwrap(), 0xAA);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xBB);
    }

    #[test]
    fn bootstrap_handshake_stays_in_upload_mode_after_cc_ack() {
        let mut apu = Apu::default();
        apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN], None)
            .unwrap();
        apu.reset();

        assert_eq!(apu.read_apu_port(0).unwrap(), 0xAA);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xBB);

        apu.write_cpu_port(0, 0xCC).unwrap();
        apu.write_cpu_port(1, 0x00).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xCC);

        apu.write_cpu_port(0, 0xAB).unwrap();
        apu.write_cpu_port(1, 0xC7).unwrap();
        apu.write_cpu_port(2, 0x60).unwrap();
        apu.write_cpu_port(3, 0x13).unwrap();
        apu.step_master_cycles(6);

        assert_eq!(apu.read_apu_port(0).unwrap(), 0xAB);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xC7);
        assert_eq!(apu.read_apu_port(2).unwrap(), 0x60);
        assert_eq!(apu.read_apu_port(3).unwrap(), 0x13);
    }

    #[test]
    fn bootstrap_upload_completion_exposes_program_ready_ports() {
        let mut apu = Apu::default();
        apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN], None)
            .unwrap();
        apu.reset();

        apu.write_cpu_port(0, 0xCC).unwrap();
        apu.step_master_cycles(6);
        apu.write_cpu_port(0, 0x00).unwrap();
        apu.write_cpu_port(1, 0x00).unwrap();
        apu.step_master_cycles(6);

        assert_eq!(apu.read_apu_port(0).unwrap(), 0xAA);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xBB);
        assert_eq!(apu.read_apu_port(2).unwrap(), 0x00);
        assert_eq!(apu.read_apu_port(3).unwrap(), 0x00);
    }

    #[test]
    fn bootstrap_upload_keeps_echoing_until_all_ports_go_idle() {
        let mut apu = Apu::default();
        apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN], None)
            .unwrap();
        apu.reset();

        apu.write_cpu_port(0, 0xCC).unwrap();
        apu.step_master_cycles(6);
        apu.write_cpu_port(0, 0x00).unwrap();
        apu.write_cpu_port(1, 0x00).unwrap();
        apu.write_cpu_port(2, 0x70).unwrap();
        apu.write_cpu_port(3, 0x55).unwrap();
        apu.step_master_cycles(6);

        assert_eq!(apu.read_apu_port(0).unwrap(), 0x00);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0x00);
        assert_eq!(apu.read_apu_port(2).unwrap(), 0x70);
        assert_eq!(apu.read_apu_port(3).unwrap(), 0x55);
    }
}
