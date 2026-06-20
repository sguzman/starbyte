//! Global timing model scaffolding.

use serde::{Deserialize, Serialize};

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

impl TimingState {
    /// Advance one placeholder CPU step.
    pub fn tick_cpu_step(&mut self) {
        self.master_clock = self.master_clock.saturating_add(6);
        self.dot = self.dot.wrapping_add(1);
    }
}
