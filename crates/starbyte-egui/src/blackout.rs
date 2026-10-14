//! Passive, low-cost gameplay blackout diagnostics. Captures register snapshots
//! once when an already-visible game stays black for three seconds. A fade may
//! be intentional: this never pauses the guest or claims a confirmed deadlock.
use std::path::Path;

use starbyte_core::Emulator;
use tracing::warn;

const REPORT_AFTER_BLACK_FRAMES: u32 = 180;

#[derive(Debug, Default)]
pub struct BlackScreenMonitor {
    saw_visible_frame: bool,
    consecutive_black_frames: u32,
    last_frame: u64,
    reported_current_blackout: bool,
    /// Sparse 1s interval snapshots distinguish CPU wait loops from fades.
    blackout_checkpoints: Vec<String>,
}

impl BlackScreenMonitor {
    /// Sample framebuffer/CPU status after a successfully completed frame.
    pub fn observe(
        &mut self,
        emulator: &Emulator,
        cache_root: &Path,
        audio_pairs: usize,
        nonzero_pairs: usize,
    ) {
        let frame = emulator.timing().frame;
        let visible = emulator
            .framebuffer()
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[..3].iter().any(|&channel| channel != 0));
        let should_report = self.observe_visibility(frame, visible);
        if self.saw_visible_frame
            && self.consecutive_black_frames > 0
            && self.consecutive_black_frames.is_multiple_of(60)
            && self.blackout_checkpoints.len() < 8
        {
            let cpu = emulator.cpu_registers();
            let ppu_writes: u64 = emulator
                .system_observability()
                .ppu_write_counts()
                .iter()
                .map(|&n| u64::from(n))
                .sum();
            let ports = emulator.apu_to_cpu_ports();
            self.blackout_checkpoints.push(format!(
                "Frame {frame}: CPU={:02X}:{:04X} PPU-writes={ppu_writes} SPC-to-CPU={ports:?}",
                cpu.pbr, cpu.pc,
            ));
        }
        if !should_report {
            return;
        }

        let cpu = emulator.cpu_registers();
        let inidisp = emulator.peek_ppu_register(0x2100).unwrap_or(0);
        let bgmode = emulator.peek_ppu_register(0x2105).unwrap_or(0);
        let main_layers = emulator.peek_ppu_register(0x212C).unwrap_or(0);
        let sub_layers = emulator.peek_ppu_register(0x212D).unwrap_or(0);
        let (nmitimen, htime, vtime) = emulator.irq_timer_configuration();
        let ppu_writes: u64 = emulator
            .system_observability()
            .ppu_write_counts()
            .iter()
            .map(|&n| u64::from(n))
            .sum();
        let reads = emulator.system_observability().apu_port_read_counts();
        let writes = emulator.system_observability().apu_port_write_counts();
        let input_ports = emulator.cpu_to_apu_ports();
        let output_ports = emulator.apu_to_cpu_ports();
        let (host_buttons, game_latched_buttons, joypad_busy) = emulator.joypad_status();

        let report = format!(
            "Starbyte sustained-black-screen diagnostic (may be intentional fade)\n\
             Frame {frame}; black frames in a row {}\n\
             CPU {:02X}:{:04X}; A={:04X} X={:04X} Y={:04X} P={:02X}\n\
             PPU INIDISP={inidisp:02X} BGMODE={bgmode:02X} TM={main_layers:02X} TS={sub_layers:02X} writes={ppu_writes}\n\
             IRQ NMITIMEN={nmitimen:02X} HTIME={htime} VTIME={vtime}\n\
             APU CPU-to-SPC={input_ports:?}; SPC-to-CPU={output_ports:?}\n\
             APU MMIO reads={reads:?}; writes={writes:?}\n\
             Input host={host_buttons:04X} latched={game_latched_buttons:04X} busy={joypad_busy}\n\
             Audio last frame pairs={audio_pairs}; nonzero pairs={nonzero_pairs}\n\
             Blackout checkpoints (once per second):\n{}\n\
             No cartridge data, SRAM, or save-state bytes included.\n",
            self.consecutive_black_frames, cpu.pbr, cpu.pc, cpu.a, cpu.x, cpu.y, cpu.p,
            self.blackout_checkpoints.join("\n"),
        );
        let path = cache_root
            .join("diagnostics")
            .join(format!("black-screen-{frame}.txt"));
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            warn!(%error, "could not create black-screen diagnostic directory");
            return;
        }
        match std::fs::write(&path, report) {
            Ok(()) => warn!(
                path = %path.display(),
                frame,
                "saved sustained-black-screen diagnostic; game continues"
            ),
            Err(error) => warn!(%error, "could not write black-screen diagnostic"),
        }
    }

    fn observe_visibility(&mut self, frame: u64, visible: bool) -> bool {
        if frame <= self.last_frame {
            *self = Self::default(); // ROM reloaded or state moved backward.
        }
        self.last_frame = frame;
        if visible {
            self.saw_visible_frame = true;
            self.consecutive_black_frames = 0;
            self.reported_current_blackout = false;
            self.blackout_checkpoints.clear();
            return false;
        }
        if !self.saw_visible_frame {
            return false; // Normal startup black frames need no report.
        }
        self.consecutive_black_frames = self.consecutive_black_frames.saturating_add(1);
        if self.consecutive_black_frames >= REPORT_AFTER_BLACK_FRAMES
            && !self.reported_current_blackout
        {
            self.reported_current_blackout = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::BlackScreenMonitor;

    #[test]
    fn capture_once_per_blackout_only_after_prior_visible_frame() {
        let mut detector = BlackScreenMonitor::default();
        for frame in 1..=300 {
            assert!(!detector.observe_visibility(frame, false));
        }
        assert!(!detector.observe_visibility(301, true));
        for frame in 302..481 {
            assert!(!detector.observe_visibility(frame, false));
        }
        assert!(detector.observe_visibility(481, false));
        assert!(!detector.observe_visibility(482, false));
        assert!(!detector.observe_visibility(483, true));
        for frame in 484..663 {
            assert!(!detector.observe_visibility(frame, false));
        }
        assert!(detector.observe_visibility(663, false));
        // New ROM/reset must not treat a black boot as a continuation.
        assert!(!detector.observe_visibility(1, false));
        assert!(!detector.observe_visibility(200, false));
    }
}
