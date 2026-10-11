//! Partial SNES S-DSP voice mixer and BRR synthesis engine.
//!
//! This DSP consumes SPC700 audio RAM, not the SNES main CPU bus. It handles
//! directory-based BRR samples, pitch stepping and PMON, signed channel/master
//! volume, timed ADSR/GAIN envelopes, NON shared noise, and key-on/off/loop
//! state, RAM-backed echo/FIR, and four-tap Gaussian interpolation. Exact
//! hardware phase scheduling remains unfinished; this is not bit-exact audio.

use serde::{Deserialize, Serialize};

use super::gaussian;

const VOICE_COUNT: usize = 8;
const REGISTER_COUNT: usize = 128;
const BRR_SAMPLES_PER_BLOCK: usize = 16;
const PITCH_UNIT: u32 = 0x1000;
/// Shared 32 kHz envelope clock wraps after 30,720 DSP sample periods.
const ENVELOPE_COUNTER_WRAP: u16 = 30_720;
/// SPC DSP envelope periods in audio samples. Rate zero never clocks.
const ENVELOPE_PERIODS: [u16; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 40, 32,
    24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];
const ENVELOPE_OFFSETS: [u16; 32] = [
    0, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040,
    536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 0, 0,
];

const fn initial_noise_seed() -> u16 {
    0x4000
}

fn envelope_rate_ready(rate: u8, counter: u16) -> bool {
    let period = ENVELOPE_PERIODS[usize::from(rate)];
    period != 0 && (counter + ENVELOPE_OFFSETS[usize::from(rate)]) % period == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dsp {
    registers: Vec<u8>,
    voices: Vec<Voice>,
    /// Shared rate-table phase. Older save states resume at phase zero.
    #[serde(default)]
    envelope_counter: u16,
    /// Shared 15-bit noise LFSR. The sound chip powers on with seed $4000.
    #[serde(default = "initial_noise_seed")]
    noise_lfsr: u16,
    /// Echo ring offset is expressed in bytes, relative to ESA.
    #[serde(default)]
    echo_offset: u16,
    /// Eight stereo samples, newest at echo_history_index - 1.
    #[serde(default)]
    echo_history: [[i16; 2]; 8],
    #[serde(default)]
    echo_history_index: usize,
}

impl Default for Dsp {
    fn default() -> Self {
        // Prepare the exact Gaussian table while the APU is constructed,
        // rather than on the first live BRR voice's audio callback.
        gaussian::prewarm();
        let mut registers = vec![0; REGISTER_COUNT];
        // The synthetic/default mixer starts unmuted but must not overwrite
        // SPC low RAM with echo before a sound driver configures echo space.
        registers[0x6c] = 0x20;
        Self {
            registers,
            voices: vec![Voice::default(); VOICE_COUNT],
            envelope_counter: 0,
            noise_lfsr: initial_noise_seed(),
            echo_offset: 0,
            echo_history: [[0; 2]; 8],
            echo_history_index: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Voice {
    active: bool,
    releasing: bool,
    address: u16,
    header: u8,
    samples: [i16; BRR_SAMPLES_PER_BLOCK],
    /// Last three decoded BRR values from the previous 16-sample block.
    #[serde(default)]
    past_samples: [i16; 3],
    index: usize,
    fraction: u32,
    previous: i32,
    older: i32,
    envelope: i32,
    /// ADSR attack/decay/sustain progress; release is tracked separately.
    #[serde(default)]
    envelope_phase: EnvelopePhase,
}

/// ADSR phases. Direct and sliding GAIN can override the envelope at any time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum EnvelopePhase {
    #[default]
    Attack,
    Decay,
    Sustain,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            releasing: false,
            address: 0,
            header: 0,
            samples: [0; BRR_SAMPLES_PER_BLOCK],
            past_samples: [0; 3],
            index: 0,
            fraction: 0,
            previous: 0,
            older: 0,
            envelope: 0,
            envelope_phase: EnvelopePhase::Attack,
        }
    }
}

impl Dsp {
    /// Stop the voices and restore power-on register values.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Read one DSP register, including live ENDX/ENVX values.
    #[must_use]
    pub fn read_register(&self, address: u8) -> u8 {
        self.registers[usize::from(address & 0x7f)]
    }

    /// Write a DSP register and process key-on / key-off commands.
    ///
    /// Hardware latches KON/KOFF on internal sample phases. This simplified
    /// implementation applies them immediately, before the next sample pair.
    pub fn write_register(&mut self, address: u8, value: u8, ram: &[u8]) {
        let reg = usize::from(address & 0x7f);
        match reg {
            0x4c => {
                self.registers[reg] = value;
                for index in 0..VOICE_COUNT {
                    if value & (1 << index) != 0 {
                        self.key_on(index, ram);
                    }
                }
            }
            0x5c => {
                self.registers[reg] = value;
                for index in 0..VOICE_COUNT {
                    if value & (1 << index) != 0 {
                        self.voices[index].releasing = true;
                    }
                }
            }
            0x7c => self.registers[0x7c] = 0,
            0x6c if value & 0x80 != 0 => {
                self.registers[reg] = value;
                self.voices.fill(Voice::default());
            }
            _ => self.registers[reg] = value,
        }
    }

    /// Render one interleaved stereo sample pair at the DSP's 32 kHz cadence.
    ///
    /// The host calls this only when the APU's fractional master clock
    /// indicates that a sample is due.
    #[must_use]
    pub fn next_stereo_pair(&mut self, ram: &mut [u8]) -> (i16, i16) {
        // The S-DSP clock decrements its common envelope counter once per
        // output sample. All eight voices share this phase.
        self.envelope_counter = if self.envelope_counter == 0 {
            ENVELOPE_COUNTER_WRAP - 1
        } else {
            self.envelope_counter - 1
        };
        // SNES NON uses one common 15-bit Fibonacci LFSR for all voices.
        // FLG bits 0..4 select its clock rate from the envelope table.
        if envelope_rate_ready(self.registers[0x6c] & 0x1f, self.envelope_counter) {
            let feedback = (self.noise_lfsr ^ (self.noise_lfsr >> 1)) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | (feedback << 14);
        }
        let noise_sample = (self.noise_lfsr << 1) as i16;
        let mut left = 0_i64;
        let mut right = 0_i64;
        let mut echo_left = 0_i64;
        let mut echo_right = 0_i64;
        // PMON modulates voice N's pitch with voice N-1's envelope-scaled
        // output, not with its raw BRR sample. Voice zero ignores PMON.
        let mut previous_voice_output = 0_i16;
        for index in 0..VOICE_COUNT {
            let base = index * 16;
            let pitch = (u16::from(self.registers[base + 3] & 0x3f) << 8)
                | u16::from(self.registers[base + 2]);
            let pitch = if index > 0 && self.registers[0x2d] & (1 << index) != 0 {
                let modulation = ((i32::from(previous_voice_output) >> 5) * i32::from(pitch)) >> 10;
                (i32::from(pitch) + modulation).clamp(0, 0x7fff) as u16
            } else {
                pitch
            };
            let adsr1 = self.registers[base + 5];
            let adsr2 = self.registers[base + 6];
            let gain = self.registers[base + 7];
            let noise = (self.registers[0x3d] & (1 << index) != 0).then_some(noise_sample);
            let (sample, ended_block) = self.voices[index].clock(
                index,
                pitch,
                adsr1,
                adsr2,
                gain,
                self.envelope_counter,
                noise,
                ram,
                &self.registers,
            );
            previous_voice_output = sample;
            // The DSP applies each signed voice volume before adding it to
            // the main and echo accumulators. Both accumulators saturate to
            // 16 bits after every voice, not just after all eight voices.
            // Keeping excess precision here can turn loud, cancelling
            // voices into clipping that does not occur on the hardware.
            let voice_left =
                (i64::from(sample) * i64::from(self.registers[base] as i8)) >> 7;
            let voice_right =
                (i64::from(sample) * i64::from(self.registers[base + 1] as i8)) >> 7;
            left = i64::from(clamp_i16(left + voice_left));
            right = i64::from(clamp_i16(right + voice_right));
            if self.registers[0x4d] & (1 << index) != 0 {
                echo_left = i64::from(clamp_i16(echo_left + voice_left));
                echo_right = i64::from(clamp_i16(echo_right + voice_right));
            }
            self.registers[base + 8] = (self.voices[index].envelope >> 4) as u8;
            self.registers[base + 9] = (i32::from(sample) >> 8) as u8;
            if ended_block {
                self.registers[0x7c] |= 1 << index;
            }
        }
        // DSP volume multipliers are signed fixed-point shifts, including
        // their negative rounding. Truncating Rust integer division loses
        // low-amplitude negative samples and changes the echo-send signal.
        let left = (left * i64::from(self.registers[0x0c] as i8)) >> 7;
        let right = (right * i64::from(self.registers[0x1c] as i8)) >> 7;
        let (left, right) = self.process_echo(ram, [left, right], [echo_left, echo_right]);
        if self.registers[0x6c] & 0x40 != 0 {
            (0, 0)
        } else {
            (left, right)
        }
    }

    /// Echo reads and writes SPC RAM at ESA with EDL-selected ring length.
    /// FIR runs on eight historical 16-bit stereo samples; echo effects are
    /// not yet precisely synchronized to the S-DSP 32-cycle pipeline.
    fn process_echo(&mut self, ram: &mut [u8], main: [i64; 2], send: [i64; 2]) -> (i16, i16) {
        let address = (u16::from(self.registers[0x6d]) << 8).wrapping_add(self.echo_offset);
        let recent = [
            i16::from_le_bytes([
                read_ram(ram, address),
                read_ram(ram, address.wrapping_add(1)),
            ]),
            i16::from_le_bytes([
                read_ram(ram, address.wrapping_add(2)),
                read_ram(ram, address.wrapping_add(3)),
            ]),
        ];
        let pos = self.echo_history_index;
        self.echo_history[pos] = [recent[0] >> 1, recent[1] >> 1];

        // FIR0 acts on the oldest history sample; FIR7 acts on the
        // newest. Hardware accumulates the first seven products with
        // signed 16-bit wrap, then clamps only the final FIR7 addition.
        // Clamping all eight at the end changes high-gain echo behavior.
        let mut sum = [0_i64; 2];
        for tap in 0..7 {
            let history = self.echo_history[(pos + 1 + tap) % 8];
            let coefficient = i64::from(self.registers[0x0f + tap * 16] as i8);
            for channel in 0..2 {
                sum[channel] += (i64::from(history[channel]) * coefficient) >> 6;
            }
        }
        let final_coefficient = i64::from(self.registers[0x7f] as i8);
        let filtered = [0, 1].map(|channel| {
            let wrapped = i64::from(sum[channel] as i16);
            let newest = i64::from(self.echo_history[pos][channel]);
            i64::from(clamp_i16(wrapped + ((newest * final_coefficient) >> 6))) & !1
        });
        let feedback = i64::from(self.registers[0x0d] as i8);
        let mut output = [0_i16; 2];
        for channel in 0..2 {
            let echo_volume =
                i64::from(self.registers[if channel == 0 { 0x2c } else { 0x3c }] as i8);
            output[channel] = clamp_i16(main[channel] + ((filtered[channel] * echo_volume) >> 7));
            if self.registers[0x6c] & 0x20 == 0 {
                let sample = clamp_i16(send[channel] + ((filtered[channel] * feedback) >> 7)) & !1;
                let [low, high] = sample.to_le_bytes();
                let sample_address = address.wrapping_add((channel * 2) as u16);
                write_ram(ram, sample_address, low);
                write_ram(ram, sample_address.wrapping_add(1), high);
            }
        }

        self.echo_history_index = (pos + 1) % 8;
        let delay_bytes = u16::from(self.registers[0x7d] & 0x0f) * 2048;
        let delay_bytes = delay_bytes.max(4);
        self.echo_offset = (self.echo_offset + 4) % delay_bytes;
        (output[0], output[1])
    }

    fn key_on(&mut self, index: usize, ram: &[u8]) {
        let base = index * 16;
        let dir = u16::from(self.registers[0x5d]) << 8;
        let source = u16::from(self.registers[base + 4]);
        let entry = dir.wrapping_add(source * 4);
        let start = read_word(ram, entry);
        let voice = &mut self.voices[index];
        *voice = Voice {
            active: true,
            address: start,
            envelope: 0,
            ..Voice::default()
        };
        voice.decode_block(ram);
        self.registers[0x7c] &= !(1 << index);
    }
}

impl Voice {
    fn clock(
        &mut self,
        voice_index: usize,
        pitch: u16,
        adsr1: u8,
        adsr2: u8,
        gain: u8,
        counter: u16,
        noise: Option<i16>,
        ram: &[u8],
        registers: &[u8],
    ) -> (i16, bool) {
        if !self.active {
            return (0, false);
        }

        self.advance_envelope(adsr1, adsr2, gain, counter);

        // Gaussian interpolation spans three earlier decoded samples,
        // including values in the previous BRR block. NON overrides the
        // interpolated waveform, not its envelope.
        let waveform = noise.unwrap_or_else(|| self.interpolated_waveform());
        // The S-DSP uses a signed arithmetic shift, not truncating integer
        // division. Its output sample also clears the low bit before PMON,
        // OUTX and channel-volume processing.
        let result = (((i32::from(waveform) * self.envelope) >> 11) as i16) & !1;
        let mut ended_block = false;
        self.fraction += u32::from(pitch);
        while self.fraction >= PITCH_UNIT && self.active {
            self.fraction -= PITCH_UNIT;
            self.index += 1;
            if self.index == BRR_SAMPLES_PER_BLOCK {
                self.index = 0;
                self.past_samples.copy_from_slice(&self.samples[13..16]);
                if self.header & 1 != 0 {
                    // ENDX records the BRR end event even for looping voices.
                    ended_block = true;
                    if self.header & 2 == 0 {
                        self.active = false;
                        break;
                    }
                    let dir = u16::from(registers[0x5d]) << 8;
                    let source = u16::from(registers[voice_index * 16 + 4]);
                    let entry = dir.wrapping_add(source * 4);
                    self.address = read_word(ram, entry.wrapping_add(2));
                } else {
                    self.address = self.address.wrapping_add(9);
                }
                self.decode_block(ram);
            }
        }
        (result, ended_block)
    }

    /// Four consecutive BRR samples, with a three-sample carry at a block
    /// boundary. Fetch only decoded history: no speculative BRR block reads.
    fn interpolated_waveform(&self) -> i16 {
        let mut taps = [0_i16; 4];
        for (tap, sample) in taps.iter_mut().enumerate() {
            let position = self.index as isize + tap as isize - 3;
            *sample = if position < 0 {
                self.past_samples[(position + 3) as usize]
            } else {
                self.samples[position as usize]
            };
        }
        gaussian::interpolate(taps, self.fraction)
    }

    /// Rate-controlled ADSR and GAIN; hardware pipeline timing is still
    /// separate outstanding fidelity work.
    fn advance_envelope(&mut self, adsr1: u8, adsr2: u8, gain: u8, counter: u16) {
        if self.releasing {
            // SPC release is fixed at -8 per output sample, not rate-table
            // controlled. Keep key-off decay independent of envelope mode.
            self.envelope = (self.envelope - 8).max(0);
            if self.envelope == 0 {
                self.active = false;
            }
            return;
        }
        if adsr1 & 0x80 == 0 {
            // Fixed GAIN replaces the envelope immediately. A mode switch
            // doesn't require key-on or restarting a BRR sample.
            if gain & 0x80 == 0 {
                self.envelope = i32::from(gain) << 4;
                return;
            }
            let rate = gain & 0x1f;
            if !envelope_rate_ready(rate, counter) {
                return;
            }
            self.envelope = match (gain >> 5) & 3 {
                0 => self.envelope - 0x20,                     // Linear decrease.
                1 => self.envelope - 1 - (self.envelope >> 8), // Exponential decrease.
                2 => self.envelope + 0x20,                     // Linear increase.
                _ => self.envelope + if self.envelope < 0x600 { 0x20 } else { 8 },
            }
            .clamp(0, 0x7ff);
            return;
        }

        let rate = match self.envelope_phase {
            EnvelopePhase::Attack => (adsr1 & 0x0f) * 2 + 1,
            EnvelopePhase::Decay => ((adsr1 >> 3) & 0x0e) + 0x10,
            EnvelopePhase::Sustain => adsr2 & 0x1f,
        };
        if !envelope_rate_ready(rate, counter) {
            return;
        }
        match self.envelope_phase {
            EnvelopePhase::Attack => {
                self.envelope += if rate == 31 { 0x400 } else { 0x20 };
                if self.envelope >= 0x7ff {
                    self.envelope = 0x7ff;
                    self.envelope_phase = EnvelopePhase::Decay;
                }
            }
            EnvelopePhase::Decay | EnvelopePhase::Sustain => {
                self.envelope = (self.envelope - 1 - (self.envelope >> 8)).max(0);
                if self.envelope_phase == EnvelopePhase::Decay
                    && (self.envelope >> 8) <= i32::from(adsr2 >> 5)
                {
                    self.envelope_phase = EnvelopePhase::Sustain;
                }
            }
        }
    }

    fn decode_block(&mut self, ram: &[u8]) {
        self.header = read_ram(ram, self.address);
        let shift = u32::from(self.header >> 4);
        let filter = (self.header >> 2) & 3;
        for i in 0..BRR_SAMPLES_PER_BLOCK {
            let packed = read_ram(ram, self.address.wrapping_add(1 + (i / 2) as u16));
            let nibble = if i % 2 == 0 { packed >> 4 } else { packed & 15 };
            let signed = i32::from((nibble as i8) << 4) >> 4;
            let decoded = if shift <= 12 {
                (signed << shift) >> 1
            } else if signed < 0 {
                -2048
            } else {
                0
            };
            // Preserve the DSP's integer operation order. Combining these
            // filter terms into rational coefficients changes rounding for
            // small positive/negative samples, even when algebraically equal.
            let prior = self.previous * 2;
            let two_ago = self.older;
            let predicted = match filter {
                0 => decoded,
                1 => decoded + (prior >> 1) + ((-prior) >> 5),
                2 => decoded + prior - two_ago + (two_ago >> 4) + ((-3 * prior) >> 6),
                _ => decoded + prior - two_ago + ((-13 * prior) >> 7) + ((3 * two_ago) >> 4),
            };
            // The BRR filter saturates to signed 16 bits *before* the
            // hardware's extra left shift. That shift then wraps as i16;
            // clamping to 15 bits instead changes loud/filtered waveforms.
            let clipped = predicted.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
            let sample = (clipped * 2) as i16;
            self.samples[i] = sample;
            self.older = self.previous;
            // Subsequent filters consume the wrapped decoded sample, not
            // the unwrapped or initially clipped intermediate value.
            self.previous = i32::from(sample) >> 1;
        }
    }
}

fn read_ram(ram: &[u8], address: u16) -> u8 {
    ram.get(usize::from(address)).copied().unwrap_or(0)
}

fn write_ram(ram: &mut [u8], address: u16, value: u8) {
    if let Some(slot) = ram.get_mut(usize::from(address)) {
        *slot = value;
    }
}

fn read_word(ram: &[u8], address: u16) -> u16 {
    u16::from(read_ram(ram, address)) | (u16::from(read_ram(ram, address.wrapping_add(1))) << 8)
}

fn clamp_i16(value: i64) -> i16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::Dsp;

    fn looping_sample() -> (Dsp, Vec<u8>) {
        let mut ram = vec![0; 0x10000];
        // Directory entry zero at $0200: both pointers target $0300.
        ram[0x200] = 0x00;
        ram[0x201] = 0x03;
        ram[0x202] = 0x00;
        ram[0x203] = 0x03;
        ram[0x300] = 0xc3; // BRR range 12, filter 0, loop + end.
        ram[0x301..0x309].fill(0x77);
        let mut dsp = Dsp::default();
        dsp.write_register(0x5d, 0x02, &ram);
        dsp.write_register(0x00, 0x7f, &ram);
        dsp.write_register(0x01, 0x7f, &ram);
        dsp.write_register(0x02, 0x00, &ram);
        dsp.write_register(0x03, 0x10, &ram);
        dsp.write_register(0x07, 0x7f, &ram);
        dsp.write_register(0x0c, 0x7f, &ram);
        dsp.write_register(0x1c, 0x7f, &ram);
        (dsp, ram)
    }

    #[test]
    fn negative_low_level_main_mix_uses_signed_arithmetic_shift() {
        let (mut dsp, mut ram) = looping_sample();
        // NON starts at -32768. Direct GAIN 1 produces a -256 voice output;
        // VOL(L)=1 yields -2 after the voice >>7, and MVOL(L)=1 yields -1.
        // Integer division toward zero would incorrectly turn -2 / 128 into 0.
        dsp.write_register(0x00, 1, &ram);
        dsp.write_register(0x01, 0, &ram);
        dsp.write_register(0x0c, 1, &ram);
        dsp.write_register(0x1c, 0, &ram);
        dsp.write_register(0x07, 1, &ram);
        dsp.write_register(0x3d, 1, &ram);
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.next_stereo_pair(&mut ram), (-1, 0));
    }

    #[test]
    fn multiple_voices_saturate_in_order_before_master_volume() {
        let (mut dsp, mut ram) = looping_sample();
        // NON has a fixed negative sample while the initial LFSR rate is
        // zero. Three negative voice volumes saturate the positive mix;
        // a fourth positive volume then subtracts from that saturated
        // value. Summing unbounded voice products first would clip to
        // full scale instead of yielding a small positive output.
        for index in 0..4 {
            let base = (index * 16) as u8;
            dsp.write_register(base, if index < 3 { 0x80 } else { 0x7f }, &ram);
            dsp.write_register(base + 1, 0, &ram);
            dsp.write_register(base + 7, 0x7f, &ram);
        }
        dsp.write_register(0x3d, 0x0f, &ram); // NON on four voices.
        dsp.write_register(0x4c, 0x0f, &ram);
        assert_eq!(dsp.next_stereo_pair(&mut ram), (505, 0));
    }

    #[test]
    fn negative_echo_volume_and_feedback_round_toward_negative_infinity() {
        let (mut dsp, mut ram) = looping_sample();
        // A preloaded tiny negative echo input, routed through the newest FIR tap,
        // reaches negative fixed-point scaling before the final output clamp.
        ram[0x6000..0x6004].copy_from_slice(&[0xfe, 0xff, 0xfe, 0xff]);
        dsp.write_register(0x6d, 0x60, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x7f, 0x7f, &ram);
        dsp.write_register(0x2c, 1, &ram);
        dsp.write_register(0x3c, 1, &ram);
        dsp.write_register(0x0d, 1, &ram);
        dsp.write_register(0x6c, 0, &ram);
        assert_eq!(dsp.next_stereo_pair(&mut ram), (-1, -1));
        assert_eq!(&ram[0x6000..0x6004], &[0xfe, 0xff, 0xfe, 0xff]);
    }

    #[test]
    fn fir_zero_is_oldest_and_seven_is_newest() {
        let (mut dsp, mut ram) = looping_sample();
        // Preload only the first echo frame. Later echo samples are zero,
        // and FLG defaults to write-inhibited, so they stay that way.
        ram[0x5000..0x5004].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        dsp.write_register(0x6d, 0x50, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);

        dsp.write_register(0x7f, 0x7f, &ram);
        let immediate = dsp.next_stereo_pair(&mut ram);
        assert!(immediate.0 > 1_000);
        assert_eq!(immediate.0, immediate.1);

        // A fresh DSP with FIR0 only must delay this input by seven
        // samples. FIR7 would have produced output immediately.
        let (mut delayed_dsp, mut delayed_ram) = looping_sample();
        delayed_ram[0x5000..0x5004].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        for (reg, value) in [(0x6d, 0x50), (0x7d, 1), (0x2c, 0x7f), (0x3c, 0x7f), (0x0f, 0x7f)] {
            delayed_dsp.write_register(reg, value, &delayed_ram);
        }
        for _ in 0..7 {
            assert_eq!(delayed_dsp.next_stereo_pair(&mut delayed_ram), (0, 0));
        }
        let delayed = delayed_dsp.next_stereo_pair(&mut delayed_ram);
        assert!(delayed.0 > 1_000);
        assert_eq!(delayed.0, delayed.1);
    }

    #[test]
    fn fir_first_seven_sum_wraps_before_saturating_last_tap() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x5000..0x5004].copy_from_slice(&[0xfe, 0x7f, 0xfe, 0x7f]);
        dsp.echo_history.fill([16_383, 16_383]);
        dsp.write_register(0x6d, 0x50, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        for tap in 0..7 {
            dsp.write_register(0x0f + tap * 16, 0x7f, &ram);
        }
        dsp.write_register(0x7f, 0x80, &ram); // Final signed coefficient = -128.
        let term = (16_383_i32 * 127) >> 6;
        let wrapped = i32::from((term * 7) as i16);
        let final_term = (16_383_i32 * -128) >> 6;
        let filtered = (wrapped + final_term).clamp(-32_768, 32_767) & !1;
        let expected = ((filtered * 127) >> 7) as i16;
        let output = dsp.next_stereo_pair(&mut ram);
        assert_eq!(output, (expected, expected));
        assert!(expected < 0, "wrap must change the sign of high-gain FIR");
    }

    #[test]
    fn global_mute_silences_dac_but_not_voice_outx_or_echo_ram_writes() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4d, 1, &ram); // Route voice zero into echo.
        dsp.write_register(0x7f, 0x7f, &ram); // Newest FIR tap.
        dsp.write_register(0x6d, 0x50, &ram); // Safe RAM echo region.
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x6c, 0x40, &ram); // Mute output; echo writes on.
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..32 {
            assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
        }
        assert_eq!(dsp.read_register(0x08), 0x7f);
        assert_ne!(dsp.read_register(0x09), 0);
        assert!(ram[0x5010..0x5080].iter().any(|&byte| byte != 0));

        // Muting is an output-gain gate, not a voice restart.
        let sample_index = dsp.voices[0].index;
        dsp.write_register(0x6c, 0, &ram);
        let (left, right) = dsp.next_stereo_pair(&mut ram);
        assert!(left > 1000);
        assert_eq!(left, right);
        assert_ne!(dsp.voices[0].index, sample_index);
    }

    #[test]
    fn spc_ram_echo_delay_and_fir_replay_after_512_stereo_samples() {
        let (mut dsp, mut ram) = looping_sample();
        // Keep the dry output silent. Route voice zero only to the echo
        // write buffer at $5000, with a one-unit (512-frame) delay.
        dsp.write_register(0x0c, 0, &ram);
        dsp.write_register(0x1c, 0, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        dsp.write_register(0x4d, 1, &ram);
        dsp.write_register(0x7f, 0x7f, &ram); // FIR newest-sample tap (FIR7).
        dsp.write_register(0x6d, 0x50, &ram); // ESA $5000.
        dsp.write_register(0x7d, 1, &ram); // 2048 echo bytes = 512 pairs.
        dsp.write_register(0x6c, 0, &ram); // Permit echo RAM writes.
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..512 {
            assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
        }
        // Gaussian startup has an initial silent decode window. The echo
        // ring contains those initial silent frames before audible BRR PCM.
        assert_ne!(&ram[0x5010..0x5014], &[0; 4]);
        for _ in 0..4 {
            dsp.next_stereo_pair(&mut ram);
        }
        let echoed = dsp.next_stereo_pair(&mut ram);
        assert!(echoed.0 > 1000, "echo must replay earlier voice output");
        assert_eq!(echoed.0, echoed.1);
        assert_eq!(dsp.echo_offset, 20);
    }

    #[test]
    fn echo_write_inhibit_still_allows_read_and_fir_output() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x5000..0x5004].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        dsp.write_register(0x6d, 0x50, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x7f, 0x7f, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        // Default FLG bit 5 inhibits echo writes, but not reads.
        let echoed = dsp.next_stereo_pair(&mut ram);
        assert!(echoed.0 > 1000);
        assert_eq!(echoed.0, echoed.1);
        assert_eq!(&ram[0x5000..0x5004], &[0x10, 0x27, 0x10, 0x27]);
    }

    #[test]
    fn echo_signed_feedback_and_negative_fir_are_visible_in_spc_ram() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x6000..0x6004].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        dsp.write_register(0x6d, 0x60, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x7f, 0xc0, &ram); // Negative FIR coefficient.
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        dsp.write_register(0x0d, 0x40, &ram); // Positive echo feedback.
        dsp.write_register(0x6c, 0, &ram);
        let output = dsp.next_stereo_pair(&mut ram);
        assert!(output.0 < -1000);
        assert_eq!(output.0, output.1);
        let stored = i16::from_le_bytes([ram[0x6000], ram[0x6001]]);
        assert!(stored < -1000, "feedback should write signed filtered echo");
        assert_eq!(stored, i16::from_le_bytes([ram[0x6002], ram[0x6003]]));
    }

    #[test]
    fn echo_esa_address_wraps_through_end_of_spc_ram() {
        let (mut dsp, mut ram) = looping_sample();
        // $FF00 + $0100 must wrap to RAM $0000, not address $10000.
        ram[0..4].copy_from_slice(&[0x10, 0x27, 0x10, 0x27]);
        dsp.echo_offset = 0x0100;
        dsp.write_register(0x6d, 0xff, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x7f, 0x7f, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        let sample = dsp.next_stereo_pair(&mut ram);
        assert!(sample.0 > 1000);
        assert_eq!(sample.0, sample.1);
        assert_eq!(&ram[0..4], &[0x10, 0x27, 0x10, 0x27]);
        assert_eq!(dsp.echo_offset, 0x0104);
    }

    #[test]
    fn echo_fir_history_and_spc_ram_resume_exactly_after_save() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4d, 1, &ram);
        dsp.write_register(0x7f, 0x7f, &ram);
        dsp.write_register(0x6f, 0x20, &ram); // FIR previous-sample tap (FIR6).
        dsp.write_register(0x6d, 0x50, &ram);
        dsp.write_register(0x7d, 1, &ram);
        dsp.write_register(0x2c, 0x7f, &ram);
        dsp.write_register(0x3c, 0x7f, &ram);
        dsp.write_register(0x6c, 0, &ram);
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..519 {
            dsp.next_stereo_pair(&mut ram);
        }
        let snapshot = serde_json::to_string(&dsp).unwrap();
        let mut restored: Dsp = serde_json::from_str(&snapshot).unwrap();
        let mut resumed_ram = ram.clone();
        for _ in 0..256 {
            assert_eq!(
                dsp.next_stereo_pair(&mut ram),
                restored.next_stereo_pair(&mut resumed_ram)
            );
        }
        assert_eq!(ram, resumed_ram);
        assert_eq!(
            serde_json::to_value(&dsp).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
    }

    #[test]
    fn brr_filters_preserve_signed_integer_rounding_for_small_history() {
        for filter in [2_u8, 3] {
            let mut ram = vec![0_u8; 65_536];
            ram[0x300] = filter << 2;
            // For prior=6 and two_ago=1, each shift is rounded before
            // summation: sample +8, versus the simplified coefficient's +10.
            // Mirrored negative history similarly yields -12 versus -10.
            for (previous, older, expected) in [(3, 1, 8), (-3, -1, -12)] {
                let mut voice = super::Voice {
                    address: 0x300,
                    previous,
                    older,
                    ..super::Voice::default()
                };
                voice.decode_block(&ram);
                assert_eq!(voice.samples[0], expected, "BRR filter {filter}");
            }
        }
    }

    #[test]
    fn brr_filtered_samples_wrap_after_signed_saturation() {
        let mut ram = vec![0_u8; 65_536];
        ram[0x300] = 0x08; // Zero BRR nibbles, filter 2.
        let mut voice = super::Voice {
            address: 0x300,
            previous: 15_000,
            ..super::Voice::default()
        };
        voice.decode_block(&ram);
        let predicted = (15_000 * 61) >> 5;
        let wrapped = (predicted * 2) as i16;
        assert!(wrapped < 0);
        assert_eq!(voice.samples[0], wrapped);
        // Filter history must contain the signed, *wrapped* sample. Otherwise
        // the next BRR sample has the wrong predictor sign.
        assert!(voice.samples[1] > 0);
        assert_eq!(voice.older, i32::from(voice.samples[14]) >> 1);
        assert_eq!(voice.previous, i32::from(voice.samples[15]) >> 1);
    }

    #[test]
    fn gaussian_window_carries_previous_brr_block_without_hard_discontinuity() {
        let mut ram = vec![0_u8; 65_536];
        ram[0x200..0x204].copy_from_slice(&[0, 3, 0, 3]);
        ram[0x300] = 0xc3; // Looping positive block.
        ram[0x301..0x309].fill(0x77);
        let (mut dsp, mut original_ram) = looping_sample();
        original_ram.copy_from_slice(&ram);
        dsp.write_register(0x4c, 1, &original_ram);
        for _ in 0..16 {
            dsp.next_stereo_pair(&mut original_ram);
        }
        assert_eq!(dsp.voices[0].past_samples, dsp.voices[0].samples[13..16]);
        assert_eq!(dsp.voices[0].index, 0);
        let first_loop = dsp.next_stereo_pair(&mut original_ram).0;
        assert!(
            first_loop > 1000,
            "previous block must feed interpolation on a loop"
        );
        let snapshot = serde_json::to_string(&dsp).unwrap();
        let mut restored: Dsp = serde_json::from_str(&snapshot).unwrap();
        for _ in 0..48 {
            assert_eq!(
                dsp.next_stereo_pair(&mut original_ram),
                restored.next_stereo_pair(&mut original_ram)
            );
        }
    }

    #[test]
    fn gaussian_blends_opposite_polarity_samples_across_brr_block_boundary() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x300] = 0xc0; // First BRR block: positive, not an end marker.
        ram[0x301..0x309].fill(0x44);
        ram[0x309] = 0xc3; // Second BRR block: negative, end/loop.
        ram[0x30a..0x312].fill(0xcc);
        ram[0x202] = 0x09; // Loop to the second block on subsequent ends.
        ram[0x203] = 0x03;
        dsp.write_register(0x4c, 1, &ram);

        for _ in 0..16 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.voices[0].address, 0x0309);
        let first = dsp.next_stereo_pair(&mut ram).0;
        let second = dsp.next_stereo_pair(&mut ram).0;
        let third = dsp.next_stereo_pair(&mut ram).0;
        // A step-function sampler would go negative immediately. The four-tap
        // Gaussian preserves the preceding positive block for two samples.
        assert!(first > 1000);
        assert!(second > 1000);
        assert!(third < -1000);
    }

    #[test]
    fn voice_signed_envelope_uses_arithmetic_shift_and_even_output_bits() {
        let mut voice = super::Voice {
            active: true,
            ..super::Voice::default()
        };
        let registers = [0_u8; 128];
        // Direct GAIN 1 sets envelope=16; (-2 * 16) >> 11 is -1,
        // and the DSP's low-bit mask makes the output -2, not zero.
        let (negative, _) = voice.clock(0, 0, 0, 0, 1, 0, Some(-2), &[], &registers);
        assert_eq!(negative, -2);
        let (positive, _) = voice.clock(0, 0, 0, 0, 1, 0, Some(2), &[], &registers);
        assert_eq!(positive, 0);
        let (loud, _) = voice.clock(0, 0, 0, 0, 0x7f, 0, Some(-16_384), &[], &registers);
        assert!(loud < -1000);
        assert_eq!(loud & 1, 0);
    }

    #[test]
    fn gaussian_history_resets_on_rekey_and_old_saves_default_to_silence() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..32 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert!(dsp.voices[0].past_samples.iter().any(|&value| value != 0));
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.voices[0].past_samples, [0; 3]);
        assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));

        // Save states predating the new carryover field must deserialize
        // safely without making up prior BRR waveform history.
        let mut legacy = serde_json::to_value(&dsp).unwrap();
        for voice in legacy["voices"].as_array_mut().unwrap() {
            voice.as_object_mut().unwrap().remove("past_samples");
        }
        let restored: Dsp = serde_json::from_value(legacy).unwrap();
        assert!(restored.voices.iter().all(|v| v.past_samples == [0; 3]));
    }

    #[test]
    fn gaussian_phase_changes_output_on_nonconstant_brr_waveform() {
        let (mut full, mut ram) = looping_sample();
        // Alternating high/low decoded nibble input: interpolation varies
        // continuously as phase changes, unlike the previous step sampler.
        ram[0x301..0x309].fill(0x70);
        full.write_register(0x4c, 1, &ram);
        let mut half = full.clone();
        half.write_register(0x02, 0, &ram);
        half.write_register(0x03, 8, &ram);
        for _ in 0..7 {
            full.next_stereo_pair(&mut ram);
            half.next_stereo_pair(&mut ram);
        }
        assert_ne!(
            full.voices[0].fraction, half.voices[0].fraction,
            "test must select different fractional phases"
        );
        assert_ne!(
            full.next_stereo_pair(&mut ram),
            half.next_stereo_pair(&mut ram)
        );
    }

    #[test]
    fn looping_brr_voice_generates_sample_data_and_stereo_volume() {
        let (mut dsp, mut ram) = looping_sample();
        assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
        dsp.write_register(0x4c, 1, &ram);
        for tick in 0..64 {
            let (left, right) = dsp.next_stereo_pair(&mut ram);
            // Four-tap Gaussian interpolation takes three decoded samples
            // to fill its startup window.
            if tick >= 3 {
                assert!(left > 1000);
            }
            assert_eq!(left, right);
        }
        assert_eq!(dsp.read_register(0x7c), 1);
    }

    #[test]
    fn separate_voices_use_their_own_brr_source_and_loop_directory() {
        let (mut dsp, mut ram) = looping_sample();
        // Source one points to a separate, negative BRR waveform.
        ram[0x204] = 0x00;
        ram[0x205] = 0x04;
        ram[0x206] = 0x00;
        ram[0x207] = 0x04;
        ram[0x400] = 0xc3;
        ram[0x401..0x409].fill(0xaa); // Avoid full-scale Gaussian wrap.
        // Silence voice zero and drive only voice one to the left channel.
        dsp.write_register(0x00, 0, &ram);
        dsp.write_register(0x10, 0x7f, &ram);
        dsp.write_register(0x11, 0, &ram);
        dsp.write_register(0x13, 0x10, &ram);
        dsp.write_register(0x14, 1, &ram);
        dsp.write_register(0x17, 0x7f, &ram);
        dsp.write_register(0x4c, 0b10, &ram);
        for tick in 0..48 {
            let (left, right) = dsp.next_stereo_pair(&mut ram);
            if tick >= 6 {
                assert!(left < -1000, "voice one must retain its own loop source");
            }
            assert_eq!(right, 0);
        }
        assert_eq!(dsp.read_register(0x7c) & 0b10, 0b10);
    }

    #[test]
    fn looping_voice_endx_is_latched_cleared_and_reasserted_at_next_loop() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..15 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.read_register(0x7c) & 1, 0);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.read_register(0x7c) & 1, 1);
        dsp.write_register(0x7c, 0xff, &ram);
        assert_eq!(dsp.read_register(0x7c) & 1, 0);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.read_register(0x7c) & 1, 0);
        for _ in 0..15 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.read_register(0x7c) & 1, 1);
    }

    #[test]
    fn voice_end_sets_endx_and_muting_stops_output() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x300] = 0xc1; // End without loop.
        dsp.write_register(0x4c, 1, &ram);
        for tick in 0..16 {
            let output = dsp.next_stereo_pair(&mut ram).0;
            if tick >= 3 {
                assert!(output > 0);
            }
        }
        assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
        assert_eq!(dsp.read_register(0x7c) & 1, 1);
        dsp.write_register(0x7c, 0xff, &ram);
        assert_eq!(dsp.read_register(0x7c), 0);
        for _ in 0..24 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(
            dsp.read_register(0x7c),
            0,
            "stopped voices must not recreate ENDX after a software clear"
        );
    }

    #[test]
    fn key_off_releases_and_global_mute_silences() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x6c, 0x40, &ram);
        assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
        dsp.write_register(0x6c, 0x00, &ram);
        for _ in 0..3 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert!(dsp.next_stereo_pair(&mut ram).0 > 0);
        dsp.write_register(0x5c, 1, &ram);
        for _ in 0..300 {
            let _ = dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.next_stereo_pair(&mut ram), (0, 0));
    }

    #[test]
    fn envelope_rate_table_and_gain_modes_follow_shared_sample_clock() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.voices[0].envelope, 0);

        // Direct GAIN always applies immediately, independent of rate.
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, 0x7f0);
        dsp.write_register(0x07, 0x9f, &ram); // GAIN linear down, rate 31.
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, 0x7d0);
        dsp.write_register(0x07, 0xbf, &ram); // Exponential down, rate 31.
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, 0x7d0 - 1 - (0x7d0 >> 8));
        dsp.write_register(0x07, 0xdf, &ram); // Linear up, rate 31.
        let before = dsp.voices[0].envelope;
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, before + 0x20);
        dsp.write_register(0x07, 0xff, &ram); // Bent increase, rate 31.
        let before = dsp.voices[0].envelope;
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, before + 8);
        dsp.write_register(0x07, 0x80, &ram); // Rate zero: never updates.
        let before = dsp.voices[0].envelope;
        for _ in 0..64 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.voices[0].envelope, before);
    }

    #[test]
    fn adsr_attack_decay_sustain_are_rate_controlled_and_serializable() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x05, 0xff, &ram); // Attack 15, decay 7.
        dsp.write_register(0x06, 0x00, &ram); // Sustain level 0, rate disabled.
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Attack);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, 0x400);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, 0x7ff);
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Decay);
        for _ in 0..4096 {
            dsp.next_stereo_pair(&mut ram);
            if dsp.voices[0].envelope_phase == super::EnvelopePhase::Sustain {
                break;
            }
        }
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Sustain);
        let frozen = dsp.voices[0].envelope;
        for _ in 0..64 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.voices[0].envelope, frozen);

        let saved = serde_json::to_string(&dsp).unwrap();
        let mut resumed: Dsp = serde_json::from_str(&saved).unwrap();
        dsp.write_register(0x06, 0x1f, &ram); // Enable rapid sustain decay.
        resumed.write_register(0x06, 0x1f, &ram);
        for _ in 0..128 {
            assert_eq!(
                dsp.next_stereo_pair(&mut ram),
                resumed.next_stereo_pair(&mut ram)
            );
        }
        assert_eq!(dsp.voices[0].envelope, resumed.voices[0].envelope);
        assert!(dsp.voices[0].envelope < frozen);
    }

    #[test]
    fn slow_gain_rate_obeys_period_and_is_independent_of_audio_chunking() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.next_stereo_pair(&mut ram); // Initialize direct GAIN.
        dsp.write_register(0x07, 0x81, &ram); // Linear down at rate 1 (2048 ticks).
        let baseline = dsp.voices[0].envelope;
        for _ in 0..2046 {
            dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.voices[0].envelope, baseline);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.voices[0].envelope, baseline - 0x20);
    }

    #[test]
    fn pitch_modulation_uses_previous_enveloped_voice_output() {
        let (mut base, mut ram) = looping_sample();
        // The two voices use the same positive BRR source. Voice one's pitch
        // is half a sample per DSP tick without PMON.
        base.write_register(0x12, 0, &ram);
        base.write_register(0x13, 0x08, &ram);
        base.write_register(0x17, 0x7f, &ram);
        base.write_register(0x4c, 0b11, &ram);
        let mut modulated = base.clone();
        modulated.write_register(0x2d, 0b10, &ram);
        let mut ignored_voice_zero = base.clone();
        ignored_voice_zero.write_register(0x2d, 0b01, &ram);

        for _ in 0..64 {
            let reference = base.next_stereo_pair(&mut ram);
            modulated.next_stereo_pair(&mut ram);
            assert_eq!(ignored_voice_zero.next_stereo_pair(&mut ram), reference);
        }
        // The higher effective pitch advances voice one farther through BRR
        // samples, but voice zero is unaffected.
        assert_eq!(base.voices[0].index, modulated.voices[0].index);
        assert_eq!(base.voices[0].fraction, modulated.voices[0].fraction);
        assert_ne!(base.voices[1].fraction, modulated.voices[1].fraction);
        assert_eq!(
            base.voices[1].fraction,
            ignored_voice_zero.voices[1].fraction
        );
    }

    #[test]
    fn negative_previous_voice_slows_pmon_and_state_survives_save() {
        let (mut base, mut ram) = looping_sample();
        // One negative BRR voice modulates a distinct positive second voice.
        ram[0x300] = 0xc3;
        ram[0x301..0x309].fill(0xaa); // Signed waveform below wrap threshold.
        ram[0x204] = 0x00;
        ram[0x205] = 0x04;
        ram[0x206] = 0x00;
        ram[0x207] = 0x04;
        ram[0x400] = 0xc3;
        ram[0x401..0x409].fill(0x77);
        base.write_register(0x12, 0, &ram);
        base.write_register(0x13, 0x02, &ram); // Avoid fractional wrap during startup.
        base.write_register(0x14, 1, &ram);
        base.write_register(0x17, 0x7f, &ram);
        base.write_register(0x4c, 0b11, &ram);

        let mut modulated = base.clone();
        modulated.write_register(0x2d, 0b10, &ram);
        // PMON cannot modulate from the initial all-zero Gaussian window.
        // Once the first three decoded samples have reached the FIR taps,
        // negative voice zero lowers voice one's effective playback pitch.
        for _ in 0..6 {
            base.next_stereo_pair(&mut ram);
            modulated.next_stereo_pair(&mut ram);
        }
        assert!(modulated.voices[1].fraction < base.voices[1].fraction);

        let saved = serde_json::to_string(&modulated).unwrap();
        let mut restored: Dsp = serde_json::from_str(&saved).unwrap();
        for _ in 0..100 {
            assert_eq!(
                modulated.next_stereo_pair(&mut ram),
                restored.next_stereo_pair(&mut ram)
            );
        }
        assert_eq!(modulated.voices[1].index, restored.voices[1].index);
        assert_eq!(modulated.voices[1].fraction, restored.voices[1].fraction);
    }

    #[test]
    fn noise_lfsr_uses_shared_clock_and_serializes_without_reset() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x3d, 1, &ram); // NON voice 0.
        dsp.write_register(0x6c, 0x1f, &ram); // Highest noise clock rate.
        assert_eq!(dsp.noise_lfsr, 0x4000);
        let first = dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.noise_lfsr, 0x2000);
        assert!(first.0 > 0);
        assert_eq!(first.0, first.1);
        dsp.next_stereo_pair(&mut ram);
        assert_eq!(dsp.noise_lfsr, 0x1000);

        let snapshot = serde_json::to_string(&dsp).unwrap();
        let mut restored: Dsp = serde_json::from_str(&snapshot).unwrap();
        for _ in 0..128 {
            assert_eq!(
                dsp.next_stereo_pair(&mut ram),
                restored.next_stereo_pair(&mut ram)
            );
            assert_eq!(dsp.noise_lfsr, restored.noise_lfsr);
        }
        assert_ne!(dsp.noise_lfsr, 0x4000);
    }

    #[test]
    fn noise_fibonacci_lfsr_has_full_nonzero_fifteen_bit_period() {
        let mut dsp = Dsp::default();
        dsp.write_register(0x6c, 0x1f, &[]);
        let mut visited = [false; 0x8000];
        for _ in 0..32_767 {
            let state = usize::from(dsp.noise_lfsr);
            assert_ne!(state, 0);
            assert!(
                !visited[state],
                "LFSR repeated before completing full period"
            );
            visited[state] = true;
            dsp.next_stereo_pair(&mut []);
        }
        assert_eq!(dsp.noise_lfsr, 0x4000);
        assert!(!visited[0]);
    }

    #[test]
    fn noise_rate_zero_holds_lfsr_while_non_toggles_waveform() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x6c, 0, &ram); // Rate zero never clocks LFSR.
        dsp.write_register(0x3d, 1, &ram);
        let noise_output = dsp.next_stereo_pair(&mut ram);
        for _ in 0..4096 {
            let _ = dsp.next_stereo_pair(&mut ram);
        }
        assert_eq!(dsp.noise_lfsr, 0x4000);
        assert!(noise_output.0 < 0); // Signed 15-bit $4000.
        dsp.write_register(0x3d, 0, &ram);
        let sample_output = dsp.next_stereo_pair(&mut ram);
        assert!(sample_output.0 > 0); // Original positive BRR sample.
        assert_eq!(sample_output.0, sample_output.1);
    }

    #[test]
    fn two_non_voices_share_the_same_noise_waveform() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x10, 0x7f, &ram);
        dsp.write_register(0x11, 0, &ram);
        dsp.write_register(0x13, 0x10, &ram);
        dsp.write_register(0x17, 0x7f, &ram);
        dsp.write_register(0x3d, 0b11, &ram);
        dsp.write_register(0x6c, 0x1f, &ram);
        dsp.write_register(0x4c, 0b11, &ram);
        let both = dsp.next_stereo_pair(&mut ram);
        assert!(both.0 > 0);
        assert!(both.1 > 0);
        // Integer master-volume scaling rounds twice; allow two LSBs.
        assert!((i32::from(both.0) - 2 * i32::from(both.1)).abs() <= 2);
    }

    #[test]
    fn dsp_state_serializes_across_sample_boundaries() {
        let (mut dsp, mut ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..5 {
            let _ = dsp.next_stereo_pair(&mut ram);
        }
        let state = serde_json::to_string(&dsp).unwrap();
        let mut resumed: Dsp = serde_json::from_str(&state).unwrap();
        for _ in 0..50 {
            assert_eq!(
                dsp.next_stereo_pair(&mut ram),
                resumed.next_stereo_pair(&mut ram)
            );
        }
    }
}
