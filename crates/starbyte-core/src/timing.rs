//! Global timing model scaffolding.

use serde::{Deserialize, Serialize};

/// Master dots per scanline in the bootstrap NTSC timing model.
pub const DOTS_PER_SCANLINE: u16 = 341;
/// Scanlines per NTSC frame in the bootstrap timing model.
pub const NTSC_SCANLINES_PER_FRAME: u16 = 262;
/// First scanline treated as vertical blank.
pub const VBLANK_START_SCANLINE: u16 = 225;

/// High-level timing counters shared across subsystems.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingState {
    /// Master clock ticks.
    pub master_clock: u64,
    /// Scanline number in the current frame.
    pub scanline: u16,
    /// Dot within the current scanline.
    pub dot: u16,
    /// Frame counter.
    pub frame: u64,
}

/// Notable timing transitions produced while advancing the clock.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TimingEvents {
    /// True when one or more scanline boundaries were crossed.
    pub crossed_scanline: bool,
    /// True when VBlank started during this advance.
    pub entered_vblank: bool,
    /// True when a frame boundary was crossed.
    pub started_new_frame: bool,
}

impl TimingState {
    /// Advance master-clock time and surface major timing transitions.
    pub fn advance_master_clocks(&mut self, clocks: u64) -> TimingEvents {
        let mut events = TimingEvents::default();

        for _ in 0..clocks {
            self.master_clock = self.master_clock.saturating_add(1);
            self.dot = self.dot.wrapping_add(1);

            if self.dot < DOTS_PER_SCANLINE {
                continue;
            }

            self.dot = 0;
            self.scanline = self.scanline.wrapping_add(1);
            events.crossed_scanline = true;

            if self.scanline == VBLANK_START_SCANLINE {
                events.entered_vblank = true;
            }

            if self.scanline < NTSC_SCANLINES_PER_FRAME {
                continue;
            }

            self.scanline = 0;
            self.frame = self.frame.saturating_add(1);
            events.started_new_frame = true;
        }
        events
    }

    /// Whether the timing position is currently inside the vertical blank interval.
    #[must_use]
    pub const fn in_vblank(&self) -> bool {
        self.scanline >= VBLANK_START_SCANLINE
    }
}
