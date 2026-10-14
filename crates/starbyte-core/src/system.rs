//! Bootstrap SNES system bus, WRAM, and timing-visible MMIO model.

use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::Result;
use crate::apu::Apu;
use crate::bus::{Address, Bus};
use crate::cartridge::Cartridge;
use crate::coprocessor::Coprocessor;
use crate::dma::DmaController;
use crate::input::ControllerState;
use crate::ppu::{FrameBuffer, Ppu, SCREEN_HEIGHT};
use crate::timing::{
    DOTS_PER_SCANLINE, HBLANK_START_DOT, MASTER_CLOCKS_PER_DOT, NTSC_SCANLINES_PER_FRAME,
    TimingState, VBLANK_START_SCANLINE,
};

const WRAM_SIZE: usize = 128 * 1024;
const LOW_WRAM_MIRROR_SIZE: usize = 0x2000;
const APU_IO_PORT_COUNT: usize = 4;
const PPU_REGISTER_COUNT: usize = 0x40;
const DEFAULT_IRQ_TIMER_COMPARE: u16 = 0x01FF;
/// Approximate SNES automatic joypad read duration: ~3 scanlines.
/// Precise controller serial timing remains outside the bootstrap clock model.
const AUTO_JOYPAD_READ_MASTER_CLOCKS: u64 = 4224;

const fn default_irq_timer_compare() -> u16 {
    DEFAULT_IRQ_TIMER_COMPARE
}

const fn default_wrio() -> u8 {
    0xFF
}

/// PPU counter latches and independent low/high read toggles. Both registers
/// are 9-bit, and a read of STAT78 resets the byte toggles.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct HvCounterLatch {
    horizontal: u16,
    vertical: u16,
    horizontal_high_next: bool,
    vertical_high_next: bool,
    latched: bool,
}

/// Compact MMIO activity gathered during commercial-ROM probing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemBusObservability {
    ppu_write_counts: Vec<u32>,
    apu_port_read_counts: Vec<u32>,
    apu_port_write_counts: Vec<u32>,
    /// NMI edges raised by VBlank timing (diagnostic, never saved).
    #[serde(skip)]
    nmi_assertions: u64,
    /// NMI edges actually consumed by the 65816 (diagnostic, never saved).
    #[serde(skip)]
    nmi_dispatches: u64,
    // Small live scanline samples are diagnostic-only, not saved guest state.
    #[serde(skip)]
    ppu_scanline_samples: Vec<[u16; 5]>,
    #[serde(skip)]
    completed_ppu_scanline_samples: Vec<[u16; 5]>,
}

impl Default for SystemBusObservability {
    fn default() -> Self {
        Self {
            ppu_write_counts: vec![0; PPU_REGISTER_COUNT],
            apu_port_read_counts: vec![0; APU_IO_PORT_COUNT],
            apu_port_write_counts: vec![0; APU_IO_PORT_COUNT],
            nmi_assertions: 0,
            nmi_dispatches: 0,
            ppu_scanline_samples: Vec::new(),
            completed_ppu_scanline_samples: Vec::new(),
        }
    }
}

impl SystemBusObservability {
    /// Per-register write counts for `$2100-$213F`.
    #[must_use]
    pub fn ppu_write_counts(&self) -> &[u32] {
        &self.ppu_write_counts
    }

    /// Per-port CPU read counts for `$2140-$2143`.
    #[must_use]
    pub fn apu_port_read_counts(&self) -> &[u32] {
        &self.apu_port_read_counts
    }

    /// Per-port CPU write counts for `$2140-$2143`.
    #[must_use]
    pub fn apu_port_write_counts(&self) -> &[u32] {
        &self.apu_port_write_counts
    }

    /// Number of scheduled and dispatched NMI edges since cartridge reset.
    #[must_use]
    pub const fn nmi_counts(&self) -> (u64, u64) {
        (self.nmi_assertions, self.nmi_dispatches)
    }

    /// Register snapshots from selected scanlines of the last completed frame.
    /// Each entry is [scanline, INIDISP, BGMODE, TM, TS].
    #[must_use]
    pub fn completed_ppu_scanline_samples(&self) -> &[[u16; 5]] {
        &self.completed_ppu_scanline_samples
    }
}

/// One explicitly enabled SNES cartridge-ROM read substitution.
///
/// Unlike a RAM write, the replacement applies only when the requested CPU
/// bus address resolves to a cartridge byte. The original ROM is untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomReadPatch {
    /// Full 24-bit SNES CPU bus address.
    pub address: u32,
    /// Byte to supply for matching cartridge reads.
    pub value: u8,
}

/// CPU-visible system bus state needed for bootstrap correctness work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBus {
    cartridge: Option<Cartridge>,
    /// Host-controlled cheats are never embedded in guest save-state files.
    #[serde(skip)]
    rom_read_patches: Vec<RomReadPatch>,
    /// Number of real cartridge reads intercepted by the active patch set.
    #[serde(skip)]
    rom_read_patch_hits: u64,
    coprocessor: Option<Coprocessor>,
    save_ram: Vec<u8>,
    wram: Vec<u8>,
    ppu: Ppu,
    /// Coherent PPU/video-memory snapshot from a late *visible* scanline.
    /// Unlike frame-end PPU state, this predates the game's VBlank uploads.
    /// Never serialized as guest save-state data.
    #[serde(skip)]
    visible_ppu_snapshot: Option<Ppu>,
    /// In-progress raster pixels, generated with *live* PPU state at each
    /// completed scanline. Enabled for Modes 1 and 2; Mode 2 shops require
    /// their per-line HDMA changes as well as BG3 offset-per-tile handling.
    #[serde(skip)]
    raster_frame: FrameBuffer,
    #[serde(skip)]
    completed_raster_frame: Option<FrameBuffer>,
    #[serde(skip)]
    raster_recording: bool,
    #[serde(skip)]
    raster_lines_rendered: u16,
    cpu_to_apu_io: Vec<u8>,
    /// CPU ports actually written since the last APU synchronization.
    /// Without this mask, an SPC CONTROL port-clear is undone every CPU
    /// instruction by blindly replaying stale CPU-side register contents.
    #[serde(default)]
    pending_apu_port_writes: u8,
    apu_to_cpu_io: Vec<u8>,
    dma: DmaController,
    timing: TimingState,
    open_bus: u8,
    nmitimen: u8,
    /// Approximate horizontal IRQ timer target ($4207/$4208).
    #[serde(default = "default_irq_timer_compare")]
    htime: u16,
    /// Approximate vertical IRQ timer target ($4209/$420A).
    #[serde(default = "default_irq_timer_compare")]
    vtime: u16,
    /// Programmable I/O port $4201; bit 7 enables PPU H/V latching.
    #[serde(default = "default_wrio")]
    wrio: u8,
    /// Latched H/V counters read via $213C/$213D.
    #[serde(default)]
    hv_latch: HvCounterLatch,
    rdnmi: bool,
    #[serde(default)]
    pending_nmi: bool,
    timeup: bool,
    #[serde(default)]
    pending_irq: bool,
    joypad: JoypadIo,
    #[serde(default)]
    math: MathIo,
    wram_address: u32,
    #[serde(default)]
    observability: SystemBusObservability,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self {
            cartridge: None,
            rom_read_patches: Vec::new(),
            rom_read_patch_hits: 0,
            coprocessor: None,
            save_ram: Vec::new(),
            wram: vec![0; WRAM_SIZE],
            ppu: Ppu::default(),
            visible_ppu_snapshot: None,
            raster_frame: FrameBuffer::default(),
            completed_raster_frame: None,
            raster_recording: false,
            raster_lines_rendered: 0,
            cpu_to_apu_io: vec![0; APU_IO_PORT_COUNT],
            pending_apu_port_writes: 0,
            apu_to_cpu_io: vec![0; APU_IO_PORT_COUNT],
            dma: DmaController::default(),
            timing: TimingState::default(),
            open_bus: 0,
            nmitimen: 0,
            htime: DEFAULT_IRQ_TIMER_COMPARE,
            vtime: DEFAULT_IRQ_TIMER_COMPARE,
            wrio: default_wrio(),
            hv_latch: HvCounterLatch::default(),
            rdnmi: false,
            pending_nmi: false,
            timeup: false,
            pending_irq: false,
            joypad: JoypadIo::default(),
            math: MathIo::default(),
            wram_address: 0,
            observability: SystemBusObservability::default(),
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
        self.rom_read_patches.clear();
        self.rom_read_patch_hits = 0;
    }

    /// Replace host-controlled ROM-read substitutions for the loaded cartridge.
    ///
    /// Later entries take precedence at the same CPU address. A cartridge
    /// replacement clears these patches; a soft reset preserves them.
    pub fn set_rom_read_patches(&mut self, patches: &[RomReadPatch]) {
        if self.rom_read_patches != patches {
            self.rom_read_patches.clear();
            self.rom_read_patches.extend_from_slice(patches);
            self.rom_read_patch_hits = 0;
        }
    }

    /// Count of matching cartridge reads since the active ROM patch set changed.
    ///
    /// A hit proves the emulated system used the replacement byte; it does
    /// not prove a visible gameplay effect or cartridge revision compatibility.
    #[must_use]
    pub const fn rom_read_patch_hits(&self) -> u64 {
        self.rom_read_patch_hits
    }

    /// Clear transient system state while keeping the current cartridge installed.
    pub fn reset(&mut self) {
        self.wram.fill(0);
        if let Some(coprocessor) = &mut self.coprocessor {
            trace!(coprocessor = ?coprocessor.kind(), "resetting coprocessor runtime");
            coprocessor.reset();
        }
        self.ppu = Ppu::default();
        self.visible_ppu_snapshot = None;
        self.raster_frame = FrameBuffer::default();
        self.completed_raster_frame = None;
        self.raster_recording = false;
        self.raster_lines_rendered = 0;
        self.cpu_to_apu_io.fill(0);
        self.pending_apu_port_writes = 0;
        self.apu_to_cpu_io.fill(0);
        self.dma = DmaController::default();
        self.timing = TimingState::default();
        self.open_bus = 0;
        self.nmitimen = 0;
        self.htime = DEFAULT_IRQ_TIMER_COMPARE;
        self.vtime = DEFAULT_IRQ_TIMER_COMPARE;
        self.wrio = default_wrio();
        self.hv_latch = HvCounterLatch::default();
        self.rdnmi = false;
        self.pending_nmi = false;
        self.timeup = false;
        self.pending_irq = false;
        self.joypad = JoypadIo::default();
        self.math = MathIo::default();
        self.wram_address = 0;
        self.observability = SystemBusObservability::default();
    }

    /// Advance global timing and derive pending interrupt state from it.
    pub fn advance_master_clocks(&mut self, clocks: u64) {
        if clocks == 0 {
            return;
        }
        let previous_scanline = self.timing.scanline;
        let previous_frame = self.timing.frame;
        let irq_match = self.irq_timer_crossed(clocks);
        let events = self.timing.advance_master_clocks(clocks);
        if let Some(coprocessor) = &mut self.coprocessor {
            coprocessor.step_master_cycles(clocks);
        }
        // Process every crossed scanline separately. The old boolean-only
        // handler silently skipped intermediate HDMA transfers when a long
        // DMA/CPU operation crossed multiple scanlines.
        let lines_crossed = (self.timing.frame - previous_frame)
            * u64::from(NTSC_SCANLINES_PER_FRAME)
            + u64::from(self.timing.scanline)
            - u64::from(previous_scanline);
        for offset in 0..lines_crossed {
            let finished_line = ((u64::from(previous_scanline) + offset)
                % u64::from(NTSC_SCANLINES_PER_FRAME)) as u16;
            if finished_line < SCREEN_HEIGHT as u16 {
                // The line is already completed. Capture its pixels BEFORE
                // processing HDMA for the next line, so subsequent changes
                // to TM, CGRAM, VRAM, scroll or OBJ cannot rewrite them.
                if finished_line == 0 {
                    self.raster_recording = matches!(self.ppu.read_register(0x2105) & 0x07, 1 | 2);
                    self.raster_lines_rendered = 0;
                }
                if self.raster_recording {
                    self.ppu
                        .render_scanline(&mut self.raster_frame, usize::from(finished_line));
                    self.raster_lines_rendered = self.raster_lines_rendered.saturating_add(1);
                }
            }

            let scanline = (finished_line + 1) % NTSC_SCANLINES_PER_FRAME;
            if scanline == 0 {
                self.observability.completed_ppu_scanline_samples =
                    std::mem::take(&mut self.observability.ppu_scanline_samples);
                self.completed_raster_frame = if self.raster_recording
                    && self.raster_lines_rendered == SCREEN_HEIGHT as u16
                {
                    Some(std::mem::take(&mut self.raster_frame))
                } else {
                    None
                };
                self.raster_recording = false;
                self.raster_lines_rendered = 0;
                self.initialize_hdma_channels();
            }
            self.step_hdma_for_scanline(scanline);
            if scanline == 211 {
                // Preserve the previous coherent single-frame fallback for
                // incomplete raster captures.
                self.visible_ppu_snapshot = Some(self.ppu.clone());
            }
            if matches!(scanline, 1 | 32 | 64 | 96 | 128 | 160 | 192 | 211 | 224) {
                let snapshot = [
                    scanline,
                    u16::from(self.peek_ppu_register(0x2100).unwrap_or(0)),
                    u16::from(self.peek_ppu_register(0x2105).unwrap_or(0)),
                    u16::from(self.peek_ppu_register(0x212C).unwrap_or(0)),
                    u16::from(self.peek_ppu_register(0x212D).unwrap_or(0)),
                ];
                self.observability.ppu_scanline_samples.push(snapshot);
            }
        }
        if events.entered_vblank {
            trace!(
                frame = self.timing.frame,
                scanline = self.timing.scanline,
                "entered vblank"
            );
            // NMITIMEN bit 0 starts the SNES's automatic joypad read
            // every VBlank. Games such as Super Mario World consume
            // $4218/$4219, without explicitly pulsing the $4016 latch.
            if self.nmitimen & 0x01 != 0 {
                self.joypad
                    .start_auto_read(self.timing.master_clock + AUTO_JOYPAD_READ_MASTER_CLOCKS);
            }
            self.rdnmi = true;
            if self.nmi_enabled() {
                self.pending_nmi = true;
                self.observability.nmi_assertions =
                    self.observability.nmi_assertions.saturating_add(1);
            }
        }
        if irq_match {
            self.timeup = true;
            self.pending_irq = true;
        }
        self.joypad.advance_auto_read(self.timing.master_clock);
    }

    /// Approximate IRQ trigger coordinates in the current dot-based timing
    /// model. Timer IRQs do not fire merely because a scanline changed.
    /// H-only: each scanline at HTIME. V-only: once per frame at VTIME/0.
    /// H+V: once per frame at VTIME/HTIME. Cycle-level offsets remain TODO.
    fn irq_timer_crossed(&self, clocks: u64) -> bool {
        let sub_dot_clocks = self.timing.master_clock % MASTER_CLOCKS_PER_DOT;
        let elapsed_dots = (sub_dot_clocks + clocks) / MASTER_CLOCKS_PER_DOT;
        if elapsed_dots == 0 {
            return false;
        }
        let dots = u64::from(DOTS_PER_SCANLINE);
        let scanlines = u64::from(NTSC_SCANLINES_PER_FRAME);
        let frame_dots = dots * scanlines;
        let start = u64::from(self.timing.scanline) * dots + u64::from(self.timing.dot);

        match self.nmitimen & 0x30 {
            0x10 if self.htime < DOTS_PER_SCANLINE => crosses_periodic_position(
                u64::from(self.timing.dot),
                elapsed_dots,
                u64::from(self.htime),
                dots,
            ),
            0x20 if self.vtime < NTSC_SCANLINES_PER_FRAME => crosses_periodic_position(
                start,
                elapsed_dots,
                u64::from(self.vtime) * dots,
                frame_dots,
            ),
            0x30 if self.htime < DOTS_PER_SCANLINE && self.vtime < NTSC_SCANLINES_PER_FRAME => {
                crosses_periodic_position(
                    start,
                    elapsed_dots,
                    u64::from(self.vtime) * dots + u64::from(self.htime),
                    frame_dots,
                )
            }
            _ => false,
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
        if let Some(raster) = &self.completed_raster_frame {
            framebuffer.pixels_mut().copy_from_slice(raster.pixels());
            if let Some(coprocessor) = &self.coprocessor {
                coprocessor.render_overlay(framebuffer);
            }
            return;
        }
        let live_main = self.ppu.read_register(0x212C);
        // Only substitute when the end-of-frame state has *no* background
        // layers, while the visible portion explicitly enabled one. This
        // avoids changing already-working Mode7 maps and ordinary scenes.
        // A single coherent snapshot also avoids the mixed-timestamp VRAM
        // corruption of the rolled-back experimental scanline compositor.
        let ppu = self
            .visible_ppu_snapshot
            .as_ref()
            .filter(|visible| {
                let visible_main = visible.read_register(0x212C);
                let visible_display = visible.read_register(0x2100);
                // Scope the initial correction to Mode 1, the mode of
                // Chrono's observed post-naming blackout. In particular,
                // leave the already-working Mode7 overworld unchanged.
                self.ppu.read_register(0x2105) & 0x07 == 1
                    && visible.read_register(0x2105) & 0x07 == 1
                    && live_main & 0x0F == 0
                    && visible_main & 0x0F != 0
                    && visible_display & 0x80 == 0
                    && visible_display & 0x0F != 0
            })
            .unwrap_or(&self.ppu);
        ppu.render_frame(framebuffer);
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

    /// Copy CPU-visible APU output ports from the APU runtime into the system bus view.
    pub fn sync_apu_ports_from_runtime(&mut self, apu: &Apu) {
        for port in 0..APU_IO_PORT_COUNT {
            self.apu_to_cpu_io[port] = apu.read_apu_port(port).unwrap_or(0);
        }
    }

    /// Replay only CPU mailbox writes made since the previous synchronization.
    ///
    /// SPC700 CONTROL may independently clear its input latches. Replaying
    /// all four last-written CPU bytes every 65816 instruction would
    /// immediately undo that hardware clear, preventing real sound drivers
    /// from safely exchanging commands with the main CPU.
    pub fn sync_apu_ports_to_runtime(&mut self, apu: &mut Apu) {
        let pending = self.pending_apu_port_writes;
        self.pending_apu_port_writes = 0;
        for port in 0..APU_IO_PORT_COUNT {
            if pending & (1 << port) != 0 {
                let _ = apu.write_cpu_port(port, self.cpu_to_apu_io[port]);
            }
        }
    }

    /// Borrow the current CPU-to-APU communication ports.
    #[must_use]
    pub fn cpu_to_apu_ports(&self) -> &[u8] {
        &self.cpu_to_apu_io
    }

    /// Borrow the current APU-to-CPU communication ports.
    #[must_use]
    pub fn apu_to_cpu_ports(&self) -> &[u8] {
        &self.apu_to_cpu_io
    }

    /// Number of bytes transferred through the DMA and HDMA engines.
    #[must_use]
    pub const fn dma_transferred_bytes(&self) -> u64 {
        self.dma.transfer_count
    }

    /// Current CPU interrupt-enable bits and H/V IRQ match coordinates.
    #[must_use]
    pub const fn irq_timer_configuration(&self) -> (u8, u16, u16) {
        (self.nmitimen, self.htime, self.vtime)
    }

    /// Controller-1 host bits, last latched $4218/$4219 bits, and auto-read busy.
    /// These are diagnostic values; querying them does not clock the joypad.
    #[must_use]
    pub const fn joypad_status(&self) -> (u16, u16, bool) {
        (
            self.joypad.controller1.to_bits(),
            self.joypad.latched1,
            self.joypad.auto_read_complete_at.is_some(),
        )
    }

    /// Peek at canonical SNES WRAM without changing open-bus or MMIO state.
    /// Only addresses in banks $7E and $7F are accepted.
    #[must_use]
    pub fn peek_wram_u8(&self, address: u32) -> Option<u8> {
        if !(0x7E0000..=0x7FFFFF).contains(&address) {
            return None;
        }
        high_wram_index(address).and_then(|index| self.wram.get(index).copied())
    }

    /// Borrow compact MMIO observability counters collected during execution.
    #[must_use]
    pub const fn observability(&self) -> &SystemBusObservability {
        &self.observability
    }

    /// Read one PPU register without mutating bus-visible side effects.
    #[must_use]
    pub fn peek_ppu_register(&self, register: u16) -> Option<u8> {
        if !(0x2100..=0x213F).contains(&register) {
            return None;
        }

        Some(self.ppu.read_register(register))
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
            if let Some(patch) = self
                .rom_read_patches
                .iter()
                .rev()
                .find(|patch| patch.address == address)
                .copied()
            {
                self.rom_read_patch_hits = self.rom_read_patch_hits.saturating_add(1);
                patch.value
            } else {
                value
            }
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

    fn poll_nmi(&mut self) -> bool {
        let pending = self.pending_nmi;
        self.pending_nmi = false;
        if pending {
            self.observability.nmi_dispatches = self.observability.nmi_dispatches.saturating_add(1);
        }
        pending
    }

    fn poll_irq(&mut self) -> bool {
        let pending = self.pending_irq;
        self.pending_irq = false;
        pending
    }
}

impl SystemBus {
    // The SNES DMA source/destination is connected to the A bus. CPU/PPU
    // MMIO lives on a separate bus and cannot be accessed through A-bus DMA.
    // In particular, an invalid reverse-DMA write to $420B must never start
    // another DMA transfer recursively and overflow the host stack.
    fn dma_a_bus_read(&mut self, address: Address) -> u8 {
        if dma_a_bus_mmio_forbidden(address) {
            self.open_bus
        } else {
            self.read(address)
        }
    }

    fn dma_a_bus_write(&mut self, address: Address, value: u8) {
        if dma_a_bus_mmio_forbidden(address) {
            self.open_bus = value;
        } else {
            self.write(address, value);
        }
    }

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
                    self.dma_a_bus_write(a_full, value);
                } else {
                    let value = self.dma_a_bus_read(a_full);
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

    fn step_hdma_for_scanline(&mut self, scanline: u16) {
        if scanline >= VBLANK_START_SCANLINE {
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

            // SNES HDMA always transfers on the first line of a block.
            // Descriptor bit 7 then chooses every-line transfer (set) or
            // wait-through-the-block after that first write (clear).
            if channel.hdma_first_line || channel.hdma_repeat {
                self.perform_hdma_transfer(&mut channel);
            }
            channel.hdma_first_line = false;

            channel.hdma_line_counter = channel.hdma_line_counter.saturating_sub(1);
            if channel.hdma_line_counter == 0 {
                self.reload_hdma_block(channel_index, &mut channel);
            }
            self.dma.set_channel(channel_index, channel);
        }
    }

    fn reload_hdma_block(&mut self, channel_index: usize, channel: &mut crate::dma::DmaChannel) {
        let table_full =
            (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_table_address);
        let line_descriptor = self.dma_a_bus_read(table_full);
        channel.hdma_table_address = channel.hdma_table_address.wrapping_add(1);

        if line_descriptor == 0 {
            channel.hdma_active = false;
            channel.hdma_line_counter = 0;
            channel.hdma_repeat = false;
            channel.hdma_first_line = false;
            return;
        }

        channel.hdma_active = true;
        // $80 means a single transfer followed by a 128-line wait.
        channel.hdma_line_counter = if line_descriptor == 0x80 {
            128
        } else {
            line_descriptor & 0x7F
        };
        channel.hdma_repeat = line_descriptor > 0x80;
        channel.hdma_first_line = true;

        if channel.hdma_indirect() {
            let low = self.dma_a_bus_read(
                (u32::from(channel.a_bus_bank) << 16) | u32::from(channel.hdma_table_address),
            );
            channel.hdma_table_address = channel.hdma_table_address.wrapping_add(1);
            let high = self.dma_a_bus_read(
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
            let source_bank = if channel.hdma_indirect() {
                channel.indirect_bank
            } else {
                channel.a_bus_bank
            };
            let source_address =
                (u32::from(source_bank) << 16) | u32::from(channel.hdma_data_address);
            let target_address =
                0x002100_u32 + u32::from(channel.b_bus_address) + u32::from(*pattern_offset);
            let value = self.dma_a_bus_read(source_address);
            self.write(target_address, value);
            channel.hdma_data_address = channel.hdma_data_address.wrapping_add(1);
            self.dma.transfer_count = self.dma.transfer_count.saturating_add(1);
        }

        if !channel.hdma_indirect() {
            channel.hdma_table_address = channel.hdma_data_address;
        }
    }

    /// Capture the approximate current raster position. The horizontal dot
    /// coordinate intentionally uses the existing timing model; no new
    /// instruction scheduling or clock changes are introduced.
    fn latch_hv_counters(&mut self) {
        self.hv_latch.horizontal = self.timing.dot & 0x01FF;
        self.hv_latch.vertical = self.timing.scanline & 0x01FF;
        self.hv_latch.latched = true;
        // Hardware does not reset low/high byte selectors when latching.
    }

    fn read_mmio(&mut self, address: Address) -> Option<u8> {
        if !is_cpu_io_bank(address) {
            return None;
        }
        let register = (address & 0xFFFF) as u16;

        match register {
            0x2137 => {
                // A software H/V latch read only works while WRIO bit 7 is
                // high. SLHV itself is an open-bus register.
                if self.wrio & 0x80 != 0 {
                    self.latch_hv_counters();
                }
                Some(self.open_bus)
            }
            0x213C => {
                let value = if self.hv_latch.horizontal_high_next {
                    (self.open_bus & 0xFE) | ((self.hv_latch.horizontal >> 8) as u8 & 1)
                } else {
                    self.hv_latch.horizontal as u8
                };
                self.hv_latch.horizontal_high_next = !self.hv_latch.horizontal_high_next;
                Some(value)
            }
            0x213D => {
                let value = if self.hv_latch.vertical_high_next {
                    (self.open_bus & 0xFE) | ((self.hv_latch.vertical >> 8) as u8 & 1)
                } else {
                    self.hv_latch.vertical as u8
                };
                self.hv_latch.vertical_high_next = !self.hv_latch.vertical_high_next;
                Some(value)
            }
            0x213F => {
                self.hv_latch.horizontal_high_next = false;
                self.hv_latch.vertical_high_next = false;
                // STAT78: NTSC (bit4 clear), external counter latch (bit6),
                // PPU2 revision 3 in bits0..3; preserve open-bus bit5.
                Some((self.open_bus & 0x20) | 0x03 | if self.hv_latch.latched { 0x40 } else { 0 })
            }
            0x2100..=0x213F => Some(self.ppu.read_data_register(register)),
            0x2140..=0x2143 => {
                let port = usize::from(register - 0x2140);
                self.observability.apu_port_read_counts[port] =
                    self.observability.apu_port_read_counts[port].saturating_add(1);
                Some(self.apu_to_cpu_io[port])
            }
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
                self.pending_irq = false;
                Some(value)
            }
            0x4212 => {
                let mut value = self.open_bus & 0x3E;
                if self.joypad.auto_read_complete_at.is_some() {
                    value |= 0x01;
                }
                if self.timing.in_vblank() {
                    value |= 0x80;
                }
                // HVBJOY bit 6 is H-blank status, not a dot-zero pulse.
                // Games poll this through most of the right-hand blanking
                // interval (roughly dot 274 to the end of each scanline).
                if self.timing.dot >= HBLANK_START_DOT {
                    value |= 0x40;
                }
                Some(value)
            }
            0x4213 => Some(self.wrio),
            0x4214 => Some(self.math.quotient as u8),
            0x4215 => Some((self.math.quotient >> 8) as u8),
            0x4216 => Some(self.math.product_remainder as u8),
            0x4217 => Some((self.math.product_remainder >> 8) as u8),
            0x4218 => Some((self.joypad.latched1 & 0x00FF) as u8),
            0x4219 => Some((self.joypad.latched1 >> 8) as u8),
            0x421A..=0x421F => Some(0), // No second controller or multitap connected.
            0x4300..=0x437F => Some(self.dma.read_register(register - 0x4300)),
            _ => None,
        }
    }

    fn write_mmio(&mut self, address: Address, value: u8) -> Option<()> {
        if !is_cpu_io_bank(address) {
            return None;
        }
        let register = (address & 0xFFFF) as u16;

        match register {
            0x2100..=0x213F => {
                let index = usize::from(register - 0x2100);
                self.observability.ppu_write_counts[index] =
                    self.observability.ppu_write_counts[index].saturating_add(1);
                self.ppu.write_register(register, value);
                Some(())
            }
            0x2140..=0x2143 => {
                let port = usize::from(register - 0x2140);
                self.observability.apu_port_write_counts[port] =
                    self.observability.apu_port_write_counts[port].saturating_add(1);
                self.cpu_to_apu_io[port] = value;
                self.pending_apu_port_writes |= 1 << port;
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
                if !self.irq_enabled() {
                    self.timeup = false;
                    self.pending_irq = false;
                }
                if self.nmi_enabled() && !nmi_was_enabled && self.timing.in_vblank() {
                    self.rdnmi = true;
                    self.pending_nmi = true;
                    self.observability.nmi_assertions =
                        self.observability.nmi_assertions.saturating_add(1);
                }
                Some(())
            }
            0x4201 => {
                // H/V hardware latch on the falling edge of WRIO bit 7.
                if self.wrio & 0x80 != 0 && value & 0x80 == 0 {
                    self.latch_hv_counters();
                }
                self.wrio = value;
                Some(())
            }
            0x4202 => {
                self.math.multiplicand = value;
                Some(())
            }
            0x4203 => {
                // The unsigned 8x8 multiplier shares its result registers
                // with the 16/8 divider's remainder.
                self.math.product_remainder = u16::from(self.math.multiplicand) * u16::from(value);
                self.math.quotient = u16::from(value);
                Some(())
            }
            0x4204 => {
                self.math.dividend = (self.math.dividend & 0xFF00) | u16::from(value);
                Some(())
            }
            0x4205 => {
                self.math.dividend = (self.math.dividend & 0x00FF) | (u16::from(value) << 8);
                Some(())
            }
            0x4206 => {
                if value == 0 {
                    self.math.quotient = 0xFFFF;
                    self.math.product_remainder = self.math.dividend;
                } else {
                    self.math.quotient = self.math.dividend / u16::from(value);
                    self.math.product_remainder = self.math.dividend % u16::from(value);
                }
                Some(())
            }
            0x4207 => {
                self.htime = (self.htime & 0x0100) | u16::from(value);
                Some(())
            }
            0x4208 => {
                self.htime = (self.htime & 0x00FF) | (u16::from(value & 0x01) << 8);
                Some(())
            }
            0x4209 => {
                self.vtime = (self.vtime & 0x0100) | u16::from(value);
                Some(())
            }
            0x420A => {
                self.vtime = (self.vtime & 0x00FF) | (u16::from(value & 0x01) << 8);
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

/// The SNES's CPU-side unsigned multiply/divide registers ($4202-$4206,
/// $4214-$4217). Results are committed at operation start in the current
/// instruction-granular timing model; mid-operation results are not modeled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct MathIo {
    multiplicand: u8,
    dividend: u16,
    quotient: u16,
    product_remainder: u16,
}

impl Default for MathIo {
    fn default() -> Self {
        Self {
            multiplicand: 0xFF,
            dividend: 0xFFFF,
            quotient: 0,
            product_remainder: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
struct JoypadIo {
    controller1: ControllerState,
    latch_line: bool,
    latched1: u16,
    shift1: u16,
    /// Pending auto-read completion time in master clock units.
    #[serde(default)]
    auto_read_complete_at: Option<u64>,
    /// Snapshot of the input at VBlank before automatic serial read completes.
    #[serde(default)]
    auto_read_bits: u16,
}

impl JoypadIo {
    fn start_auto_read(&mut self, complete_at: u64) {
        self.auto_read_bits = self.controller1.to_bits();
        self.auto_read_complete_at = Some(complete_at);
    }

    fn advance_auto_read(&mut self, master_clock: u64) {
        if self
            .auto_read_complete_at
            .is_some_and(|complete_at| master_clock >= complete_at)
        {
            self.latched1 = self.auto_read_bits;
            self.auto_read_complete_at = None;
        }
    }

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
            return (self.latched1 >> 15) as u8;
        }

        let bit = (self.shift1 >> 15) as u8;
        self.shift1 = (self.shift1 << 1) | 1;
        bit
    }
}

/// Return true when (start, start + advance] crosses a periodic timer match.
/// The endpoints are exclusive of the current instant so one match cannot
/// retrigger on every CPU instruction that starts exactly at its position.
fn crosses_periodic_position(start: u64, advance: u64, target: u64, period: u64) -> bool {
    if advance == 0 {
        return false;
    }
    if advance >= period {
        return true;
    }
    let delta = (target + period - (start % period)) % period;
    advance >= if delta == 0 { period } else { delta }
}

/// CPU MMIO is only decoded in system banks $00-$3F and $80-$BF.
fn is_cpu_io_bank(address: Address) -> bool {
    let bank = ((address >> 16) & 0xFF) as u8;
    matches!(bank, 0x00..=0x3F | 0x80..=0xBF)
}

/// Hardware bus-A DMA cannot reach B-bus PPU/APU ports or internal CPU MMIO.
/// This includes the DMA trigger ($420B): treating it as an ordinary CPU
/// write would recurse into execute_dma indefinitely.
fn dma_a_bus_mmio_forbidden(address: Address) -> bool {
    if !is_cpu_io_bank(address) {
        return false;
    }
    let register = (address & 0xFFFF) as u16;
    matches!(
        register,
        0x2100..=0x21FF | 0x4000..=0x41FF | 0x4200..=0x421F | 0x4300..=0x437F
    )
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
    use crate::apu::Apu;
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

    fn make_cart_with_title_and_rom_type(
        mapper: Mapper,
        rom_type: u8,
        title: &[u8; 21],
    ) -> Cartridge {
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
    fn rom_read_substitution_only_intercepts_cartridge_reads() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        let original = bus.read(0x008000);
        bus.write(0x7e0010, 0x27);
        bus.set_rom_read_patches(&[
            RomReadPatch {
                address: 0x008000,
                value: 0xA5,
            },
            RomReadPatch {
                address: 0x7e0010,
                value: 0xFF,
            },
            RomReadPatch {
                address: 0x002100,
                value: 0xFF,
            },
        ]);
        assert_eq!(bus.rom_read_patch_hits(), 0);
        assert_eq!(bus.read(0x008000), 0xA5);
        assert_eq!(bus.rom_read_patch_hits(), 1);
        assert_eq!(bus.read(0x7e0010), 0x27);
        assert_eq!(bus.rom_read_patch_hits(), 1);
        // The cartridge's original bytes are never changed.
        assert_eq!(bus.cartridge().unwrap().read_byte(0x008000), Some(original));
        bus.set_rom_read_patches(&[]);
        assert_eq!(bus.rom_read_patch_hits(), 0);
        assert_eq!(bus.read(0x008000), original);
        assert_eq!(bus.rom_read_patch_hits(), 0);
    }

    #[test]
    fn rom_read_substitution_is_exact_and_isolated_by_cartridge() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.set_rom_read_patches(&[
            RomReadPatch {
                address: 0x008000,
                value: 0xAA,
            },
            RomReadPatch {
                address: 0x008000,
                value: 0xBB,
            },
        ]);
        assert_eq!(bus.read(0x008000), 0xBB);
        assert_eq!(bus.rom_read_patch_hits(), 1);
        // Per-frame reapplication of the same set must preserve hit history.
        bus.set_rom_read_patches(&[
            RomReadPatch {
                address: 0x008000,
                value: 0xAA,
            },
            RomReadPatch {
                address: 0x008000,
                value: 0xBB,
            },
        ]);
        assert_eq!(bus.rom_read_patch_hits(), 1);
        assert_ne!(bus.read(0x808000), 0xBB);
        assert_eq!(bus.rom_read_patch_hits(), 1);
        bus.reset();
        assert_eq!(bus.read(0x008000), 0xBB);
        bus.install_cartridge(make_cart(Mapper::HiRom));
        assert_ne!(bus.read(0x008000), 0xBB);
    }

    #[test]
    fn rom_read_substitution_is_not_serialized_into_guest_state() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.set_rom_read_patches(&[RomReadPatch {
            address: 0x008000,
            value: 0xCA,
        }]);
        let serialized = serde_json::to_string(&bus).unwrap();
        assert!(!serialized.contains("rom_read_patches"));
        let mut restored: SystemBus = serde_json::from_str(&serialized).unwrap();
        assert_ne!(restored.read(0x008000), 0xCA);
    }

    #[test]
    fn cpu_math_multiplier_produces_unsigned_product_in_system_banks() {
        let mut bus = SystemBus::default();
        bus.write(0x044202, 0xD0);
        bus.write(0x044203, 0x04);
        assert_eq!(bus.read(0x044216), 0x40);
        assert_eq!(bus.read(0x804217), 0x03);
        assert_eq!(bus.read(0x004214), 0x04);

        // Updating only the second factor must start another multiplication.
        bus.write(0x004203, 0x02);
        assert_eq!(bus.read(0x004216), 0xA0);
        assert_eq!(bus.read(0x004217), 0x01);
        bus.write(0x004202, 0xFF);
        bus.write(0x004203, 0xFF);
        assert_eq!(bus.read(0x004216), 0x01);
        assert_eq!(bus.read(0x004217), 0xFE);
    }

    #[test]
    fn cpu_math_divider_produces_quotient_and_remainder_in_system_banks() {
        let mut bus = SystemBus::default();
        bus.write(0x044204, 0x40);
        bus.write(0x044205, 0x03);
        bus.write(0x044206, 0x08);
        assert_eq!(bus.read(0x044214), 0x68);
        assert_eq!(bus.read(0x044215), 0x00);
        assert_eq!(bus.read(0x044216), 0x00);
        assert_eq!(bus.read(0x044217), 0x00);

        bus.write(0x004204, 0x03);
        bus.write(0x004205, 0x01);
        bus.write(0x004206, 0x02);
        assert_eq!(bus.read(0x004214), 0x81);
        assert_eq!(bus.read(0x004215), 0x00);
        assert_eq!(bus.read(0x004216), 0x01);
        assert_eq!(bus.read(0x004217), 0x00);

        bus.write(0x004206, 0);
        assert_eq!(bus.read(0x004214), 0xFF);
        assert_eq!(bus.read(0x004215), 0xFF);
        assert_eq!(bus.read(0x004216), 0x03);
        assert_eq!(bus.read(0x004217), 0x01);
    }

    #[test]
    fn cpu_math_is_reset_and_not_mirrored_into_cartridge_banks() {
        let mut bus = SystemBus::default();
        bus.write(0x004202, 0xD0);
        bus.write(0x004203, 0x04);
        bus.write(0x404203, 0xFF);
        assert_eq!(bus.read(0x004216), 0x40);
        assert_eq!(bus.read(0x004217), 0x03);
        bus.reset();
        assert_eq!(bus.read(0x004216), 0);
        assert_eq!(bus.read(0x004217), 0);
        bus.write(0x004203, 0x02);
        assert_eq!(bus.read(0x004216), 0xFE);
        assert_eq!(bus.read(0x004217), 0x01);
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
    fn apu_port_sync_replays_only_new_cpu_writes_after_spc_input_clear() {
        let mut bus = SystemBus::default();
        let mut apu = Apu::default();
        apu.reset();
        bus.write(0x002140, 0xa5);
        bus.write(0x002142, 0x77);
        bus.sync_apu_ports_to_runtime(&mut apu);
        assert_eq!(apu.read_cpu_port(0).unwrap(), 0xa5);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 0x77);

        // The SPC700 clears its CPU input latches using CONTROL bits 4/5.
        apu.write_spc_bus(0x00f1, 0x30);
        assert_eq!(apu.read_cpu_port(0).unwrap(), 0);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 0);
        bus.sync_apu_ports_to_runtime(&mut apu);
        assert_eq!(apu.read_cpu_port(0).unwrap(), 0);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 0);

        // A real second CPU write must be forwarded even if it writes
        // exactly the same value again. Value equality cannot substitute
        // for tracking write events.
        bus.write(0x002140, 0xa5);
        bus.sync_apu_ports_to_runtime(&mut apu);
        assert_eq!(apu.read_cpu_port(0).unwrap(), 0xa5);
        assert_eq!(apu.read_cpu_port(2).unwrap(), 0);

        // Dirty writes made before a save must survive serialization;
        // otherwise a restored frame would lose the outstanding command.
        bus.write(0x002141, 0x4d);
        let snapshot = serde_json::to_string(&bus).unwrap();
        let mut restored: SystemBus = serde_json::from_str(&snapshot).unwrap();
        let mut restored_apu = apu.clone();
        restored.sync_apu_ports_to_runtime(&mut restored_apu);
        assert_eq!(restored_apu.read_cpu_port(1).unwrap(), 0x4d);
        restored.sync_apu_ports_to_runtime(&mut restored_apu);
        assert_eq!(restored_apu.read_cpu_port(1).unwrap(), 0x4d);
    }

    #[test]
    fn records_ppu_and_apu_port_mmio_activity() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));

        let mut apu = Apu::default();
        apu.reset();
        bus.sync_apu_ports_from_runtime(&apu);

        bus.write(0x002100, 0x80);
        bus.write(0x00212C, 0x10);
        assert_eq!(bus.read(0x002140), 0xAA);
        bus.write(0x002140, 0xCC);

        assert_eq!(bus.observability().ppu_write_counts()[0], 1);
        assert_eq!(bus.observability().ppu_write_counts()[0x2C], 1);
        assert_eq!(bus.observability().apu_port_read_counts()[0], 1);
        assert_eq!(bus.observability().apu_port_write_counts()[0], 1);
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
        bus.install_cartridge(make_cart_with_title_and_rom_type(
            Mapper::LoRom,
            0xF3,
            b"STARBYTE CX4 TEST    ",
        ));

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
            srtc.coprocessor()
                .map(crate::coprocessor::Coprocessor::kind),
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
    fn automatic_joypad_read_latches_start_during_vblank_after_busy_interval() {
        use crate::timing::{DOTS_PER_SCANLINE, MASTER_CLOCKS_PER_DOT, VBLANK_START_SCANLINE};

        let mut bus = SystemBus::default();
        bus.write(0x004200, 0x01); // NMITIMEN: enable automatic joypad read.
        bus.set_controller1(ControllerState {
            start: true,
            b: true,
            ..ControllerState::default()
        });
        assert_eq!(bus.read(0x004218), 0);

        bus.advance_master_clocks(
            u64::from(VBLANK_START_SCANLINE) * u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT,
        );
        assert_eq!(bus.read(0x004212) & 0x81, 0x81); // VBlank and busy.
        assert_eq!(bus.read(0x004218), 0, "read has not finished yet");

        // Controller is sampled at the beginning of automatic polling.
        bus.set_controller1(ControllerState::default());
        bus.advance_master_clocks(AUTO_JOYPAD_READ_MASTER_CLOCKS - 1);
        assert_eq!(bus.read(0x004212) & 0x01, 1, "still busy");
        assert_eq!(bus.read(0x004218), 0);

        bus.advance_master_clocks(1);
        assert_eq!(bus.read(0x004212) & 0x01, 0, "auto read finished");
        assert_eq!(bus.read(0x004218), 0, "signature remains zero");
        assert_eq!(bus.read(0x004219), 0x90, "B and Start are latched");
    }

    #[test]
    fn automatic_joypad_polling_requires_nmitimen_bit_zero() {
        use crate::timing::{DOTS_PER_SCANLINE, MASTER_CLOCKS_PER_DOT, VBLANK_START_SCANLINE};

        let mut bus = SystemBus::default();
        bus.write(0x004200, 0x80); // NMI enabled, auto polling disabled.
        bus.set_controller1(ControllerState {
            start: true,
            ..ControllerState::default()
        });
        bus.advance_master_clocks(
            u64::from(VBLANK_START_SCANLINE) * u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT,
        );
        bus.advance_master_clocks(AUTO_JOYPAD_READ_MASTER_CLOCKS);
        assert_eq!(bus.read(0x004212) & 0x01, 0, "never busy");
        assert_eq!(bus.read(0x004218), 0, "no automatic latch");

        // Explicit manual latching still works regardless of NMITIMEN.
        bus.write(0x004016, 1);
        bus.write(0x004016, 0);
        assert_eq!(bus.read(0x004218), 0);
        assert_eq!(bus.read(0x004219), 0x10);
    }

    #[test]
    fn automatic_joypad_read_uses_current_vblank_sample_on_subsequent_frames() {
        use crate::timing::{
            DOTS_PER_SCANLINE, MASTER_CLOCKS_PER_DOT, NTSC_SCANLINES_PER_FRAME,
            VBLANK_START_SCANLINE,
        };

        let mut bus = SystemBus::default();
        bus.write(0x004200, 0x01);
        let clocks_to_vblank =
            u64::from(VBLANK_START_SCANLINE) * u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        let clocks_per_frame = u64::from(NTSC_SCANLINES_PER_FRAME)
            * u64::from(DOTS_PER_SCANLINE)
            * MASTER_CLOCKS_PER_DOT;

        bus.advance_master_clocks(clocks_to_vblank);
        bus.advance_master_clocks(AUTO_JOYPAD_READ_MASTER_CLOCKS);
        assert_eq!(bus.read(0x004218), 0);

        bus.set_controller1(ControllerState {
            start: true,
            ..ControllerState::default()
        });
        bus.advance_master_clocks(
            clocks_per_frame - clocks_to_vblank - AUTO_JOYPAD_READ_MASTER_CLOCKS,
        );
        assert_eq!(bus.timing().scanline, 0);
        bus.advance_master_clocks(clocks_to_vblank);
        assert_eq!(bus.read(0x004212) & 0x01, 1);
        bus.advance_master_clocks(AUTO_JOYPAD_READ_MASTER_CLOCKS);
        assert_eq!(bus.read(0x004218), 0);
        assert_eq!(bus.read(0x004219), 0x10);
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

        assert_eq!(bus.read(0x004218), 0x80); // A, first byte.
        assert_eq!(bus.read(0x004219), 0x90); // B and Start, second byte.
        assert_eq!(bus.read(0x00421A), 0); // No second controller connected.
        assert_eq!(bus.read(0x00421B), 0);
        assert_eq!(bus.read(0x004016) & 0x01, 1); // B
        assert_eq!(bus.read(0x004016) & 0x01, 0); // Y
        assert_eq!(bus.read(0x004016) & 0x01, 0); // Select
        assert_eq!(bus.read(0x004016) & 0x01, 1); // Start
        for _ in 0..12 {
            bus.read(0x004016); // Remaining controller bits.
        }
        assert_eq!(bus.read(0x004016) & 0x01, 1); // Serial reads beyond bit 16 return 1.
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
    fn reverse_dma_to_mdmaen_cannot_recursively_start_dma() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        // The previous implementation wrote the B-bus value through the
        // regular CPU bus. A reverse transfer into $420B triggered DMA
        // recursively until the Rust host stack overflowed.
        bus.write(0x002100, 0x01);
        bus.write(0x004300, 0x80); // B -> A, one register.
        bus.write(0x004301, 0x00); // B-bus $2100 contains $01.
        bus.write(0x004302, 0x0B);
        bus.write(0x004303, 0x42);
        bus.write(0x004304, 0x00);
        bus.write(0x004305, 0x01);
        bus.write(0x004306, 0x00);
        bus.write(0x00420B, 0x01);

        assert_eq!(bus.dma.transfer_count, 1);
        assert_eq!(bus.dma.dma_enable_mask(), 0);
        assert_eq!(bus.ppu().read_register(0x2100), 0x01);
    }

    #[test]
    fn dma_a_bus_cannot_read_ppu_mmio_as_a_memory_source() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x002100, 0x0F);
        bus.write(0x002121, 0x00);
        bus.write(0x004300, 0x00); // A -> B, single byte.
        bus.write(0x004301, 0x22); // CGRAM data register.
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x21); // Invalid A-bus source $002100.
        bus.write(0x004304, 0x00);
        bus.write(0x004305, 0x01);
        bus.write(0x004306, 0x00);
        bus.write(0x00420B, 0x01);

        // The invalid A-bus read is open bus, not the $2100 register's $0F.
        assert_eq!(bus.ppu().cgram()[0], 0x01);
    }

    #[test]
    fn mmio_is_not_mirrored_into_cartridge_banks() {
        let mut rom = make_cart(Mapper::LoRom).rom().to_vec();
        rom[0x2100] = 0x5A;
        let cartridge = Cartridge::from_bytes(rom, None).unwrap();
        let mut bus = SystemBus::default();
        bus.install_cartridge(cartridge);

        bus.write(0x002100, 0x0F);
        bus.write(0x402100, 0x80); // LoROM address, not PPU MMIO.
        assert_eq!(bus.ppu().read_register(0x2100), 0x0F);
        assert_eq!(bus.read(0x402100), 0x5A);
        assert_eq!(bus.read(0x002100), 0x0F);
        bus.write(0x40420B, 0x01); // Not the DMA enable register.
        assert_eq!(bus.dma.transfer_count, 0);
    }

    #[test]
    fn cpu_visible_vram_ports_store_tilemap_data() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x002115, 0x80); // Increment after the high VRAM data port.
        bus.write(0x002116, 0x00);
        bus.write(0x002117, 0x00);
        bus.write(0x002118, 0x34);
        bus.write(0x002119, 0x12);

        assert_eq!(&bus.ppu().vram()[..2], &[0x34, 0x12]);
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
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT);

        assert_eq!(&bus.ppu().cgram()[..2], &[0x00, 0x7C]);
    }

    #[test]
    fn hdma_repeating_direct_block_transfers_on_first_and_every_next_line() {
        let mut bus = SystemBus::default();
        // $83 = 3 lines, one TM write from fresh table data each scanline.
        for (offset, value) in [(0, 0x83), (1, 0x01), (2, 0x05), (3, 0x10), (4, 0x00)] {
            bus.write(0x7E0100 + offset, value);
        }
        bus.write(0x004300, 0x00);
        bus.write(0x004301, 0x2C);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x00420C, 0x01);
        let clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        for (expected, transfers) in [(0x01, 1), (0x05, 2), (0x10, 3)] {
            bus.advance_master_clocks(clocks);
            assert_eq!(bus.ppu().read_register(0x212C), expected);
            assert_eq!(bus.dma.transfer_count, transfers);
        }
        assert!(!bus.dma.channel(0).unwrap().hdma_active);
    }

    #[test]
    fn hdma_nonrepeating_block_transfers_once_then_waits_for_next_descriptor() {
        let mut bus = SystemBus::default();
        // $03 = one TM write then wait three lines; $01 = next block.
        for (offset, value) in [(0, 0x03), (1, 0x05), (2, 0x01), (3, 0x10), (4, 0x00)] {
            bus.write(0x7E0100 + offset, value);
        }
        bus.write(0x004300, 0x00);
        bus.write(0x004301, 0x2C);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x00420C, 0x01);
        let clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        for (expected, transfers) in [(0x05, 1), (0x05, 1), (0x05, 1), (0x10, 2)] {
            bus.advance_master_clocks(clocks);
            assert_eq!(bus.ppu().read_register(0x212C), expected);
            assert_eq!(bus.dma.transfer_count, transfers);
        }
        assert!(!bus.dma.channel(0).unwrap().hdma_active);
    }

    #[test]
    fn hdma_descriptor_80_waits_128_lines_after_first_write() {
        let mut bus = SystemBus::default();
        for (offset, value) in [(0, 0x80), (1, 0x05), (2, 0x01), (3, 0x10), (4, 0x00)] {
            bus.write(0x7E0100 + offset, value);
        }
        bus.write(0x004300, 0x00);
        bus.write(0x004301, 0x2C);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x00420C, 0x01);
        let clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        for line in 1..=129 {
            bus.advance_master_clocks(clocks);
            if line <= 128 {
                assert_eq!(bus.ppu().read_register(0x212C), 0x05);
                assert_eq!(bus.dma.transfer_count, 1);
            }
        }
        assert_eq!(bus.ppu().read_register(0x212C), 0x10);
        assert_eq!(bus.dma.transfer_count, 2);
    }

    #[test]
    fn hdma_repeating_indirect_block_advances_data_pointer_each_line() {
        let mut bus = SystemBus::default();
        // A direct HDMA table in bank 7E points into distinct data in bank 7F.
        for (offset, value) in [(0, 0x82), (1, 0x00), (2, 0x02), (3, 0x00)] {
            bus.write(0x7E0100 + offset, value);
        }
        bus.write(0x7F0200, 0x01);
        bus.write(0x7F0201, 0x05);
        bus.write(0x004300, 0x40);
        bus.write(0x004301, 0x2C);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x004307, 0x7F);
        bus.write(0x00420C, 0x01);
        let clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(clocks);
        assert_eq!(bus.ppu().read_register(0x212C), 0x01);
        bus.advance_master_clocks(clocks);
        assert_eq!(bus.ppu().read_register(0x212C), 0x05);
        assert_eq!(bus.dma.transfer_count, 2);
        assert!(!bus.dma.channel(0).unwrap().hdma_active);
    }

    #[test]
    fn hdma_indirect_uses_dedicated_indirect_bank() {
        let mut bus = SystemBus::default();
        bus.install_cartridge(make_cart(Mapper::LoRom));
        bus.write(0x002121, 0x00);

        // HDMA table in bank 7E points at data in bank 7F.
        bus.write(0x7E0100, 0x01);
        bus.write(0x7E0101, 0x00);
        bus.write(0x7E0102, 0x02);
        bus.write(0x7F0200, 0x00);
        bus.write(0x7F0201, 0x7C);

        bus.write(0x004300, 0x42);
        bus.write(0x004301, 0x22);
        bus.write(0x004302, 0x00);
        bus.write(0x004303, 0x01);
        bus.write(0x004304, 0x7E);
        bus.write(0x004307, 0x7F);
        bus.write(0x00420C, 0x01);
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT);

        assert_eq!(&bus.ppu().cgram()[..2], &[0x00, 0x7C]);
    }

    #[test]
    fn chrono_field_hv_reads_return_current_scanline_not_zero() {
        let mut bus = SystemBus::default();
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * 86 + 40);
        // Chrono Trigger Field_ReadHVLatch at C0:5A46: STAT78 resets the
        // output selector, then SLHV latches the position, then two OPVCT
        // reads construct a 9-bit vertical counter.
        let _ = bus.read(0x00213F);
        let _ = bus.read(0x002137);
        let low = bus.read(0x00213D);
        let high = bus.read(0x00213D) & 1;
        assert_eq!(u16::from(low) | (u16::from(high) << 8), 86);
        assert_eq!(bus.read(0x00213D), 86);

        bus.advance_master_clocks(line_clocks * 172);
        let _ = bus.read(0x00213F);
        let _ = bus.read(0x002137);
        let low = bus.read(0x00213D);
        let high = bus.read(0x00213D) & 1;
        assert_eq!(u16::from(low) | (u16::from(high) << 8), 258);
    }

    #[test]
    fn hv_latch_requires_wrio_high_and_read_byte_toggles_reset_on_stat78() {
        let mut bus = SystemBus::default();
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * 112 + 20);
        bus.write(0x004201, 0x00); // 1 -> 0 transition latches line 112.
        assert_eq!(bus.read(0x004213), 0);
        bus.advance_master_clocks(line_clocks * 10);
        let _ = bus.read(0x002137); // Disabled while WRIO bit 7 is zero.
        assert_eq!(bus.read(0x00213D), 112);
        assert_eq!(bus.read(0x00213D) & 1, 0);
        assert_eq!(bus.read(0x00213D), 112);

        // STAT78 independently resets both horizontal and vertical toggles.
        let _ = bus.read(0x00213C);
        let _ = bus.read(0x00213F);
        assert_eq!(bus.read(0x00213D), 112);
        assert_eq!(bus.read(0x00213C), 5); // Dot coordinate: 20 clocks / 4.
        bus.write(0x004201, 0x80);
        let _ = bus.read(0x002137);
        assert_eq!(bus.read(0x00213D) & 1, 0); // High byte after prior low.
        let _ = bus.read(0x00213F);
        assert_eq!(bus.read(0x00213D), 122);
    }

    #[test]
    fn mode1_raster_preserves_early_background_even_if_later_lines_disable_it() {
        let mut bus = SystemBus::default();
        bus.write(0x002100, 0x0F);
        bus.write(0x002105, 0x01);
        bus.write(0x00212C, 0x01);
        bus.write(0x002121, 0);
        bus.write(0x002122, 0x1F);
        bus.write(0x002122, 0x00);
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * 64);
        bus.write(0x00212C, 0x10);
        bus.write(0x002121, 0);
        bus.write(0x002122, 0);
        bus.write(0x002122, 0x7C);
        bus.advance_master_clocks(line_clocks * u64::from(NTSC_SCANLINES_PER_FRAME - 64));
        assert!(bus.completed_raster_frame.is_some());
        let mut frame = FrameBuffer::default();
        bus.render_frame(&mut frame);
        // Earlier raster pixels are red; after TM switches to sprites-only
        // the PPU's backdrop changes to blue.
        assert_eq!(&frame.pixels()[0..4], &[248, 0, 0, 255]);
        assert_eq!(
            &frame.pixels()[100 * 256 * 4..100 * 256 * 4 + 4],
            &[0, 0, 248, 255]
        );
    }

    #[test]
    fn mode2_shop_raster_keeps_early_rows_when_vblank_changes_palette() {
        let mut bus = SystemBus::default();
        bus.write(0x002100, 0x0F);
        bus.write(0x002105, 2); // The merchant UI uses Mode 2.
        bus.write(0x002121, 0);
        bus.write(0x002122, 0x1F);
        bus.write(0x002122, 0x00); // Red backdrop in the first 64 lines.
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * 64);
        bus.write(0x002121, 0);
        bus.write(0x002122, 0);
        bus.write(0x002122, 0x7C); // Blue backdrop below that.
        bus.advance_master_clocks(line_clocks * u64::from(NTSC_SCANLINES_PER_FRAME - 64));

        assert!(bus.completed_raster_frame.is_some());
        let mut frame = FrameBuffer::default();
        bus.render_frame(&mut frame);
        assert_eq!(&frame.pixels()[0..4], &[248, 0, 0, 255]);
        assert_eq!(
            &frame.pixels()[100 * 256 * 4..100 * 256 * 4 + 4],
            &[0, 0, 248, 255]
        );
    }

    #[test]
    fn mode7_does_not_use_incomplete_mode1_raster_capture() {
        let mut bus = SystemBus::default();
        bus.write(0x002100, 0x0F);
        bus.write(0x002105, 7);
        bus.write(0x00212C, 1);
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * u64::from(NTSC_SCANLINES_PER_FRAME));
        assert!(bus.completed_raster_frame.is_none());
        let mut actual = FrameBuffer::default();
        let mut expected = FrameBuffer::default();
        bus.render_frame(&mut actual);
        bus.ppu.render_frame(&mut expected);
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn visible_ppu_snapshot_preserves_pre_vblank_background_and_palette() {
        let mut bus = SystemBus::default();
        bus.write(0x002100, 0x0F); // Visible, full brightness.
        bus.write(0x002105, 0x01); // Mode 1, BG1 is 4bpp.
        bus.write(0x00212C, 0x01); // Main screen BG1.
        // BG1 tile map entry 0 refers to tile 1.
        bus.write(0x002115, 0x80);
        bus.write(0x002116, 0);
        bus.write(0x002117, 0);
        bus.write(0x002118, 1);
        bus.write(0x002119, 0);
        // Tile 1 uses four-plane data beginning at VRAM byte 32.
        bus.write(0x002116, 16);
        bus.write(0x002117, 0);
        bus.write(0x002118, 0x80);
        bus.write(0x002119, 0);
        // Color index 1 is bright red.
        bus.write(0x002121, 1);
        bus.write(0x002122, 0x1F);
        bus.write(0x002122, 0x00);
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(line_clocks * 211);
        let snapshot = bus.visible_ppu_snapshot.as_ref().unwrap();
        assert_eq!(snapshot.read_register(0x212C), 1);
        let mut expected = FrameBuffer::default();
        snapshot.render_frame(&mut expected);
        assert_eq!(&expected.pixels()[..4], &[248, 0, 0, 255]);

        // After VBlank begins, the game makes visible BG1 disappear and
        // overwrites VRAM and CGRAM for its *next* scene.
        bus.write(0x00212C, 0x10);
        bus.write(0x002121, 1);
        bus.write(0x002122, 0);
        bus.write(0x002122, 0);
        bus.write(0x002116, 16);
        bus.write(0x002117, 0);
        bus.write(0x002118, 0);
        bus.write(0x002119, 0);
        let mut frame = FrameBuffer::default();
        bus.render_frame(&mut frame);
        assert_eq!(frame.pixels(), expected.pixels());
        assert_eq!(bus.ppu.read_register(0x212C), 0x10);
    }

    #[test]
    fn ordinary_background_frames_keep_the_existing_frame_end_renderer() {
        let mut bus = SystemBus::default();
        bus.write(0x002100, 0x0F);
        bus.write(0x002105, 0x07);
        bus.write(0x00212C, 1);
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT * 211);
        assert!(bus.visible_ppu_snapshot.is_some());
        // Although an earlier snapshot exists, a still-visible BG at frame
        // end continues down the exact legacy render path.
        bus.write(0x002132, 0x3F);
        let mut actual = FrameBuffer::default();
        let mut expected = FrameBuffer::default();
        bus.render_frame(&mut actual);
        bus.ppu.render_frame(&mut expected);
        assert_eq!(actual.pixels(), expected.pixels());
        // Mode7 is never switched to a different raster source by this
        // narrowly scoped Mode1 correction, even if VBlank disables BG1.
        bus.write(0x00212C, 0x10);
        let mut mode7_actual = FrameBuffer::default();
        let mut mode7_expected = FrameBuffer::default();
        bus.render_frame(&mut mode7_actual);
        bus.ppu.render_frame(&mut mode7_expected);
        assert_eq!(mode7_actual.pixels(), mode7_expected.pixels());
        bus.reset();
        assert!(bus.visible_ppu_snapshot.is_none());
    }

    #[test]
    fn nmi_diagnostic_counts_only_enabled_edges_and_cpu_dispatches() {
        let mut bus = SystemBus::default();
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        // No VBlank interrupt when disabled.
        bus.advance_master_clocks(line_clocks * u64::from(VBLANK_START_SCANLINE));
        assert_eq!(bus.observability().nmi_counts(), (0, 0));
        // Leave VBlank before enabling; otherwise enabling NMI during VBlank
        // immediately asserts an edge on real SNES hardware.
        bus.advance_master_clocks(
            line_clocks * u64::from(NTSC_SCANLINES_PER_FRAME - VBLANK_START_SCANLINE),
        );
        bus.write(0x004200, 0x80);
        bus.advance_master_clocks(line_clocks * u64::from(VBLANK_START_SCANLINE));
        assert_eq!(bus.observability().nmi_counts(), (1, 0));
        assert!(bus.poll_nmi());
        assert_eq!(bus.observability().nmi_counts(), (1, 1));
        assert!(!bus.poll_nmi());
        assert_eq!(bus.observability().nmi_counts(), (1, 1));
    }

    #[test]
    fn h_timer_irq_only_triggers_at_programmed_dot() {
        let mut bus = SystemBus::default();
        bus.write(0x004200, 0x10);
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * 3 * MASTER_CLOCKS_PER_DOT);
        // Default HTIME is out of range: no phantom scanline IRQs.
        assert_eq!(bus.read(0x004211) & 0x80, 0);

        bus.write(0x004207, 32);
        bus.write(0x004208, 0);
        bus.advance_master_clocks(31 * MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0);
        bus.advance_master_clocks(MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        assert_eq!(bus.read(0x004211) & 0x80, 0);

        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        bus.write(0x004200, 0); // Disabling IRQ acknowledges it.
        assert_eq!(bus.read(0x004211) & 0x80, 0);
        assert!(!bus.poll_irq());
    }

    #[test]
    fn v_timer_irq_fires_once_per_frame_at_programmed_scanline() {
        let mut bus = SystemBus::default();
        bus.write(0x004209, 2);
        bus.write(0x00420A, 0);
        bus.write(0x004200, 0x20);
        let dots = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(dots * 2 - 1);
        assert_eq!(bus.read(0x004211) & 0x80, 0);
        bus.advance_master_clocks(MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        bus.advance_master_clocks(dots * 4);
        assert_eq!(bus.read(0x004211) & 0x80, 0);
        bus.advance_master_clocks(dots * u64::from(NTSC_SCANLINES_PER_FRAME - 4));
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
    }

    #[test]
    fn hv_timer_irq_requires_both_programmed_coordinates() {
        let mut bus = SystemBus::default();
        bus.write(0x004207, 20);
        bus.write(0x004208, 0);
        bus.write(0x004209, 2);
        bus.write(0x00420A, 0);
        bus.write(0x004200, 0x30);
        let target = (u64::from(DOTS_PER_SCANLINE) * 2 + 20) * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(target - 1);
        assert_eq!(bus.read(0x004211) & 0x80, 0);
        bus.advance_master_clocks(MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * 5 * MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0);
    }

    #[test]
    fn raises_nmi_and_irq_from_timing_progression() {
        let mut bus = SystemBus::default();
        bus.write(0x004207, 32);
        bus.write(0x004208, 0);
        bus.write(0x004200, 0x90);

        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x004211) & 0x80, 0x80);
        assert_eq!(bus.read(0x004211) & 0x80, 0x00);

        let clocks_to_vblank = u64::from(DOTS_PER_SCANLINE)
            * u64::from(VBLANK_START_SCANLINE - 1)
            * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(clocks_to_vblank);
        assert!(bus.timing().in_vblank());
        assert_eq!(bus.read(0x004210) & 0x80, 0x80);
        assert_eq!(bus.read(0x004210) & 0x80, 0x00);
    }

    #[test]
    fn hvbjoy_hblank_bit_stays_high_through_horizontal_blanking() {
        use crate::timing::HBLANK_START_DOT;

        let mut bus = SystemBus::default();
        let dot_clocks = MASTER_CLOCKS_PER_DOT;
        assert_eq!(bus.read(0x004212) & 0x40, 0);

        bus.advance_master_clocks(u64::from(HBLANK_START_DOT - 1) * dot_clocks);
        assert_eq!(bus.timing().dot, HBLANK_START_DOT - 1);
        assert_eq!(bus.read(0x004212) & 0x40, 0);

        bus.advance_master_clocks(dot_clocks);
        assert_eq!(bus.timing().dot, HBLANK_START_DOT);
        assert_eq!(bus.read(0x004212) & 0x40, 0x40);

        bus.advance_master_clocks(u64::from(DOTS_PER_SCANLINE - HBLANK_START_DOT - 1) * dot_clocks);
        assert_eq!(bus.timing().dot, DOTS_PER_SCANLINE - 1);
        assert_eq!(bus.read(0x004212) & 0x40, 0x40);

        bus.advance_master_clocks(dot_clocks);
        assert_eq!(bus.timing().dot, 0);
        assert_eq!(bus.read(0x004212) & 0x40, 0);
    }

    #[test]
    fn hvbjoy_hblank_is_exposed_on_mirrored_io_banks_without_changing_vblank() {
        let mut bus = SystemBus::default();
        bus.advance_master_clocks(u64::from(HBLANK_START_DOT) * MASTER_CLOCKS_PER_DOT);
        assert_eq!(bus.read(0x804212) & 0x40, 0x40);
        assert_eq!(bus.read(0x004212) & 0x80, 0);
        bus.advance_master_clocks(
            u64::from(DOTS_PER_SCANLINE)
                * u64::from(crate::timing::VBLANK_START_SCANLINE)
                * MASTER_CLOCKS_PER_DOT,
        );
        assert!(bus.timing().in_vblank());
        assert_eq!(bus.read(0x004212) & 0x80, 0x80);
    }

    #[test]
    fn wraps_frames_through_explicit_timing() {
        let mut bus = SystemBus::default();
        let frame_clocks = u64::from(DOTS_PER_SCANLINE)
            * u64::from(NTSC_SCANLINES_PER_FRAME)
            * MASTER_CLOCKS_PER_DOT;
        bus.advance_master_clocks(frame_clocks);

        assert_eq!(bus.timing().frame, 1);
        assert_eq!(bus.timing().scanline, 0);
        assert_eq!(bus.timing().dot, 0);
    }

    #[test]
    fn ppu_register_writes_drive_rendered_frame() {
        let mut bus = SystemBus::default();
        let mut frame = FrameBuffer::default();
        bus.write(0x002100, 0x0F);
        bus.write(0x002121, 0x00);
        bus.write(0x002122, 0x1F);
        bus.write(0x002122, 0x00);
        bus.write(0x00212C, 0x01);
        bus.render_frame(&mut frame);

        assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 0xFF]);
    }

    #[test]
    fn samples_active_scanline_display_registers_before_end_of_frame() {
        let mut bus = SystemBus::default();
        let line_clocks = u64::from(DOTS_PER_SCANLINE) * MASTER_CLOCKS_PER_DOT;
        bus.write(0x002100, 0x0F);
        bus.write(0x00212C, 0x01);
        for line in 1..=261 {
            if line == 112 {
                bus.write(0x00212C, 0x10);
            }
            bus.advance_master_clocks(line_clocks);
        }
        bus.advance_master_clocks(line_clocks);
        let samples = bus.observability().completed_ppu_scanline_samples();
        assert_eq!(samples.iter().find(|s| s[0] == 64).unwrap()[3], 0x01);
        assert_eq!(samples.iter().find(|s| s[0] == 128).unwrap()[3], 0x10);
        assert_eq!(samples.iter().find(|s| s[0] == 128).unwrap()[1], 0x0F);
    }
}
