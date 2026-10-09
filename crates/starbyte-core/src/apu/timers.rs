//! SPC700's three timer channels, clocked independently of SNES CPU instructions.
//!
//! This models control, target, and four-bit read-to-clear output counters.
//! Timer 0/1 receive 8 kHz ticks, timer 2 receives 64 kHz ticks.
//! The audio processor's nominal 1.024 MHz clock is converted from the NTSC
//! master clock with a persisted fractional phase to avoid chunking drift.

use serde::{Deserialize, Serialize};

const NTSC_MASTER_CLOCK_HZ: u128 = 21_477_272;
const SPC_CLOCK_HZ: u128 = 1_024_000;
const TIMER_DIVIDERS: [u64; 3] = [128, 128, 16];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct SpcTimers {
    channels: [Timer; 3],
    #[serde(default)]
    spc_clock_fraction: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
struct Timer {
    target: u8,
    enabled: bool,
    divider_phase: u64,
    stage: u16,
    output: u8,
}

impl SpcTimers {
    pub(super) fn write_control(&mut self, value: u8) {
        for (index, timer) in self.channels.iter_mut().enumerate() {
            let enabled = value & (1 << index) != 0;
            if enabled && !timer.enabled {
                timer.divider_phase = 0;
                timer.stage = 0;
                timer.output = 0;
            }
            timer.enabled = enabled;
        }
    }

    pub(super) fn write_target(&mut self, index: usize, value: u8) {
        self.channels[index].target = value;
    }

    pub(super) fn read_and_clear(&mut self, index: usize) -> u8 {
        let output = self.channels[index].output;
        self.channels[index].output = 0;
        output
    }

    pub(super) fn advance_master_cycles(&mut self, master_cycles: u64) {
        let total = u128::from(self.spc_clock_fraction)
            + u128::from(master_cycles) * SPC_CLOCK_HZ;
        let cycles = (total / NTSC_MASTER_CLOCK_HZ) as u64;
        self.spc_clock_fraction = (total % NTSC_MASTER_CLOCK_HZ) as u64;
        self.advance_spc_cycles(cycles);
    }

    fn advance_spc_cycles(&mut self, cycles: u64) {
        for (timer, divider) in self.channels.iter_mut().zip(TIMER_DIVIDERS) {
            // The internal divider runs at the same SPC clock as all other
            // channels but is reset when this simplified timer is enabled.
            if !timer.enabled {
                continue;
            }
            let total = timer.divider_phase + cycles;
            let ticks = total / divider;
            timer.divider_phase = total % divider;
            let target = if timer.target == 0 {
                256_u64
            } else {
                u64::from(timer.target)
            };
            let stage = u64::from(timer.stage) + ticks;
            let output_increments = stage / target;
            timer.stage = (stage % target) as u16;
            timer.output = (u64::from(timer.output) + output_increments) as u8 & 0x0f;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SpcTimers;

    #[test]
    fn independent_timer_rates_and_read_to_clear() {
        let mut timers = SpcTimers::default();
        timers.write_target(0, 4);
        timers.write_target(1, 4);
        timers.write_target(2, 4);
        timers.write_control(0b101);
        timers.advance_spc_cycles(128);
        assert_eq!(timers.read_and_clear(0), 0); // 1 tick out of 4
        assert_eq!(timers.read_and_clear(1), 0); // disabled
        assert_eq!(timers.read_and_clear(2), 2); // 8 ticks / 4
        assert_eq!(timers.read_and_clear(2), 0);
        timers.advance_spc_cycles(384);
        assert_eq!(timers.read_and_clear(0), 1);
        assert_eq!(timers.read_and_clear(2), 6);
    }

    #[test]
    fn target_zero_means_256_and_output_wraps_to_four_bits() {
        let mut timers = SpcTimers::default();
        timers.write_target(0, 0);
        timers.write_control(1);
        timers.advance_spc_cycles(128 * 255);
        assert_eq!(timers.read_and_clear(0), 0);
        timers.advance_spc_cycles(128);
        assert_eq!(timers.read_and_clear(0), 1);
        timers.advance_spc_cycles(128 * 256 * 17);
        assert_eq!(timers.read_and_clear(0), 1);
    }

    #[test]
    fn enabling_resets_counters_and_disabling_stops_ticks() {
        let mut timers = SpcTimers::default();
        timers.write_target(0, 1);
        timers.write_control(1);
        timers.advance_spc_cycles(128 * 5);
        timers.write_control(0);
        timers.advance_spc_cycles(128 * 5);
        assert_eq!(timers.read_and_clear(0), 5);
        timers.write_control(1);
        assert_eq!(timers.read_and_clear(0), 0);
        timers.advance_spc_cycles(128);
        assert_eq!(timers.read_and_clear(0), 1);
    }

    #[test]
    fn fractional_ntsc_clock_is_chunking_and_save_state_invariant() {
        let mut whole = SpcTimers::default();
        let mut small = SpcTimers::default();
        for timers in [&mut whole, &mut small] {
            timers.write_target(0, 39);
            timers.write_target(2, 251);
            timers.write_control(0b101);
        }
        let one_second = 21_477_272;
        whole.advance_master_cycles(one_second);
        for _ in 0..(one_second / 256) {
            small.advance_master_cycles(256);
        }
        small.advance_master_cycles(one_second % 256);
        assert_eq!(whole.read_and_clear(0), small.read_and_clear(0));
        assert_eq!(whole.read_and_clear(2), small.read_and_clear(2));
        let saved = serde_json::to_string(&small).unwrap();
        let mut resumed: SpcTimers = serde_json::from_str(&saved).unwrap();
        for master_clocks in [1, 50, 781, 21_477, 500_000] {
            small.advance_master_cycles(master_clocks);
            resumed.advance_master_cycles(master_clocks);
            assert_eq!(small.read_and_clear(0), resumed.read_and_clear(0));
            assert_eq!(small.read_and_clear(2), resumed.read_and_clear(2));
        }
    }
}
