//! SPC700's three timer channels, clocked independently of SNES CPU instructions.
//!
//! This models control, target, and four-bit read-to-clear output counters.
//! Timer 0/1 receive 8 kHz ticks, timer 2 receives 64 kHz ticks.
//! Low-level dividers never stop or reset; enabling clears only the timer's
//! comparator stage and output. Target writes use 8-bit equality semantics.
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
                // Only stages 2/3 reset on a rising enable edge. The shared
                // low-level divider runs even while a timer is disabled.
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
        let total = u128::from(self.spc_clock_fraction) + u128::from(master_cycles) * SPC_CLOCK_HZ;
        let cycles = (total / NTSC_MASTER_CLOCK_HZ) as u64;
        self.spc_clock_fraction = (total % NTSC_MASTER_CLOCK_HZ) as u64;
        self.advance_spc_cycles(cycles);
    }

    fn advance_spc_cycles(&mut self, cycles: u64) {
        for (timer, divider) in self.channels.iter_mut().zip(TIMER_DIVIDERS) {
            // Stage 1 is free-running regardless of CONTROL enable state.
            // Timers 0/1 share a 128-cycle edge; timer 2 divides by 16.
            let total = timer.divider_phase + cycles;
            let ticks = total / divider;
            timer.divider_phase = total % divider;
            if !timer.enabled || ticks == 0 {
                continue;
            }

            // Stage 2 is an eight-bit *equality comparator*, not a modulo
            // counter. If software lowers TARGET below the current stage,
            // the first match is after an eight-bit wrap, not immediately.
            // The target value zero corresponds to a full 256-tick period.
            let stage = u64::from(timer.stage);
            let period = if timer.target == 0 {
                256
            } else {
                u64::from(timer.target)
            };
            let until_first = if stage < period {
                period - stage
            } else {
                256 - stage + period
            };
            if ticks < until_first {
                timer.stage = ((stage + ticks) & 0xff) as u16;
                continue;
            }

            let remaining = ticks - until_first;
            timer.stage = (remaining % period) as u16;
            let increments = 1 + remaining / period;
            timer.output = ((u64::from(timer.output) + increments) & 0x0f) as u8;
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

    #[test]
    fn disabled_low_level_divider_continues_through_enable_edges() {
        let mut timers = SpcTimers::default();
        timers.write_target(2, 1);
        // Timer 2's stage-1 clock is one tick every 16 SPC cycles.
        // Enabling at cycle 15 must deliver its first tick one cycle later.
        timers.advance_spc_cycles(15);
        timers.write_control(0b100);
        timers.advance_spc_cycles(1);
        assert_eq!(timers.read_and_clear(2), 1);

        // A second enable edge clears stages 2/3, not the divider phase.
        timers.write_control(0);
        timers.advance_spc_cycles(7);
        timers.write_control(0b100);
        timers.advance_spc_cycles(8);
        assert_eq!(timers.read_and_clear(2), 0);
        timers.advance_spc_cycles(1);
        assert_eq!(timers.read_and_clear(2), 1);
    }

    #[test]
    fn changing_target_below_current_stage_waits_for_eight_bit_wrap() {
        let mut timers = SpcTimers::default();
        timers.write_target(0, 200);
        timers.write_control(1);
        timers.advance_spc_cycles(128 * 180);
        assert_eq!(timers.read_and_clear(0), 0);

        // The counter is at 180. TARGET=40 will next match after
        // (256-180)+40 = 116 stage-1 ticks, not after 40 ticks.
        timers.write_target(0, 40);
        timers.advance_spc_cycles(128 * 100);
        assert_eq!(timers.read_and_clear(0), 0);
        timers.advance_spc_cycles(128 * 16);
        assert_eq!(timers.read_and_clear(0), 1);
        timers.advance_spc_cycles(128 * 40);
        assert_eq!(timers.read_and_clear(0), 1);
    }

    #[test]
    fn batched_timer_comparator_matches_tick_by_tick_reference() {
        for target in [0_u8, 1, 40, 200, 255] {
            for start in [0_u8, 1, 39, 40, 127, 128, 255] {
                for ticks in [0_u64, 1, 2, 15, 16, 255, 1024] {
                    let mut timers = SpcTimers::default();
                    timers.write_target(0, target);
                    timers.write_control(1);
                    // Simulate a live TARGET rewrite after stage 2 had
                    // reached any of these possible values.
                    timers.channels[0].stage = u16::from(start);
                    timers.advance_spc_cycles(128 * ticks);

                    let mut expected_stage = start;
                    let mut expected_output = 0_u8;
                    for _ in 0..ticks {
                        expected_stage = expected_stage.wrapping_add(1);
                        if expected_stage == target {
                            expected_stage = 0;
                            expected_output = (expected_output + 1) & 0x0f;
                        }
                    }
                    assert_eq!(
                        timers.channels[0].stage,
                        u16::from(expected_stage),
                        "stage: target={target}, start={start}, ticks={ticks}"
                    );
                    assert_eq!(
                        timers.read_and_clear(0),
                        expected_output,
                        "output: target={target}, start={start}, ticks={ticks}"
                    );
                }
            }
        }
    
}
