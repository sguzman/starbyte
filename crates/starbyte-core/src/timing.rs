//! Global timing model scaffolding.

use serde::{Deserialize, Serialize};

/// Approximate PPU dot cycles per NTSC scanline.
pub const DOTS_PER_SCANLINE: u16 = 341;
/// Approximate master clocks per PPU dot (1364 per NTSC scanline).
/// The long-dot/short-scanline exceptions remain unsupported.
pub const MASTER_CLOCKS_PER_DOT: u64 = 4;
/// Scanlines per NTSC frame in the bootstrap timing model.
pub const NTSC_SCANLINES_PER_FRAME: u16 = 262;
/// First scanline treated as vertical blank.
pub const VBLANK_START_SCANLINE: u16 = 225;

/// High-level timing counters shared across subsystems.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingState {
    /// Master clock ticks, including the sub-dot remainder.
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
        if clocks == 0 {
            return TimingEvents::default();
        }

        let dots_per_scanline = u64::from(DOTS_PER_SCANLINE);
        let scanlines_per_frame = u64::from(NTSC_SCANLINES_PER_FRAME);
        let dots_per_frame = dots_per_scanline * scanlines_per_frame;
        let start_dot_index = u64::from(self.scanline) * dots_per_scanline + u64::from(self.dot);
        // CPU/APU time is measured in master clocks, not PPU dots. In the
        // approximate NTSC model one dot spans four clocks. Preserve the
        // sub-dot remainder using the existing absolute master_clock count.
        let sub_dot_clocks = self.master_clock % MASTER_CLOCKS_PER_DOT;
        let elapsed_dots = (sub_dot_clocks + clocks) / MASTER_CLOCKS_PER_DOT;
        let end_dot_index = start_dot_index + elapsed_dots;

        let start_frame_offset = start_dot_index / dots_per_frame;
        let end_frame_offset = end_dot_index / dots_per_frame;
        let start_scanline_index = start_dot_index / dots_per_scanline;
        let end_scanline_index = end_dot_index / dots_per_scanline;
        let wrapped_dot_index = end_dot_index % dots_per_frame;
        let end_scanline = wrapped_dot_index / dots_per_scanline;
        let end_dot = wrapped_dot_index % dots_per_scanline;

        let entered_vblank = if self.scanline >= VBLANK_START_SCANLINE {
            false
        } else if end_frame_offset > start_frame_offset {
            true
        } else {
            end_scanline >= u64::from(VBLANK_START_SCANLINE)
        };

        self.master_clock = self.master_clock.saturating_add(clocks);
        self.frame = self
            .frame
            .saturating_add(end_frame_offset.saturating_sub(start_frame_offset));
        self.scanline = end_scanline as u16;
        self.dot = end_dot as u16;

        TimingEvents {
            crossed_scanline: end_scanline_index > start_scanline_index,
            entered_vblank,
            started_new_frame: end_frame_offset > start_frame_offset,
        }
    }

    /// Whether the timing position is currently inside the vertical blank interval.
    #[must_use]
    pub const fn in_vblank(&self) -> bool {
        self.scanline >= VBLANK_START_SCANLINE
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DOTS_PER_SCANLINE, MASTER_CLOCKS_PER_DOT, NTSC_SCANLINES_PER_FRAME, TimingEvents,
        TimingState,
        VBLANK_START_SCANLINE,
    };

    #[test]
    fn zero_clock_advance_is_noop() {
        let mut timing = TimingState::default();
        let events = timing.advance_master_clocks(0);

        assert_eq!(timing, TimingState::default());
        assert_eq!(events, TimingEvents::default());
    }

    #[test]
    fn large_advances_cross_vblank_and_frame_arithmetically() {
        let mut timing = TimingState {
            master_clock: 0,
            scanline: VBLANK_START_SCANLINE - 2,
            dot: DOTS_PER_SCANLINE - 2,
            frame: 3,
        };
        let clocks = u64::from(DOTS_PER_SCANLINE) * 4 * MASTER_CLOCKS_PER_DOT;

        let events = timing.advance_master_clocks(clocks);

        assert!(events.crossed_scanline);
        assert!(events.entered_vblank);
        assert!(!events.started_new_frame);
        assert_eq!(timing.frame, 3);
        assert_eq!(timing.scanline, VBLANK_START_SCANLINE + 2);
    }

    #[test]
    fn master_clocks_accumulate_without_advancing_partial_dots() {
        let mut timing = TimingState::default();
        timing.advance_master_clocks(3);
        assert_eq!(timing.master_clock, 3);
        assert_eq!(timing.dot, 0);
        timing.advance_master_clocks(1);
        assert_eq!(timing.master_clock, 4);
        assert_eq!(timing.dot, 1);
        timing.advance_master_clocks(5);
        assert_eq!(timing.dot, 2);
        timing.advance_master_clocks(3);
        assert_eq!(timing.dot, 3);
    }

    #[test]
    fn advancing_341_master_clocks_does_not_complete_scanline() {
        let mut timing = TimingState::default();
        let events = timing.advance_master_clocks(u64::from(DOTS_PER_SCANLINE));
        assert!(!events.crossed_scanline);
        assert_eq!(timing.scanline, 0);
        assert_eq!(timing.dot, DOTS_PER_SCANLINE / 4);
        let events = timing.advance_master_clocks(
            u64::from(DOTS_PER_SCANLINE) * (MASTER_CLOCKS_PER_DOT - 1),
        );
        assert!(events.crossed_scanline);
        assert_eq!(timing.scanline, 1);
        assert_eq!(timing.dot, 0);
    }

    #[test]
    fn full_frame_advance_wraps_frame_counter() {
        let mut timing = TimingState::default();
        let clocks =
            u64::from(DOTS_PER_SCANLINE) * u64::from(NTSC_SCANLINES_PER_FRAME) * MASTER_CLOCKS_PER_DOT;

        let events = timing.advance_master_clocks(clocks);

        assert!(events.crossed_scanline);
        assert!(events.entered_vblank);
        assert!(events.started_new_frame);
        assert_eq!(timing.frame, 1);
        assert_eq!(timing.scanline, 0);
        assert_eq!(timing.dot, 0);
    }
}
