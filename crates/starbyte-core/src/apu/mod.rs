//! Audio processing unit bootstrap boundary.

pub mod dsp;
mod gaussian;
pub mod spc700;
mod timers;
pub mod upload;

use std::{cell::RefCell, path::PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::error::{Error, Result};

use self::{
    dsp::Dsp,
    spc700::{Spc700, Spc700HaltState},
    timers::SpcTimers,
    upload::{IplUpload, UploadEvent},
};

/// Size of the user-supplied SPC700 IPL ROM.
pub const SPC700_IPL_ROM_LEN: usize = 64;

/// NTSC master-clock frequency in Hz. See `TimingState` for frame/dot clocks.
const NTSC_MASTER_CLOCK_HZ: u64 = 21_477_272;
/// SNES DSP produces 32,000 stereo sample pairs per second.
pub const DSP_SAMPLE_RATE_HZ: u64 = 32_000;
const SPC_RAM_BYTES: usize = 65_536;
/// SPC700 instructions use a nominal 1.024 MHz cycle clock.
const SPC700_CLOCK_HZ: u64 = 1_024_000;

const fn ipl_overlay_default() -> bool {
    true
}

const fn spc_test_default() -> u8 {
    // SPC TEST powers up with RAM writes enabled.
    0x0a
}

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

/// Execution progress for a sound program handed off by an opt-in IPL upload.
///
/// Absent until the transfer selects an entrypoint; production bootstrap
/// placeholder steps must never masquerade as sound-driver execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolatedSpc700Status {
    /// Current sound-CPU instruction address.
    pub pc: u16,
    /// Cumulative interpreted sound-CPU cycles since handoff.
    pub cycles: u64,
    /// Successfully dispatched instructions since handoff.
    pub instructions: u64,
    /// Running, sleeping, or stopped.
    pub halt_state: Spc700HaltState,
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
    /// Fractional SPC CPU cycles to NTSC master clocks for isolated execution.
    #[serde(default)]
    isolated_spc_master_phase: u64,
    /// Only an explicit opt-in probe runs uploaded SPC instructions beside 65816 time.
    #[serde(default)]
    host_clocked_spc_enabled: bool,
    /// Fractional sound-CPU clock corresponding to advancing SNES master clocks.
    #[serde(default)]
    host_clocked_spc_phase: u64,
    /// Signed instruction-cycle credit; a dispatched opcode may exceed the budget.
    #[serde(default)]
    host_clocked_spc_credit: i64,
    /// Independent SPC700 address space (not main CPU WRAM).
    #[serde(default = "blank_spc_ram")]
    spc_ram: Vec<u8>,
    /// Partial SNES DSP synthesis engine and voice/register state.
    #[serde(default)]
    dsp: Dsp,
    /// Sound-CPU timer registers and their fractional master-clock phase.
    #[serde(default)]
    timers: SpcTimers,
    /// SPC TEST register. Bit 1 gates ordinary RAM and DSP echo writes.
    #[serde(default = "spc_test_default")]
    spc_test: u8,
    /// SPC700 CONTROL bit 7: read the optional IPL from $FFC0-$FFFF.
    #[serde(default = "ipl_overlay_default")]
    ipl_overlay_enabled: bool,
    /// Explicitly enabled IPL transfer capture; disabled in the production shim.
    #[serde(default)]
    ipl_upload: Option<IplUpload>,
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
            isolated_spc_master_phase: 0,
            host_clocked_spc_enabled: false,
            host_clocked_spc_phase: 0,
            host_clocked_spc_credit: 0,
            spc_ram: blank_spc_ram(),
            dsp: Dsp::default(),
            timers: SpcTimers::default(),
            spc_test: spc_test_default(),
            ipl_overlay_enabled: true,
            ipl_upload: None,
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
        self.isolated_spc_master_phase = 0;
        self.host_clocked_spc_enabled = false;
        self.host_clocked_spc_phase = 0;
        self.host_clocked_spc_credit = 0;
        self.spc_ram.resize(SPC_RAM_BYTES, 0);
        self.spc_ram.fill(0);
        self.dsp.reset();
        self.timers = SpcTimers::default();
        self.spc_test = spc_test_default();
        self.ipl_overlay_enabled = true;
        if let Some(upload) = self.ipl_upload.as_mut() {
            upload.reset();
        }
        self.bootstrap_state = if self.ipl_upload.is_some() {
            BootstrapState::Idle
        } else if self.bootstrap_program_available() {
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
    /// Full cartridge runtime still uses the historical bootstrap boundary:
    /// all 256 opcodes dispatch, but commercial driver timing is not validated.
    pub fn execute_spc_program_instruction(&mut self) -> Result<Vec<crate::bus::BusEvent>> {
        // The CPU needs independent read/write callbacks to the same bus.
        // Temporarily move it out so the callbacks can safely borrow the APU.
        let mut cpu = std::mem::take(&mut self.spc700);
        // The moved-out APU CPU is only a placeholder while this opcode
        // executes. The TEST MMIO latch must observe the *active* CPU's
        // direct-page flag, not that placeholder's default PSW.
        // No SPC700 instruction modifies PSW.P and writes MMIO in the
        // same opcode, so the entry PSW is sufficient for bus writes.
        let instruction_psw = cpu.psw;
        let bus = RefCell::new(&mut *self);
        let result = cpu.step_with_memory(
            |address| bus.borrow_mut().read_spc_bus(address),
            |address, value| {
                bus.borrow_mut()
                    .write_spc_bus_with_psw(address, value, instruction_psw);
            },
        );
        let mut apu = bus.borrow_mut();
        apu.spc700 = cpu;
        if result.is_ok() {
            apu.spc700_steps = apu.spc700_steps.saturating_add(1);
        }
        result
    }

    /// Execute up to `instruction_limit` uploaded SPC700 instructions and
    /// synthesize DSP samples in emulated time, for isolated audio bring-up.
    ///
    /// This is explicitly NOT the production frame loop: it runs only after
    /// an opt-in IPL upload has handed off to an entry point. The instruction
    /// budget bounds infinite sound-driver loops. Uploaded commercial driver
    /// behavior, Linux speaker output and host/audio synchronization remain
    /// unverified. Do not mix this clock with `step_master_cycles` playback.
    pub fn run_isolated_uploaded_spc_program(
        &mut self,
        instruction_limit: usize,
        audio: &mut Vec<i16>,
    ) -> Result<usize> {
        if self.host_clocked_spc_enabled {
            return Err(Error::Unimplemented(
                "cannot mix host-clocked SPC700 execution with standalone sound-runner clock",
            ));
        }
        if self
            .ipl_upload
            .as_ref()
            .and_then(IplUpload::entrypoint)
            .is_none()
        {
            return Err(Error::Unimplemented(
                "isolated SPC700 upload has not reached a sound-program entrypoint",
            ));
        }
        let mut executed = 0;
        for _ in 0..instruction_limit {
            if self.spc700.halt_state() != Spc700HaltState::Running {
                break;
            }
            let trace = self.execute_spc_program_instruction()?;
            let cpu_cycles = trace.len() as u64;
            let numerator = u128::from(self.isolated_spc_master_phase)
                + u128::from(cpu_cycles) * u128::from(NTSC_MASTER_CLOCK_HZ);
            let denominator = u128::from(SPC700_CLOCK_HZ);
            self.isolated_spc_master_phase = (numerator % denominator) as u64;
            let master_cycles = (numerator / denominator) as u64;
            self.timers.advance_master_cycles(master_cycles);
            self.append_dsp_audio(master_cycles, audio);
            executed += 1;
        }
        Ok(executed)
    }

    /// Opt into running an uploaded sound program alongside CPU master clocks.
    ///
    /// This requires the separate opt-in IPL transfer. The production APU
    /// never enables the driver clock. Call once before the host begins its
    /// mailbox upload; do not mix with `run_isolated_uploaded_spc_program`.
    pub fn enable_isolated_host_clocked_driver(&mut self) -> Result<()> {
        if self.ipl_upload.is_none() {
            return Err(Error::Unimplemented(
                "enable opt-in IPL upload capture before sound-driver clocking",
            ));
        }
        self.host_clocked_spc_enabled = true;
        self.host_clocked_spc_phase = 0;
        self.host_clocked_spc_credit = 0;
        Ok(())
    }

    /// Whether the caller explicitly enabled the host-clocked driver probe.
    #[must_use]
    pub const fn isolated_host_clocked_driver_enabled(&self) -> bool {
        self.host_clocked_spc_enabled
    }

    /// Disarm the opt-in driver scheduler, for example when restoring a
    /// diagnostic snapshot into an ordinary desktop emulator session.
    pub fn disable_isolated_host_clocked_driver(&mut self) {
        self.host_clocked_spc_enabled = false;
        self.host_clocked_spc_phase = 0;
        self.host_clocked_spc_credit = 0;
    }

    /// Spend SNES master-clock time on uploaded SPC700 instructions, in the
    /// opt-in host-clocked probe only. Timers and DSP samples are advanced
    /// separately by `step_master_cycles` and `append_dsp_audio`, so this
    /// method must not clock either a second time.
    pub fn execute_host_clocked_spc(&mut self, master_cycles: u64) -> Result<usize> {
        // A prior large host quantum can leave positive instruction credit
        // after the 4096-instruction safety cap. A zero-time query must
        // never spend that backlog or change the sound CPU state.
        if master_cycles == 0 {
            return Ok(0);
        }
        if !self.host_clocked_spc_enabled
            || self
                .ipl_upload
                .as_ref()
                .and_then(IplUpload::entrypoint)
                .is_none()
            || self.spc700.halt_state() != Spc700HaltState::Running
        {
            return Ok(0);
        }

        let numerator = u128::from(self.host_clocked_spc_phase)
            + u128::from(master_cycles) * u128::from(SPC700_CLOCK_HZ);
        self.host_clocked_spc_phase = (numerator % u128::from(NTSC_MASTER_CLOCK_HZ)) as u64;
        let new_cycles = numerator / u128::from(NTSC_MASTER_CLOCK_HZ);
        self.host_clocked_spc_credit = self
            .host_clocked_spc_credit
            .saturating_add(i64::try_from(new_cycles).unwrap_or(i64::MAX));

        // Host frame stalls are independently guarded. Bound a single master
        // clock quantum as well to prevent an accidentally huge time advance
        // from monopolizing the GUI or diagnostics.
        let mut executed = 0;
        while self.host_clocked_spc_credit > 0
            && self.spc700.halt_state() == Spc700HaltState::Running
            && executed < 4096
        {
            let trace = self.execute_spc_program_instruction()?;
            self.host_clocked_spc_credit = self
                .host_clocked_spc_credit
                .saturating_sub(trace.len() as i64);
            executed += 1;
        }
        Ok(executed)
    }

    /// Enable byte-accurate mailbox upload capture for an isolated APU test.
    ///
    /// The production emulator deliberately retains its previous compatibility
    /// bootstrap until we can execute actual commercial sound drivers.
    /// This decoder stores uploaded bytes in audio RAM; a separate explicit
    /// host-clocked diagnostic can execute the program after handoff.
    /// Neither path may be enabled implicitly for ordinary gameplay.
    pub fn enable_isolated_ipl_upload(&mut self) {
        self.ipl_upload = Some(IplUpload::default());
        self.host_clocked_spc_enabled = false;
        self.host_clocked_spc_phase = 0;
        self.host_clocked_spc_credit = 0;
        self.bootstrap_state = BootstrapState::Idle;
        self.apu_to_cpu_ports = [0xaa, 0xbb, 0, 0];
    }

    /// State of the explicitly enabled sound-program upload decoder.
    #[must_use]
    pub fn isolated_ipl_upload(&self) -> Option<&IplUpload> {
        self.ipl_upload.as_ref()
    }

    /// Sound CPU state after isolated IPL entrypoint handoff, if any.
    #[must_use]
    pub fn isolated_spc700_status(&self) -> Option<IsolatedSpc700Status> {
        self.ipl_upload.as_ref()?.entrypoint()?;
        Some(IsolatedSpc700Status {
            pc: self.spc700.pc,
            cycles: self.spc700.cycles(),
            instructions: self.spc700_steps,
            halt_state: self.spc700.halt_state(),
        })
    }

    /// Advance placeholder APU work for a number of master cycles.
    pub fn step_master_cycles(&mut self, master_cycles: u64) {
        // Clock SPC hardware timers at their own fractional rate, independently
        // of the legacy placeholder CPU step counter.
        self.timers.advance_master_cycles(master_cycles);
        // The production sound-CPU path is still a non-executing placeholder.
        // Never advance that fake PC in opt-in transfer/execution mode: once
        // the upload hands off to the entrypoint, real SPC instructions own PC.
        // Otherwise an unrelated CPU instruction could silently skip bytes
        // of an uploaded driver before its first isolated execution.
        if self.ipl_upload.is_none() {
            let dummy_steps = master_cycles / 6;
            self.spc700.advance_placeholder_steps(dummy_steps);
            self.spc700_steps = self.spc700_steps.saturating_add(dummy_steps);
        }
        if let Some(upload) = self.ipl_upload.as_mut() {
            let event = upload.observe_with_ram_writes(
                self.cpu_to_apu_ports,
                &mut self.apu_to_cpu_ports,
                &mut self.spc_ram,
                self.spc_test & 0x02 != 0,
            );
            match event {
                UploadEvent::ByteWritten { address, value }
                | UploadEvent::NextBlockWithByte {
                    destination: address,
                    value,
                } if (0x00f0..=0x00ff).contains(&address) =>
                {
                    // The real IPL stores bytes with MOV [$00]+Y,A,
                    // including byte zero implicitly after a zero-valued
                    // block marker. The decoder has already copied RAM;
                    // replay the MMIO side effect (including DSP writes).
                    // Boot IPL executes in direct page zero regardless
                    // of any stale interpreter PSW in the host harness.
                    // TEST writes must see the boot loader's clear P flag.
                    self.write_spc_bus_with_psw(address, value, 0x02);
                }
                UploadEvent::EntryPoint { address } => {
                    // The driver was uploaded but is NOT automatically
                    // executed by the production frame loop. Discard
                    // earlier placeholder instruction counts.
                    self.spc700_steps = 0;
                    self.host_clocked_spc_phase = 0;
                    self.host_clocked_spc_credit = 0;
                    self.spc700.load_state(address, 0, 0, 0, 0, 0xef, 0x02);
                }
                _ => {}
            }
        } else {
            self.advance_bootstrap_handshake();
        }
    }

    /// Return the number of SNES DSP sample pairs due for this many NTSC master clocks.
    ///
    /// The partial DSP uses this phase accumulator for sample-stepped BRR,
    /// Gaussian filtering and echo/FIR synthesis. Retaining the fractional
    /// remainder avoids drift across CPU instructions, frames, and snapshots.
    /// This clock calculation does not run the SPC700.
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

    /// Read SPC700 RAM or an MMIO port. Timer outputs clear on read.
    ///
    /// This remains a standalone bus until real sound-driver execution is
    /// enabled. IPL overlay and general sound-CPU instruction timing are
    /// separate work.
    pub fn read_spc_bus(&mut self, address: u16) -> u8 {
        match address {
            // TEST and CONTROL are write-only MMIO. Reading the SPC bus
            // must not expose any underlying RAM bytes at these addresses.
            0x00f0 | 0x00f1 => 0,
            0x00f3 => self.dsp.read_register(self.spc_ram[0x00f2]),
            0x00f4..=0x00f7 => self.cpu_to_apu_ports[usize::from(address - 0x00f4)],
            // Timer targets are write-only MMIO. Their writes still
            // update underlying SPC RAM, but CPU reads return zero.
            0x00fa..=0x00fc => 0,
            0x00fd..=0x00ff => self.timers.read_and_clear(usize::from(address - 0x00fd)),
            0xffc0..=0xffff if self.ipl_overlay_enabled && self.ipl_rom.is_some() => {
                self.ipl_rom.as_ref().unwrap()[usize::from(address - 0xffc0)]
            }
            _ => self.read_spc_ram(address),
        }
    }

    /// Write the SPC700 RAM/MMIO bus, including indirect DSP register access.
    pub fn write_spc_bus(&mut self, address: u16, value: u8) {
        self.write_spc_bus_with_psw(address, value, self.spc700.psw);
    }

    /// Explicit executing-CPU PSW for the instruction callback: the actual
    /// SPC700 instance is temporarily moved out during opcode dispatch.
    fn write_spc_bus_with_psw(&mut self, address: u16, value: u8, psw: u8) {
        match address {
            0x00f0 => {
                // TEST responds to writes only with the SPC700 direct
                // page flag (PSW.P) clear. Its RAM mirror is always
                // written even if the hardware TEST latch ignores it.
                // Leave unsafe wait-state/clock-test modes unmodeled.
                if psw & 0x20 == 0 {
                    self.spc_test = value;
                }
            }
            0x00f1 => {
                self.ipl_overlay_enabled = value & 0x80 != 0;
                self.timers.write_control(value);
                // CONTROL bits 4 and 5 clear the CPU-to-SPC input latches;
                // they do not clear the SNES-visible SPC output ports.
                if value & 0x10 != 0 {
                    self.cpu_to_apu_ports[0..2].fill(0);
                }
                if value & 0x20 != 0 {
                    self.cpu_to_apu_ports[2..4].fill(0);
                }
            }
            // The high bit of the DSPADDR latch selects a read-only
            // DSPDATA mirror; preserve that bit in RAM at $F2.
            0x00f3 if self.spc_ram[0x00f2] & 0x80 == 0 => {
                self.dsp
                    .write_register(self.spc_ram[0x00f2], value, &self.spc_ram);
            }
            0x00f4..=0x00f7 => {
                self.apu_to_cpu_ports[usize::from(address - 0x00f4)] = value;
            }
            0x00fa..=0x00fc => {
                self.timers
                    .write_target(usize::from(address - 0x00fa), value);
            }
            // DSPADDR, read-only timer outputs, and unmapped locations
            // require no hardware side effect. Their RAM bytes are still
            // written below, as are all other MMIO overlay bytes.
            _ => {}
        }
        // TEST bit 1 disables ordinary RAM writes, but the $F0-$FF
        // MMIO window remains writable (including its underlying RAM).
        // DSP echo storage is gated independently at sample time.
        if self.spc_test & 0x02 != 0 || (0x00f0..=0x00ff).contains(&address) {
            self.write_spc_ram(address, value);
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
    /// Ordinary commercial-ROM output remains silent because the default
    /// runtime still uses a compatibility bootstrap. The opt-in host-clocked
    /// sound driver can configure this DSP and produce synthetic PCM.
    pub fn append_dsp_audio(&mut self, master_cycles: u64, output: &mut Vec<i16>) {
        let pairs = self.advance_dsp_sample_clock(master_cycles);
        for _ in 0..pairs {
            let (left, right) = self
                .dsp
                .next_stereo_pair_with_ram_writes(&mut self.spc_ram, self.spc_test & 0x02 != 0);
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

    /// Read the internal write-only TEST latch for opt-in diagnostics.
    /// This is not a guest-visible MMIO read from $F0.
    #[must_use]
    pub const fn test_latch(&self) -> u8 {
        self.spc_test
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
            using_builtin_bootstrap: self.ipl_rom.is_none() && self.ipl_upload.is_none(),
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
    use super::{Apu, SPC700_IPL_ROM_LEN, Spc700HaltState};

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
    fn placeholder_apu_bulk_stepping_keeps_exact_step_counts() {
        let mut apu = Apu::default();
        apu.step_master_cycles(6 * 70_000);
        assert_eq!(apu.status().spc700_steps, 70_000);
        assert_eq!(apu.spc700.pc, 70_000_u64 as u16);
        assert_eq!(apu.spc700.cycles(), 70_000);
        apu.step_master_cycles(5);
        assert_eq!(apu.status().spc700_steps, 70_000);
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
    fn optional_ipl_overlay_hides_but_preserves_underlying_high_ram() {
        let mut apu = Apu::default();
        let mut image = vec![0; SPC700_IPL_ROM_LEN];
        image[0] = 0xe8; // MOV A,#imm
        image[1] = 0x42;
        image[63] = 0xbb;
        apu.install_ipl_rom_bytes(image, None).unwrap();
        apu.reset();

        apu.write_spc_bus(0xffc0, 0x11);
        apu.write_spc_bus(0xffff, 0x22);
        assert_eq!(apu.read_spc_ram(0xffc0), 0x11);
        assert_eq!(apu.read_spc_bus(0xffc0), 0xe8);
        assert_eq!(apu.read_spc_bus(0xffff), 0xbb);

        // An isolated SPC instruction really fetches bytes from IPL overlay.
        apu.spc700.load_state(0xffc0, 0, 0, 0, 0xef, 0);
        apu.execute_spc_program_instruction().unwrap();
        assert_eq!(apu.spc700.a, 0x42);

        apu.write_spc_bus(0x00f1, 0);
        assert_eq!(apu.read_spc_bus(0xffc0), 0x11);
        assert_eq!(apu.read_spc_bus(0xffff), 0x22);
        apu.write_spc_bus(0x00f1, 0x80);
        assert_eq!(apu.read_spc_bus(0xffc0), 0xe8);
    }

    #[test]
    fn spc_timer_targets_control_and_read_clear_work_through_bus() {
        let mut apu = Apu::default();
        apu.write_spc_bus(0x00fa, 77);
        apu.write_spc_bus(0x00fc, 251);
        apu.write_spc_bus(0x00f1, 0b101);
        // Exactly one NTSC second corresponds to 8000 + 64000 timer ticks.
        apu.timers.advance_master_cycles(21_477_272);
        assert_eq!(apu.read_spc_bus(0x00fd), (8000 / 77) as u8 & 15);
        assert_eq!(apu.read_spc_bus(0x00fd), 0);
        assert_eq!(apu.read_spc_bus(0x00fe), 0);
        assert_eq!(apu.read_spc_bus(0x00ff), (64000 / 251) as u8 & 15);
        assert_eq!(apu.read_spc_bus(0x00ff), 0);
    }

    #[test]
    fn disabled_spc_timer_prescaler_phase_survives_apu_save_and_enable() {
        let mut apu = Apu::default();
        apu.write_spc_bus(0x00fc, 1); // TIMER2 increments every 16 SPC cycles.
        // Exactly 15 SPC cycles have elapsed at 315 NTSC master clocks.
        // The prescaler must advance even though the timer is disabled.
        apu.step_master_cycles(315);
        let saved = serde_json::to_string(&apu).unwrap();
        let mut resumed: Apu = serde_json::from_str(&saved).unwrap();

        for state in [&mut apu, &mut resumed] {
            state.write_spc_bus(0x00f1, 0b100);
            state.step_master_cycles(20);
            assert_eq!(state.read_spc_bus(0x00ff), 0);
            state.step_master_cycles(1);
            // The first stage-1 edge occurs one SPC cycle after enable,
            // rather than waiting for a new 16-cycle prescaler period.
            assert_eq!(state.read_spc_bus(0x00ff), 1);
            assert_eq!(state.read_spc_bus(0x00ff), 0);
        }
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&resumed).unwrap()
        );
    }

    #[test]
    fn movw_store_does_not_read_or_clear_timer_output_through_spc_bus() {
        let mut apu = Apu::default();
        apu.reset();
        apu.write_spc_bus(0x00fc, 1); // Timer 2 target: one divider tick.
        apu.write_spc_bus(0x00f1, 4); // Enable timer 2.
        apu.step_master_cycles(336); // At least 16 SPC cycles.
        let mut peek = apu.clone();
        assert_eq!(peek.read_spc_bus(0x00ff), 1);

        // MOVW $FF,YA writes timer output $FF and page-wrapped $00,
        // but does not read the timer's read-to-clear output. Writing
        // this read-only register must leave the output counter intact.
        for (index, byte) in [0xda, 0xff].into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + index as u16, byte);
        }
        apu.spc700.load_state(0x0400, 0x55, 0, 0x77, 0xef, 0x02);
        let trace = apu.execute_spc_program_instruction().unwrap();
        assert_eq!(trace.len(), 4);
        assert_eq!(trace[2].address, 0x00ff);
        assert_eq!(trace[2].access, crate::bus::AccessKind::Write);
        assert_eq!(trace[3].address, 0x0000);
        assert_eq!(trace[3].access, crate::bus::AccessKind::Write);
        assert_eq!(apu.read_spc_bus(0x00ff), 1);
        assert_eq!(apu.read_spc_bus(0x00ff), 0);
        assert_eq!(apu.read_spc_ram(0x00ff), 0x55);
        assert_eq!(apu.read_spc_ram(0x0000), 0x77);
    }

    #[test]
    fn spc_test_and_control_read_zero_through_bus_and_spc700_opcodes() {
        let mut apu = Apu::default();
        // Emulated RAM contents are not readable through write-only MMIO.
        apu.write_spc_ram(0x00f0, 0xab);
        apu.write_spc_ram(0x00f1, 0xcd);
        assert_eq!(apu.read_spc_bus(0x00f0), 0);
        assert_eq!(apu.read_spc_bus(0x00f1), 0);

        // MOV A,$F0; MOV $20,A; MOV A,$F1; MOV $21,A.
        let program = [0xe4, 0xf0, 0xc4, 0x20, 0xe4, 0xf1, 0xc4, 0x21];
        for (offset, byte) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + offset as u16, byte);
        }
        apu.spc700.load_state(0x0400, 0x7f, 0, 0, 0xef, 0);
        for _ in 0..4 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.spc700.pc, 0x0408);
        assert_eq!(apu.read_spc_ram(0x0020), 0);
        assert_eq!(apu.read_spc_ram(0x0021), 0);
        assert_eq!(apu.read_spc_ram(0x00f0), 0xab);
        assert_eq!(apu.read_spc_ram(0x00f1), 0xcd);
    }

    #[test]
    fn spc_timer_target_reads_are_zero_even_with_nonzero_underlying_ram() {
        let mut apu = Apu::default();
        for address in 0x00fa..=0x00fc {
            apu.write_spc_ram(address, 0xa5);
            apu.write_spc_bus(address, 3);
            assert_eq!(apu.read_spc_bus(address), 0);
            assert_eq!(apu.read_spc_ram(address), 3);
        }
        apu.write_spc_bus(0x00f1, 0b111);
        apu.step_master_cycles(100_000);
        // The real timer still runs despite the write-only target read.
        assert_ne!(apu.read_spc_bus(0x00fd), 0);
        assert_ne!(apu.read_spc_bus(0x00ff), 0);
    }

    #[test]
    fn spc_control_clears_only_selected_cpu_input_ports() {
        let mut apu = Apu::default();
        for index in 0..4 {
            apu.write_cpu_port(index, (index as u8) + 1).unwrap();
            apu.write_apu_port(index, (index as u8) + 10).unwrap();
        }
        apu.write_spc_bus(0x00f1, 0x10);
        assert_eq!(apu.read_cpu_port(0).unwrap(), 0);
        assert_eq!(apu.read_cpu_port(1).unwrap(), 0);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 3);
        assert_eq!(apu.read_cpu_port(3).unwrap(), 4);
        assert_eq!(apu.read_apu_port(0).unwrap(), 10);

        apu.write_spc_bus(0x00f1, 0x20);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 0);
        assert_eq!(apu.read_cpu_port(3).unwrap(), 0);
        assert_eq!(apu.read_apu_port(3).unwrap(), 13);
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
    fn spc_test_bit_one_gates_ordinary_ram_but_keeps_mmio_and_f8_f9_writable() {
        let mut apu = Apu::default();
        apu.reset();
        apu.write_spc_bus(0x2200, 0x11);
        assert_eq!(apu.read_spc_ram(0x2200), 0x11);

        apu.write_spc_bus(0x00f0, 0x00);
        assert_eq!(apu.read_spc_ram(0x00f0), 0);
        assert_eq!(apu.read_spc_bus(0x00f0), 0); // TEST is write-only.
        apu.write_spc_bus(0x2200, 0x99);
        assert_eq!(apu.read_spc_ram(0x2200), 0x11);
        apu.write_spc_bus(0x00f8, 0xa5);
        apu.write_spc_bus(0x00f9, 0x5a);
        assert_eq!(apu.read_spc_ram(0x00f8), 0xa5);
        assert_eq!(apu.read_spc_ram(0x00f9), 0x5a);
        apu.write_spc_bus(0x00f2, 0x0c);
        apu.write_spc_bus(0x00f3, 0x55);
        assert_eq!(apu.read_dsp_register(0x0c), 0x55);

        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        restored.write_spc_bus(0x2200, 0x77);
        assert_eq!(restored.read_spc_ram(0x2200), 0x11);
        restored.write_spc_bus(0x00f0, 0x0a);
        restored.write_spc_bus(0x2200, 0x77);
        assert_eq!(restored.read_spc_ram(0x2200), 0x77);
        restored.reset();
        restored.write_spc_bus(0x2200, 0x42);
        assert_eq!(restored.read_spc_ram(0x2200), 0x42);

        // Old snapshots did not serialize the test latch. They must
        // retain the power-on writable-RAM default on deserialization.
        let mut legacy = serde_json::to_value(&apu).unwrap();
        legacy.as_object_mut().unwrap().remove("spc_test");
        let mut from_legacy: Apu = serde_json::from_value(legacy).unwrap();
        from_legacy.write_spc_bus(0x2200, 0x66);
        assert_eq!(from_legacy.read_spc_ram(0x2200), 0x66);
    }

    #[test]
    fn executed_spc700_opcodes_obey_test_ram_write_protection() {
        let mut apu = Apu::default();
        apu.reset();
        // An original SPC700 program:
        // MOV $F0,#$00; MOV A,#$99; MOV !$2200,A;
        // MOV $F0,#$0A; MOV !$2200,A.
        let program = [
            0x8f, 0x00, 0xf0, 0xe8, 0x99, 0xc5, 0x00, 0x22,
            0x8f, 0x0a, 0xf0, 0xc5, 0x00, 0x22,
        ];
        for (index, value) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + index as u16, value);
        }
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xef, 0x02);
        for _ in 0..3 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x2200), 0);
        assert_eq!(apu.read_spc_ram(0x00f0), 0);
        for _ in 0..2 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x2200), 0x99);
        assert_eq!(apu.read_spc_ram(0x00f0), 0x0a);
    }

    #[test]
    fn spc_test_latch_ignores_writes_with_direct_page_flag_set() {
        let mut apu = Apu::default();
        apu.reset();
        apu.spc700.psw |= 0x20; // SETP.
        apu.write_spc_bus(0x00f0, 0); // Invalid hardware TEST write.
        assert_eq!(apu.read_spc_ram(0x00f0), 0);
        apu.write_spc_bus(0x2200, 0x55);
        assert_eq!(apu.read_spc_ram(0x2200), 0x55); // Still writable.

        apu.spc700.psw &= !0x20; // CLRP.
        apu.write_spc_bus(0x00f0, 0);
        apu.write_spc_bus(0x2200, 0xaa);
        assert_eq!(apu.read_spc_ram(0x2200), 0x55);

        apu.spc700.psw |= 0x20;
        apu.write_spc_bus(0x00f0, 0x0a); // Ignored until P is cleared.
        apu.write_spc_bus(0x2201, 0xa5);
        assert_eq!(apu.read_spc_ram(0x2201), 0);
        apu.spc700.psw &= !0x20;
        apu.write_spc_bus(0x00f0, 0x0a);
        apu.write_spc_bus(0x2201, 0xa5);
        assert_eq!(apu.read_spc_ram(0x2201), 0xa5);
    }

    #[test]
    fn interpreted_spc_test_write_uses_active_cpu_direct_page_flag() {
        let mut apu = Apu::default();
        apu.reset();
        // SETP; MOV A,#$00; MOV !$00F0,A (must not change TEST);
        // MOV A,#$55; MOV !$2200,A; CLRP;
        // MOV !$00F0,A (TEST=$55, bit 1 disabled);
        // MOV A,#$77; MOV !$2200,A (blocked);
        // MOV A,#$0A; MOV !$00F0,A (enable);
        // MOV !$2200,A (accepted).
        let program = [
            0x40, 0xe8, 0x00, 0xc5, 0xf0, 0x00,
            0xe8, 0x55, 0xc5, 0x00, 0x22, 0x20,
            0xc5, 0xf0, 0x00, 0xe8, 0x77, 0xc5, 0x00, 0x22,
            0xe8, 0x0a, 0xc5, 0xf0, 0x00, 0xc5, 0x00, 0x22,
        ];
        for (i, opcode) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + i as u16, opcode);
        }
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xef, 0x02);
        for _ in 0..3 {
            apu.execute_spc_program_instruction().unwrap();
        }
        apu.write_spc_bus(0x2200, 0x11); // TEST must still permit RAM writes.
        assert_eq!(apu.read_spc_ram(0x2200), 0x11);
        for _ in 0..4 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x2200), 0x55);
        for _ in 0..2 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x2200), 0x55);
        for _ in 0..3 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_spc_ram(0x2200), 0x0a);
    }

    #[test]
    fn spc_test_inhibits_echo_ram_storage_without_stopping_pcm_or_dsp_clock() {
        let mut apu = Apu::default();
        for (address, value) in [
            (0x0200, 0), (0x0201, 3), (0x0202, 0), (0x0203, 3),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        for (register, value) in [
            (0x5d, 2), (0x00, 0x7f), (0x01, 0x7f),
            (0x03, 0x10), (0x07, 0x7f),
            (0x0c, 0x7f), (0x1c, 0x7f), (0x4d, 1),
            (0x6d, 0x50), (0x7d, 1), (0x6c, 0), (0x4c, 1),
        ] {
            apu.write_spc_bus(0x00f2, register);
            apu.write_spc_bus(0x00f3, value);
        }
        apu.write_spc_bus(0x00f0, 0);
        let clocks = (21_477_272_u64 * 32).div_ceil(32_000);
        let mut gated_pcm = Vec::new();
        apu.append_dsp_audio(clocks, &mut gated_pcm);
        assert!(gated_pcm.iter().any(|&sample| sample > 1000));
        assert!((0x5000..0x5080).all(|addr| apu.read_spc_ram(addr) == 0));

        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        for instance in [&mut apu, &mut restored] {
            instance.write_spc_bus(0x00f0, 0x0a);
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        apu.append_dsp_audio(clocks, &mut a);
        restored.append_dsp_audio(clocks, &mut b);
        assert_eq!(a, b);
        assert!((0x5080..0x5100).any(|addr| apu.read_spc_ram(addr) != 0));
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }

    #[test]
    fn spc_mmio_writes_also_update_underlying_audio_ram() {
        let mut apu = Apu::default();
        // Read and write views are intentionally separate. The DSP and
        // ordinary RAM accessors must see each byte placed on the bus.
        for (address, value) in [
            (0x00f0, 0x0a), // TEST: behavior beyond RAM mirroring not modeled.
            (0x00f1, 0x00), // CONTROL hardware state.
            (0x00f2, 0x1d), // DSPADDR selects unused DSP register.
            (0x00f3, 0x53), // DSPDATA, also written into SPC RAM.
            (0x00f4, 0x62), // Output mailbox (CPU still sees the latch).
            (0x00fa, 0x07), // Timer target is write-only.
            (0x00fd, 0xe7), // Timer output is read-only, but RAM is written.
        ] {
            apu.write_spc_bus(address, value);
            assert_eq!(apu.read_spc_ram(address), value);
        }
        assert_eq!(apu.read_spc_bus(0x00f0), 0);
        assert_eq!(apu.read_spc_bus(0x00f1), 0);
        assert_eq!(apu.read_spc_bus(0x00fa), 0);
        assert_eq!(apu.read_spc_bus(0x00fd), 0);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0x62);
        // Port input is independent of the echoed RAM/output latch.
        assert_eq!(apu.read_spc_bus(0x00f4), 0);
    }

    #[test]
    fn spc700_mov_mmio_writes_mirror_to_ram_without_changing_port_readback() {
        let mut apu = Apu::default();
        // MOV $F0,#$0A; MOV $FA,#$04; MOV $F4,#$77;
        // MOV A,$F4; MOV $20,A.
        let program = [
            0x8f, 0x0a, 0xf0, 0x8f, 0x04, 0xfa, 0x8f, 0x77, 0xf4, 0xe4, 0xf4, 0xc4, 0x20,
        ];
        for (offset, byte) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + offset as u16, byte);
        }
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xef, 0);
        for _ in 0..5 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.spc700.pc, 0x040d);
        assert_eq!(apu.read_spc_ram(0x00f0), 0x0a);
        assert_eq!(apu.read_spc_ram(0x00fa), 4);
        assert_eq!(apu.read_spc_ram(0x00f4), 0x77);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0x77);
        assert_eq!(apu.read_spc_bus(0x00fa), 0); // Timer target is write-only.
        assert_eq!(apu.read_spc_ram(0x0020), 0); // CPU port input was zero.
        assert_eq!(apu.spc700.a, 0);
    }

    #[test]
    fn spc_dspaddr_high_bit_selects_read_only_register_mirror() {
        let mut apu = Apu::default();
        apu.write_spc_bus(0x00f2, 0x0c); // MVOL(L)
        apu.write_spc_bus(0x00f3, 0x36);
        assert_eq!(apu.read_spc_bus(0x00f3), 0x36);

        // $8C reads the same register as $0C, but cannot write it.
        apu.write_spc_bus(0x00f2, 0x8c);
        assert_eq!(apu.read_spc_bus(0x00f2), 0x8c);
        assert_eq!(apu.read_spc_bus(0x00f3), 0x36);
        apu.write_spc_bus(0x00f3, 0x7f);
        assert_eq!(apu.read_dsp_register(0x0c), 0x36);

        // Preserve the high bit through a save-state round trip.
        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        restored.write_spc_bus(0x00f3, 0x40);
        assert_eq!(restored.read_dsp_register(0x0c), 0x36);
        assert_eq!(restored.read_spc_bus(0x00f2), 0x8c);
        restored.write_spc_bus(0x00f2, 0x0c);
        restored.write_spc_bus(0x00f3, 0x40);
        assert_eq!(restored.read_dsp_register(0x0c), 0x40);
    }

    #[test]
    fn uploaded_spc_instructions_cannot_write_through_dsp_read_mirror() {
        let mut apu = Apu::default();
        // Original SPC700 program:
        // write MVOL(L)=0x24, select read mirror $8C,
        // attempt a forbidden write, read back MVOL(L), then write $48
        // through the normal $0C address.
        let program = [
            0x8f, 0x0c, 0xf2, 0x8f, 0x24, 0xf3, 0x8f, 0x8c, 0xf2, 0x8f, 0x7f, 0xf3, 0xe4, 0xf3,
            0x8f, 0x0c, 0xf2, 0x8f, 0x48, 0xf3,
        ];
        for (index, byte) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0200 + index as u16, byte);
        }
        apu.spc700.load_state(0x0200, 0, 0, 0, 0xef, 0);
        for _ in 0..5 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.spc700.a, 0x24);
        assert_eq!(apu.read_dsp_register(0x0c), 0x24);
        assert_eq!(apu.read_spc_bus(0x00f2), 0x8c);
        for _ in 0..2 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.read_dsp_register(0x0c), 0x48);
        assert_eq!(apu.spc700.pc, 0x0214);
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
    fn synthetic_ipl_startup_clears_ram_and_signals_ready() {
        let mut apu = Apu::default();
        // MOV X,#3; MOV SP,X; MOV A,#0; clear_loop: MOV (X),A;
        // DEC X; BNE clear_loop; MOV $F4,#$AA; MOV $F5,#$BB.
        // A tiny original regression program, not a bundled IPL ROM.
        let program = [
            0xcd, 0x03, 0xbd, 0xe8, 0x00, 0xc6, 0x1d, 0xd0, 0xfc, 0x8f, 0xaa, 0xf4, 0x8f, 0xbb,
            0xf5,
        ];
        for (index, byte) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + index as u16, byte);
        }
        for address in 1..=3 {
            apu.write_spc_ram(address, 0x88);
        }
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xff, 0);
        for _ in 0..14 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.spc700.sp, 3);
        assert_eq!(apu.spc700.pc, 0x040f);
        for address in 1..=3 {
            assert_eq!(apu.read_spc_ram(address), 0);
        }
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xaa);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0xbb);
    }

    #[test]
    fn spc700_transfer_opcode_writes_through_pointer_plus_y() {
        let mut apu = Apu::default();
        apu.write_spc_ram(0x0010, 0xfe);
        apu.write_spc_ram(0x0011, 0x02);
        // MOV [dp]+Y,A: $02FE + 3 = $0301.
        apu.write_spc_ram(0x0400, 0xd7);
        apu.write_spc_ram(0x0401, 0x10);
        apu.spc700.load_state(0x0400, 0x5a, 0, 3, 0xef, 0);
        apu.execute_spc_program_instruction().unwrap();
        assert_eq!(apu.read_spc_ram(0x0301), 0x5a);
        assert_eq!(apu.spc700.pc, 0x0402);
    }

    #[test]
    fn spc700_word_moves_wrap_within_selected_direct_page() {
        let mut apu = Apu::default();
        // SETP; MOVW YA,$FF; MOVW $FE,YA.
        for (index, value) in [0x40, 0xba, 0xff, 0xda, 0xfe].into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + index as u16, value);
        }
        apu.write_spc_ram(0x01ff, 0x34);
        apu.write_spc_ram(0x0100, 0x82);
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xef, 0);
        for _ in 0..3 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!((apu.spc700.a, apu.spc700.y), (0x34, 0x82));
        assert_eq!(apu.read_spc_ram(0x01fe), 0x34);
        assert_eq!(apu.read_spc_ram(0x01ff), 0x82);
        assert_eq!(apu.read_spc_ram(0x00ff), 0);
        assert_ne!(apu.spc700.psw & 0x80, 0);
    }

    #[test]
    fn spc700_comparisons_update_carry_and_nz_without_mutating_operands() {
        let mut apu = Apu::default();
        // CMP $30,#$7C; CMP Y,$30; INC $30; MOV Y,$30.
        for (index, value) in [0x78, 0x7c, 0x30, 0x7e, 0x30, 0xab, 0x30, 0xeb, 0x30]
            .into_iter()
            .enumerate()
        {
            apu.write_spc_ram(0x0400 + index as u16, value);
        }
        apu.write_spc_ram(0x0030, 0x7c);
        apu.spc700.load_state(0x0400, 0x66, 0, 0x70, 0xef, 0);
        apu.execute_spc_program_instruction().unwrap();
        assert_eq!(apu.spc700.psw & 0x03, 0x03); // equality -> C and Z
        apu.execute_spc_program_instruction().unwrap();
        assert_eq!(apu.spc700.psw & 0x01, 0); // Y < RAM -> clear C
        assert_ne!(apu.spc700.psw & 0x80, 0); // negative result
        apu.execute_spc_program_instruction().unwrap();
        apu.execute_spc_program_instruction().unwrap();
        assert_eq!(apu.spc700.y, 0x7d);
        assert_eq!(apu.read_spc_ram(0x0030), 0x7d);
        assert_eq!(apu.spc700.a, 0x66);
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

        // Drivers read ENDX indirectly through SPC MMIO, not the DSP test
        // helper. Loop events must latch and software clears must stick
        // until the next actual end/loop boundary.
        apu.write_spc_bus(0x00f2, 0x7c);
        assert_eq!(apu.read_spc_bus(0x00f3) & 1, 1);
        apu.write_spc_bus(0x00f3, 0xff);
        assert_eq!(apu.read_spc_bus(0x00f3) & 1, 0);
        // Re-key-on starts BRR sample zero; otherwise this exact test
        // duration can straddle an unrelated previous loop boundary.
        apu.write_dsp_register(0x4c, 1);
        apu.append_dsp_audio(1_000, &mut audio);
        assert_eq!(apu.read_spc_bus(0x00f3) & 1, 0);
        apu.append_dsp_audio(21_477, &mut audio);
        assert_eq!(apu.read_spc_bus(0x00f3) & 1, 1);
    }

    #[test]
    fn spc_dsp_held_koff_suppresses_rekey_through_mmio_and_survives_save() {
        let mut apu = Apu::default();
        for (address, value) in [
            (0x0200, 0),
            (0x0201, 3),
            (0x0202, 0),
            (0x0203, 3),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        for (register, value) in [
            (0x5d, 2),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
            (0x5c, 1), // Hold voice zero in key-off.
            (0x4c, 1), // Re-key while KOFF is still asserted.
        ] {
            apu.write_spc_bus(0x00f2, register);
            apu.write_spc_bus(0x00f3, value);
        }

        let mut blocked = Vec::new();
        apu.append_dsp_audio(21_477, &mut blocked);
        assert!(blocked.iter().all(|&sample| sample == 0));
        assert_eq!(apu.read_dsp_register(0x5c), 1);
        assert_eq!(apu.read_dsp_register(0x08), 0);

        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        for runtime in [&mut apu, &mut restored] {
            runtime.write_spc_bus(0x00f2, 0x5c);
            runtime.write_spc_bus(0x00f3, 0);
            runtime.write_spc_bus(0x00f2, 0x4c);
            runtime.write_spc_bus(0x00f3, 1);
        }
        let mut original_pcm = Vec::new();
        let mut restored_pcm = Vec::new();
        apu.append_dsp_audio(21_477, &mut original_pcm);
        restored.append_dsp_audio(21_477, &mut restored_pcm);
        assert_eq!(original_pcm, restored_pcm);
        assert!(original_pcm.iter().any(|&sample| sample > 1000));
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }

    #[test]
    fn spc_dsp_soft_reset_blocks_rekey_until_cleared_through_mmio() {
        let mut apu = Apu::default();
        for (address, value) in [
            (0x0200, 0),
            (0x0201, 3),
            (0x0202, 0),
            (0x0203, 3),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        for (reg, value) in [
            (0x5d, 2),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
        ] {
            apu.write_spc_bus(0x00f2, reg);
            apu.write_spc_bus(0x00f3, value);
        }
        apu.write_spc_bus(0x00f2, 0x4c);
        apu.write_spc_bus(0x00f3, 1);
        let mut audible = Vec::new();
        apu.append_dsp_audio(21_477, &mut audible);
        assert!(audible.iter().any(|&s| s > 1000));

        apu.write_spc_bus(0x00f2, 0x6c);
        apu.write_spc_bus(0x00f3, 0xa0); // FLG soft reset, echo write disabled.
        apu.write_spc_bus(0x00f2, 0x4c);
        apu.write_spc_bus(0x00f3, 1); // This KON must not override held reset.
        let mut silenced = Vec::new();
        apu.append_dsp_audio(21_477, &mut silenced);
        assert!(silenced.iter().all(|&s| s == 0));
        assert_eq!(apu.read_dsp_register(0x08), 0);
        assert_eq!(apu.read_dsp_register(0x09), 0);

        let state = serde_json::to_string(&apu).unwrap();
        let mut resumed: Apu = serde_json::from_str(&state).unwrap();
        for sound in [&mut apu, &mut resumed] {
            sound.write_spc_bus(0x00f2, 0x6c);
            sound.write_spc_bus(0x00f3, 0x20);
            sound.write_spc_bus(0x00f2, 0x4c);
            sound.write_spc_bus(0x00f3, 1);
        }
        let mut continuous = Vec::new();
        let mut resumed_pcm = Vec::new();
        apu.append_dsp_audio(21_477, &mut continuous);
        resumed.append_dsp_audio(21_477, &mut resumed_pcm);
        assert_eq!(continuous, resumed_pcm);
        assert!(continuous.iter().any(|&s| s > 1000));
    }

    #[test]
    fn spc_dsp_mmio_configures_echo_and_writes_actual_spc_ram() {
        let mut apu = Apu::default();
        for (address, value) in [
            (0x0200, 0x00),
            (0x0201, 0x03),
            (0x0202, 0x00),
            (0x0203, 0x03),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        for (register, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x00), // Mute direct path but keep echo send.
            (0x1c, 0x00),
            (0x2c, 0x7f),
            (0x3c, 0x7f),
            (0x4d, 0x01), // EON voice 0.
            (0x7f, 0x7f), // FIR7: newest echo sample.
            (0x6d, 0x50), // ESA $5000.
            (0x7d, 0x01), // 512 stereo-frame delay.
            (0x6c, 0x00), // Permit echo writes.
            (0x4c, 0x01), // KON voice 0.
        ] {
            apu.write_spc_bus(0x00f2, register);
            apu.write_spc_bus(0x00f3, value);
        }
        let mut samples = Vec::new();
        // Include 513 DSP sample periods with a one-sample rounding cushion.
        let clocks = (u64::from(21_477_272_u32) * 514).div_ceil(32_000);
        apu.append_dsp_audio(clocks, &mut samples);
        assert!(samples.len() >= 513 * 2);
        assert!(samples[..512 * 2].iter().all(|&s| s == 0));
        assert!(samples[512 * 2..].iter().any(|&s| s > 1000));
        assert_ne!(apu.read_spc_ram(0x5000), 0);
        assert_eq!(apu.read_dsp_register(0x4d), 0x01);
        let snapshot = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&snapshot).unwrap();
        let mut after = Vec::new();
        let mut after_restore = Vec::new();
        apu.append_dsp_audio(214_772, &mut after);
        restored.append_dsp_audio(214_772, &mut after_restore);
        assert_eq!(after, after_restore);
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
        // Software disables writes without suppressing playback of the
        // already populated SPC echo ring.
        apu.write_spc_bus(0x00f2, 0x6c);
        apu.write_spc_bus(0x00f3, 0x20);
        let old = apu.read_spc_ram(0x5004);
        apu.append_dsp_audio(21_477_272 / 32, &mut samples);
        assert_eq!(apu.read_spc_ram(0x5004), old);
    }

    #[test]
    fn echo_writes_spare_f8_f9_after_real_spc_mmio_configuration() {
        let mut apu = Apu::default();
        for (address, value) in [
            (0x0200, 0),
            (0x0201, 3),
            (0x0202, 0),
            (0x0203, 3),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        apu.write_spc_bus(0x00f8, 0xa5);
        apu.write_spc_bus(0x00f9, 0x5a);
        for (register, value) in [
            (0x5d, 2),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x4d, 1),
            (0x6d, 0),
            (0x7d, 1),
            (0x6c, 0),
            (0x4c, 1),
        ] {
            apu.write_spc_bus(0x00f2, register);
            apu.write_spc_bus(0x00f3, value);
        }
        let mut audio = Vec::new();
        let clocks = (21_477_272_u64 * 65).div_ceil(32_000);
        apu.append_dsp_audio(clocks, &mut audio);
        // Neither global direct volume nor echo volume was enabled.
        assert!(audio.iter().all(|&sample| sample == 0));
        assert_eq!(apu.read_spc_ram(0x00f8), 0xa5);
        assert_eq!(apu.read_spc_ram(0x00f9), 0x5a);
        assert_ne!(apu.read_spc_ram(0x00fc), 0);
        // These are regular SMP RAM bytes for software writes.
        apu.write_spc_bus(0x00f8, 0x42);
        assert_eq!(apu.read_spc_ram(0x00f8), 0x42);
    }

    #[test]
    fn apu_edl_change_waits_until_current_echo_ring_completes() {
        let mut apu = Apu::default();
        // The first EDL=1 ring covers $2000-$27FF. Place a sentinel
        // at the start of the second 2 KiB page to catch early writes.
        apu.write_spc_ram(0x2800, 0xa5);
        apu.write_spc_ram(0x2801, 0x5a);
        for (reg, value) in [(0x6d, 0x20), (0x7d, 1), (0x6c, 0)] {
            apu.write_spc_bus(0x00f2, reg);
            apu.write_spc_bus(0x00f3, value);
        }

        let clock_at = |samples: u64| (21_477_272_u64 * samples).div_ceil(32_000);
        let mut audio = Vec::new();
        apu.append_dsp_audio(clock_at(1), &mut audio);
        apu.write_spc_bus(0x00f2, 0x7d);
        apu.write_spc_bus(0x00f3, 2);
        assert_eq!(apu.read_dsp_register(0x7d), 2);

        // The current one-page ring still runs until its 512th sample.
        apu.append_dsp_audio(clock_at(512) - clock_at(1), &mut audio);
        assert_eq!(apu.read_spc_ram(0x2800), 0xa5);
        assert_eq!(apu.read_spc_ram(0x2801), 0x5a);
        let saved = serde_json::to_string(&apu).unwrap();
        let mut resumed: Apu = serde_json::from_str(&saved).unwrap();

        let mut later = Vec::new();
        let mut later_restored = Vec::new();
        let next_clocks = clock_at(1024) - clock_at(512);
        apu.append_dsp_audio(next_clocks, &mut later);
        resumed.append_dsp_audio(next_clocks, &mut later_restored);
        assert_eq!(later, later_restored);
        // The second 2 KiB region is not reached until sample 1025.
        assert_eq!(apu.read_spc_ram(0x2800), 0xa5);
        assert_eq!(resumed.read_spc_ram(0x2800), 0xa5);

        let last = clock_at(1025) - clock_at(1024);
        apu.append_dsp_audio(last, &mut later);
        resumed.append_dsp_audio(last, &mut later_restored);
        assert_eq!(apu.read_spc_ram(0x2800), 0);
        assert_eq!(apu.read_spc_ram(0x2801), 0);
        assert_eq!(later, later_restored);
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&resumed).unwrap()
        );
    }

    #[test]
    fn synthetic_spc700_arithmetic_program_configures_dsp_and_produces_audio() {
        let mut apu = Apu::default();
        // Copyright-free SPC sample directory and filter-zero looping BRR.
        for (address, value) in [
            (0x0200, 0x00),
            (0x0201, 0x03),
            (0x0202, 0x00),
            (0x0203, 0x03),
            (0x0300, 0xc3),
        ] {
            apu.write_spc_ram(address, value);
        }
        for offset in 1..9 {
            apu.write_spc_ram(0x0300 + offset, 0x77);
        }
        // Configure voice 0, but leave global output volumes unset
        // and key-on inactive. The sound CPU program must enable them.
        for (register, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
        ] {
            apu.write_dsp_register(register, value);
        }
        let mut before = Vec::new();
        apu.append_dsp_audio(21_477, &mut before);
        assert!(before.iter().all(|&sample| sample == 0));

        // MOV $F2,#$0C; MOV A,#$70; ADC A,#$0F; MOV $F3,A;
        // repeat for right volume ($1C); select KON ($4C), write 1.
        // The new immediate ADC opcode is therefore needed for sound.
        let program = [
            0x8f, 0x0c, 0xf2, 0xe8, 0x70, 0x88, 0x0f, 0xc4, 0xf3, 0x8f, 0x1c, 0xf2, 0xe8, 0x70,
            0x88, 0x0f, 0xc4, 0xf3, 0x8f, 0x4c, 0xf2, 0x8f, 0x01, 0xf3,
        ];
        for (index, value) in program.into_iter().enumerate() {
            apu.write_spc_ram(0x0400 + index as u16, value);
        }
        apu.spc700.load_state(0x0400, 0, 0, 0, 0xef, 0);
        for _ in 0..10 {
            apu.execute_spc_program_instruction().unwrap();
        }
        assert_eq!(apu.spc700.pc, 0x0418);
        assert_eq!(apu.read_dsp_register(0x0c), 0x7f);
        assert_eq!(apu.read_dsp_register(0x1c), 0x7f);
        assert_eq!(apu.read_dsp_register(0x4c), 0x01);

        let mut audio = Vec::new();
        apu.append_dsp_audio(214_773, &mut audio);
        assert!(audio.iter().any(|&sample| sample > 1000));
        assert!(audio.chunks_exact(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn gaussian_pcm_matches_across_cpu_sized_clock_chunks_and_apu_restore() {
        let mut whole = Apu::default();
        for (address, byte) in [
            (0x0200, 0x00),
            (0x0201, 0x03),
            (0x0202, 0x09),
            (0x0203, 0x03),
            (0x0300, 0xc0),
            (0x0309, 0xc3),
        ] {
            whole.write_spc_ram(address, byte);
        }
        for offset in 1..9 {
            whole.write_spc_ram(0x0300 + offset, 0x44);
            whole.write_spc_ram(0x0309 + offset, 0xcc);
        }
        for (register, byte) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x02, 0x00),
            (0x03, 0x08), // Half-sample fractional pitch.
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
            (0x4c, 0x01),
        ] {
            whole.write_dsp_register(register, byte);
        }
        let mut chunked = whole.clone();
        let mut all_audio = Vec::new();
        let mut pieces = Vec::new();
        const TOTAL_CLOCKS: u64 = 357_368;
        whole.append_dsp_audio(TOTAL_CLOCKS, &mut all_audio);
        chunked.append_dsp_audio(12_345, &mut pieces);
        // Serializing during the initial positive sample, including a
        // fractional DSP clock and Gaussian window, must not alter output.
        let saved = serde_json::to_string(&chunked).unwrap();
        let mut chunked: Apu = serde_json::from_str(&saved).unwrap();
        let mut remaining = TOTAL_CLOCKS - 12_345;
        let steps = [1, 7, 41, 233, 1001, 4096];
        let mut i = 0;
        while remaining > 0 {
            let next = remaining.min(steps[i % steps.len()]);
            chunked.append_dsp_audio(next, &mut pieces);
            remaining -= next;
            i += 1;
        }
        assert_eq!(all_audio, pieces);
        assert!(all_audio.iter().any(|&sample| sample > 1000));
        assert!(all_audio.iter().any(|&sample| sample < -1000));
        assert_eq!(whole.dsp_sample_phase, chunked.dsp_sample_phase);
        assert_eq!(
            serde_json::to_value(&whole.dsp).unwrap(),
            serde_json::to_value(&chunked.dsp).unwrap()
        );
        assert_eq!(whole.spc_ram, chunked.spc_ram);
    }

    #[test]
    fn signed_voice_pcm_and_live_outx_are_visible_via_spc_dsp_ports() {
        let mut apu = Apu::default();
        for (address, value) in [(0x0200, 0x00), (0x0201, 0x03)] {
            apu.write_spc_ram(address, value);
        }
        for (register, value) in [
            (0x5d, 0x02), // BRR sample directory at $0200.
            (0x00, 1),    // Tiny left voice volume.
            (0x01, 0),    // Right channel disabled.
            (0x0c, 1),    // Tiny left master volume.
            (0x1c, 0),
            (0x07, 1), // Direct GAIN -> envelope 16.
            (0x3d, 1), // NON -> initial signed noise -32768.
            (0x4c, 1), // KON voice 0.
        ] {
            apu.write_spc_bus(0x00f2, register);
            apu.write_spc_bus(0x00f3, value);
        }
        let mut pcm = Vec::new();
        apu.append_dsp_audio(672, &mut pcm);
        assert_eq!(pcm, [-1, 0]);

        // OUTX represents the signed envelope-scaled voice, not the tiny
        // post-master-volume DAC sample. ENVX remains independent of volume.
        apu.write_spc_bus(0x00f2, 0x09);
        assert_eq!(apu.read_spc_bus(0x00f3), 0xff);
        apu.write_spc_bus(0x00f2, 0x08);
        assert_eq!(apu.read_spc_bus(0x00f3), 1);

        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        let mut uninterrupted = Vec::new();
        let mut resumed = Vec::new();
        apu.append_dsp_audio(123_456, &mut uninterrupted);
        restored.append_dsp_audio(123_456, &mut resumed);
        assert_eq!(uninterrupted, resumed);
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
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
    fn two_block_ipl_upload_produces_brr_audio_from_transferred_ram() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();

        // First block: source zero's start/loop directory entry at $0200.
        for (port, value) in [(1, 1), (2, 0), (3, 2), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        for (index, byte) in [0, 3, 0, 3].into_iter().enumerate() {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
        }

        // Close block one, select a second block at $0300.
        for (port, value) in [(1, 1), (2, 0), (3, 3), (0, 5)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 5);

        // Second block: one looping, filter-zero BRR waveform.
        for (index, byte) in [0xc3, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77]
            .into_iter()
            .enumerate()
        {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
        }
        for (port, value) in [(1, 0), (2, 0), (3, 2), (0, 10)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.isolated_ipl_upload().unwrap().blocks_finished(), 2);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 13);
        assert_eq!(apu.read_spc_ram(0x0300), 0xc3);

        for (register, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
            (0x4c, 1),
        ] {
            apu.write_dsp_register(register, value);
        }
        let mut samples = Vec::new();
        apu.append_dsp_audio(214_773, &mut samples);
        assert!(samples.iter().any(|&value| value > 1000));
        assert!(samples.chunks_exact(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn opt_in_upload_copies_sound_driver_and_keeps_runtime_default_unchanged() {
        let mut apu = Apu::default();
        apu.reset();
        assert!(apu.isolated_ipl_upload().is_none());
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xaa);
        apu.step_master_cycles(60);
        assert_eq!(apu.status().spc700_steps, 10);

        apu.enable_isolated_ipl_upload();
        assert!(!apu.status().using_builtin_bootstrap);
        assert!(apu.isolated_spc700_status().is_none());
        for (port, value) in [(1, 1), (2, 0x00), (3, 0x04), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xcc);

        // Upload MOV A,#$42; MOV $F4,A with port-0 index acknowledgements.
        for (index, byte) in [0xe8, 0x42, 0xc4, 0xf4].into_iter().enumerate() {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
            assert_eq!(apu.read_apu_port(0).unwrap(), index as u8);
            assert_eq!(apu.read_spc_ram(0x0400 + index as u16), byte);
        }

        apu.write_cpu_port(1, 0).unwrap();
        apu.write_cpu_port(2, 0).unwrap();
        apu.write_cpu_port(3, 4).unwrap();
        apu.write_cpu_port(0, 5).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 5);
        assert_eq!(apu.spc700.pc, 0x0400);
        let status = apu.isolated_spc700_status().unwrap();
        assert_eq!(status.pc, 0x0400);
        assert_eq!(status.instructions, 0);
        assert_eq!(status.cycles, 0);
        assert_eq!(status.halt_state, Spc700HaltState::Running);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 4);
        assert_eq!(
            apu.isolated_ipl_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        // Host clocks after handoff must not fast-forward the real driver's
        // PC through legacy placeholder advancement.
        apu.step_master_cycles(600);
        assert_eq!(apu.spc700.pc, 0x0400);
        assert_eq!(apu.status().spc700_steps, 0);
        apu.execute_spc_program_instruction().unwrap();
        apu.execute_spc_program_instruction().unwrap();
        let status = apu.isolated_spc700_status().unwrap();
        assert_eq!(status.pc, 0x0404);
        assert_eq!(status.instructions, 2);
        assert!(status.cycles >= 2);
        assert_eq!(status.halt_state, Spc700HaltState::Running);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0x42);

        let saved = serde_json::to_string(&apu).unwrap();
        let restored: Apu = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.isolated_ipl_upload().unwrap().bytes_written(), 4);
        assert_eq!(restored.read_spc_ram(0x0402), 0xc4);
    }

    #[test]
    fn isolated_ipl_upload_to_output_mailboxes_clobbers_the_index_ack() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();
        for (port, value) in [(1, 1), (2, 0xf4), (3, 0), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xcc);

        // IPL first acknowledges index zero on SPC port 0, then its
        // MOV [$00]+Y,A writes to $F4, overwriting that acknowledgement.
        apu.write_cpu_port(1, 0x42).unwrap();
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_spc_ram(0x00f4), 0x42);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0x42);
        assert_eq!(apu.read_spc_bus(0x00f4), 0); // CPU input was zero.

        // Writing to $F5 changes output port 1, while the ordinary
        // byte-index acknowledgement remains visible on output port 0.
        apu.write_cpu_port(1, 0x53).unwrap();
        apu.write_cpu_port(0, 1).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 1);
        assert_eq!(apu.read_apu_port(1).unwrap(), 0x53);
        assert_eq!(apu.read_spc_ram(0x00f5), 0x53);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 2);
    }

    #[test]
    fn isolated_ipl_zero_block_marker_writes_implicit_byte_through_dsp_mmio() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();
        for (port, value) in [(1, 1), (2, 0), (3, 0x20), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        // A block of 254 bytes leaves the next token at $FE, so
        // command token zero is a valid signed forward marker.
        for index in 0..254_u16 {
            apu.write_cpu_port(1, 0x40).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
        }
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 254);

        // Port 1's nonzero block mode (here $0C) becomes data byte zero
        // immediately when the marker is zero, selecting DSPADDR $0C.
        for (port, value) in [(1, 0x0c), (2, 0xf2), (3, 0), (0, 0)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.isolated_ipl_upload().unwrap().blocks_finished(), 1);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 255);
        assert_eq!(apu.read_spc_ram(0x00f2), 0x0c);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0);

        // A repeated $00 is not another byte. The next token must be $01.
        apu.write_cpu_port(1, 0x5a).unwrap();
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 255);
        apu.write_cpu_port(0, 1).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_dsp_register(0x0c), 0x5a);
        assert_eq!(apu.read_spc_ram(0x00f3), 0x5a);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 256);
    }

    #[test]
    fn isolated_ipl_upload_respects_test_ram_protection_after_mmio_block() {
        let mut apu = Apu::default();
        apu.reset();
        // The IPL's boot-time P flag is clear even if the independent
        // interpreted SPC700 state was previously in direct-page one.
        apu.spc700.psw |= 0x20;
        apu.enable_isolated_ipl_upload();
        // Program TEST=$00 directly through the original IPL bus.
        for (port, value) in [(1, 1), (2, 0xf0), (3, 0), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        apu.write_cpu_port(1, 0).unwrap();
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_spc_ram(0x00f0), 0);

        // New block at $3000, index zero should acknowledge but not store.
        for (port, value) in [(1, 1), (2, 0), (3, 0x30), (0, 2)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        apu.write_cpu_port(1, 0x77).unwrap();
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0);
        assert_eq!(apu.read_spc_ram(0x3000), 0);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 2);

        // TEST can be reenabled through the real SPC bus; the next IPL
        // byte then uses the newly writable ordinary RAM again.
        apu.write_spc_bus(0x00f0, 0x0a);
        apu.write_cpu_port(1, 0x55).unwrap();
        apu.write_cpu_port(0, 1).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_spc_ram(0x3001), 0x55);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 3);
    }

    #[test]
    fn isolated_ipl_upload_to_dsp_mmio_applies_register_side_effects() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();
        // A real IPL block can target DSPADDR $F2 then DSPDATA $F3.
        // The transfer's RAM bytes must also program the DSP register.
        for (port, value) in [(1, 1), (2, 0xf2), (3, 0), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xcc);

        apu.write_cpu_port(1, 0x0c).unwrap(); // Main volume left.
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0);
        assert_eq!(apu.read_spc_ram(0x00f2), 0x0c);

        apu.write_cpu_port(1, 0x5a).unwrap();
        apu.write_cpu_port(0, 1).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 1);
        assert_eq!(apu.read_spc_ram(0x00f3), 0x5a);
        assert_eq!(apu.read_dsp_register(0x0c), 0x5a);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 2);

        // Another two-byte block with DSPADDR bit 7 set must not
        // overwrite the existing left-volume register.
        for (port, value) in [(1, 1), (2, 0xf2), (3, 0), (0, 3)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 3);
        apu.write_cpu_port(1, 0x8c).unwrap();
        apu.write_cpu_port(0, 0).unwrap();
        apu.step_master_cycles(6);
        apu.write_cpu_port(1, 0x7f).unwrap();
        apu.write_cpu_port(0, 1).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_dsp_register(0x0c), 0x5a);
        assert_eq!(apu.read_spc_ram(0x00f2), 0x8c);
        assert_eq!(apu.read_spc_ram(0x00f3), 0x7f);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 4);
        assert_eq!(apu.isolated_ipl_upload().unwrap().blocks_finished(), 1);
    }

    #[test]
    fn isolated_ipl_upload_rejects_stale_port_zero_commands_without_losing_bytes() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();
        for (port, value) in [(1, 1), (2, 0), (3, 4), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xcc);

        let program = [0xe8, 0x42, 0xc4, 0xf4]; // MOV A,#$42; MOV $F4,A.
        for (index, byte) in program.into_iter().take(3).enumerate() {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
            assert_eq!(apu.read_apu_port(0).unwrap(), index as u8);
        }
        // With expected index 3, stale token 1 and forward token 132
        // must neither acknowledge nor terminate the sound-driver block.
        for invalid in [1, 132] {
            apu.write_cpu_port(1, 0xff).unwrap();
            apu.write_cpu_port(0, invalid).unwrap();
            apu.step_master_cycles(6);
            assert_eq!(apu.read_apu_port(0).unwrap(), 2);
            assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 3);
            assert!(apu.isolated_spc700_status().is_none());
        }
        apu.write_cpu_port(1, program[3]).unwrap();
        apu.write_cpu_port(0, 3).unwrap();
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 3);
        for (index, byte) in program.into_iter().enumerate() {
            assert_eq!(apu.read_spc_ram(0x0400 + index as u16), byte);
        }
        for (port, value) in [(1, 0), (2, 0), (3, 4), (0, 5)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.isolated_spc700_status().unwrap().pc, 0x0400);
        assert_eq!(apu.isolated_ipl_upload().unwrap().bytes_written(), 4);
    }

    #[test]
    fn isolated_uploaded_sound_program_configures_dsp_and_produces_audio() {
        let mut apu = Apu::default();
        apu.reset();
        let mut output = Vec::new();
        assert!(
            apu.run_isolated_uploaded_spc_program(2, &mut output)
                .is_err()
        );
        apu.enable_isolated_ipl_upload();
        assert!(
            apu.run_isolated_uploaded_spc_program(2, &mut output)
                .is_err()
        );

        // Source zero directory and an original looping BRR waveform.
        for (index, byte) in [0, 3, 0, 3].into_iter().enumerate() {
            apu.write_spc_ram(0x0200 + index as u16, byte);
        }
        for (index, byte) in [0xc3, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77]
            .into_iter()
            .enumerate()
        {
            apu.write_spc_ram(0x0300 + index as u16, byte);
        }
        let mut program = Vec::new();
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
            // MOV $F2,#register; MOV $F3,#value.
            program.extend_from_slice(&[0x8f, register, 0xf2, 0x8f, value, 0xf3]);
        }
        program.extend_from_slice(&[0x2f, 0xfe]); // BRA -2 (bounded by host).
        for (port, value) in [(1, 1), (2, 0), (3, 4), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(apu.read_apu_port(0).unwrap(), 0xcc);
        for (index, byte) in program.iter().copied().enumerate() {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
            assert_eq!(apu.read_spc_ram(0x0400 + index as u16), byte);
        }
        for (port, value) in [(1, 0), (2, 0), (3, 4), (0, program.len() as u8 + 1)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(
            apu.isolated_ipl_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        assert_eq!(apu.spc700.pc, 0x0400);

        let mut host_clocked = apu.clone();
        host_clocked.enable_isolated_host_clocked_driver().unwrap();

        // Zero host time must never execute an opcode, and disarming a
        // diagnostic snapshot must prevent later unexpected execution.
        let mut gated = host_clocked.clone();
        assert_eq!(gated.execute_host_clocked_spc(0).unwrap(), 0);
        assert_eq!(gated.execute_host_clocked_spc(1).unwrap(), 0);
        assert_eq!(gated.isolated_spc700_status().unwrap().instructions, 0);
        gated.disable_isolated_host_clocked_driver();
        assert_eq!(gated.execute_host_clocked_spc(100_000).unwrap(), 0);
        assert_eq!(gated.isolated_spc700_status().unwrap().instructions, 0);

        // Huge external time advances must never run an unbounded sound
        // loop in one host step; the scheduler caps each call at 4096.
        let mut capped = host_clocked.clone();
        assert_eq!(
            capped.execute_host_clocked_spc(21_477_272 * 8).unwrap(),
            4096
        );
        assert_eq!(capped.isolated_spc700_status().unwrap().instructions, 4096);
        // Backlogged cycles cannot silently execute on a zero-time call.
        assert_eq!(capped.execute_host_clocked_spc(0).unwrap(), 0);
        assert_eq!(capped.isolated_spc700_status().unwrap().instructions, 4096);

        let mut host_audio = Vec::new();
        for _ in 0..500 {
            host_clocked.step_master_cycles(256);
            host_clocked.execute_host_clocked_spc(256).unwrap();
            host_clocked.append_dsp_audio(256, &mut host_audio);
        }
        let mut resumed_host = apu.clone();
        resumed_host.enable_isolated_host_clocked_driver().unwrap();
        let mut resumed_host_audio = Vec::new();
        for _ in 0..137 {
            resumed_host.step_master_cycles(256);
            resumed_host.execute_host_clocked_spc(256).unwrap();
            resumed_host.append_dsp_audio(256, &mut resumed_host_audio);
        }
        let state = serde_json::to_string(&resumed_host).unwrap();
        let mut resumed_host: Apu = serde_json::from_str(&state).unwrap();
        for _ in 137..500 {
            resumed_host.step_master_cycles(256);
            resumed_host.execute_host_clocked_spc(256).unwrap();
            resumed_host.append_dsp_audio(256, &mut resumed_host_audio);
        }
        assert_eq!(host_audio, resumed_host_audio);
        assert_eq!(
            serde_json::to_value(&host_clocked).unwrap(),
            serde_json::to_value(&resumed_host).unwrap()
        );
        let host_status = host_clocked.isolated_spc700_status().unwrap();
        assert!(host_status.instructions > 100);
        assert_eq!(host_clocked.read_dsp_register(0x4c), 1);
        assert!(host_audio.iter().any(|sample| *sample > 1_000));
        assert!(
            host_clocked
                .run_isolated_uploaded_spc_program(1, &mut Vec::new())
                .is_err()
        );

        let mut split_run = apu.clone();
        let mut split_audio = Vec::new();
        for _ in 0..4 {
            assert_eq!(
                split_run
                    .run_isolated_uploaded_spc_program(100, &mut split_audio)
                    .unwrap(),
                100
            );
        }

        // Save and restore in the middle of DSP playback, deliberately
        // splitting at an instruction count that leaves fractional clocks.
        // This must preserve the SPC PC, DSP voice envelope/phase, IPL
        // handoff, sound timers and both sample-clock remainders.
        let mut resumed_run = apu.clone();
        let mut resumed_audio = Vec::new();
        assert_eq!(
            resumed_run
                .run_isolated_uploaded_spc_program(137, &mut resumed_audio)
                .unwrap(),
            137
        );
        let snapshot = serde_json::to_string(&resumed_run).unwrap();
        let mut resumed_run: Apu = serde_json::from_str(&snapshot).unwrap();
        assert_eq!(
            resumed_run
                .run_isolated_uploaded_spc_program(263, &mut resumed_audio)
                .unwrap(),
            263
        );

        let executed = apu
            .run_isolated_uploaded_spc_program(400, &mut output)
            .unwrap();
        assert_eq!(executed, 400);
        assert_eq!(split_audio, output);
        assert_eq!(resumed_audio, output);
        assert_eq!(
            serde_json::to_value(&split_run).unwrap(),
            serde_json::to_value(&apu).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&resumed_run).unwrap(),
            serde_json::to_value(&apu).unwrap()
        );
        assert_eq!(apu.read_dsp_register(0x5d), 2);
        assert_eq!(apu.read_dsp_register(0x4c), 1);
        assert!(
            output.len() >= 30,
            "audio must advance with instruction timing"
        );
        assert!(output.iter().any(|&sample| sample > 1000));
        assert!(output.chunks_exact(2).all(|pair| pair[0] == pair[1]));

        // Host instruction limit prevents unbounded BRA loops.
        let steps = apu
            .run_isolated_uploaded_spc_program(3, &mut output)
            .unwrap();
        assert_eq!(steps, 3);
        let saved = serde_json::to_string(&apu).unwrap();
        let restored: Apu = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            restored.isolated_ipl_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        assert_eq!(restored.read_dsp_register(0x4c), 1);
    }

    #[test]
    fn uploaded_timer_driven_spc_driver_keys_audio_only_after_timer_ticks() {
        let mut apu = Apu::default();
        apu.reset();
        apu.enable_isolated_ipl_upload();
        apu.enable_isolated_host_clocked_driver().unwrap();
        // Original copyright-free looping BRR sample and SPC source directory.
        for (index, byte) in [0, 3, 0, 3].into_iter().enumerate() {
            apu.write_spc_ram(0x0200 + index as u16, byte);
        }
        for (index, byte) in [0xc3, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77]
            .into_iter()
            .enumerate()
        {
            apu.write_spc_ram(0x0300 + index as u16, byte);
        }

        // SPC700 MOV dp,#immediate configures TIMER2 at target=64, then
        // configures a DSP voice. The loop reads $FF (timer2, read-to-clear),
        // branches while zero, and sends KON only on a timer event.
        let mut program = vec![0x8f, 64, 0xfc, 0x8f, 4, 0xf1];
        for (register, value) in [
            (0x5d, 0x02),
            (0x00, 0x7f),
            (0x01, 0x7f),
            (0x03, 0x10),
            (0x07, 0x7f),
            (0x0c, 0x7f),
            (0x1c, 0x7f),
        ] {
            program.extend_from_slice(&[0x8f, register, 0xf2, 0x8f, value, 0xf3]);
        }
        let polling_pc = 0x0400 + program.len() as u16;
        program.extend_from_slice(&[
            0xe4, 0xff, // MOV A,$FF (timer2 output; clears on read).
            0xf0, 0xfc, // BEQ backwards four bytes, to timer read.
            0x8f, 0x4c, 0xf2, // DSP address=KON.
            0x8f, 0x01, 0xf3, // DSP KON voice zero.
            0x2f, 0xf4, // BRA backwards twelve bytes, to timer read.
        ]);

        for (port, value) in [(1, 1), (2, 0), (3, 4), (0, 0xcc)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        for (index, byte) in program.iter().copied().enumerate() {
            apu.write_cpu_port(1, byte).unwrap();
            apu.write_cpu_port(0, index as u8).unwrap();
            apu.step_master_cycles(6);
        }
        for (port, value) in [(1, 0), (2, 0), (3, 4), (0, program.len() as u8 + 1)] {
            apu.write_cpu_port(port, value).unwrap();
        }
        apu.step_master_cycles(6);
        assert_eq!(
            apu.isolated_ipl_upload().unwrap().entrypoint(),
            Some(0x0400)
        );
        assert_eq!(apu.read_dsp_register(0x4c), 0);

        let mut pcm = Vec::new();
        // The driver has time to set timer2 and its volume registers, but
        // fewer than 1024 SPC cycles elapse: timer target 64 cannot have
        // completed, so KON and generated audio must remain silent.
        for _ in 0..20 {
            apu.step_master_cycles(256);
            apu.execute_host_clocked_spc(256).unwrap();
            apu.append_dsp_audio(256, &mut pcm);
        }
        assert_eq!(apu.read_dsp_register(0x4c), 0);
        assert!(pcm.iter().all(|&sample| sample == 0));

        for _ in 20..5000 {
            apu.step_master_cycles(256);
            apu.execute_host_clocked_spc(256).unwrap();
            apu.append_dsp_audio(256, &mut pcm);
        }
        // $F1 and $FC are control/target MMIO, not mirrored RAM. The DSP's
        // KON action proves that timer2's read-to-clear counter fired.
        assert_eq!(apu.read_dsp_register(0x4c), 1);
        assert!(apu.isolated_spc700_status().unwrap().pc >= polling_pc);
        assert!(pcm.iter().filter(|&&value| value > 1000).count() > 100);

        // The timer, Gaussian voice history and sound CPU PC remain
        // deterministic when resuming from an APU save state.
        let saved = serde_json::to_string(&apu).unwrap();
        let mut restored: Apu = serde_json::from_str(&saved).unwrap();
        let mut continuous = Vec::new();
        let mut resumed = Vec::new();
        for _ in 0..100 {
            for (sound_apu, samples) in [(&mut apu, &mut continuous), (&mut restored, &mut resumed)]
            {
                sound_apu.step_master_cycles(256);
                sound_apu.execute_host_clocked_spc(256).unwrap();
                sound_apu.append_dsp_audio(256, samples);
            }
        }
        assert_eq!(continuous, resumed);
        assert_eq!(
            serde_json::to_value(&apu).unwrap(),
            serde_json::to_value(&restored).unwrap()
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
