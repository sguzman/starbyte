//! Partial SNES S-DSP voice mixer and BRR synthesis engine.
//!
//! This DSP consumes SPC700 audio RAM, not the SNES main CPU bus. It handles
//! directory-based BRR samples, pitch stepping and PMON, signed channel/master
//! volume, timed ADSR/GAIN envelopes, NON shared noise, and key-on/off/loop
//! state. Gaussian interpolation, echo/FIR processing and exact multi-phase
//! hardware pipeline latency still need work. This is not bit-exact DSP audio.

use serde::{Deserialize, Serialize};

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
}

impl Default for Dsp {
    fn default() -> Self {
        Self {
            registers: vec![0; REGISTER_COUNT],
            voices: vec![Voice::default(); VOICE_COUNT],
            envelope_counter: 0,
            noise_lfsr: initial_noise_seed(),
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
    pub fn next_stereo_pair(&mut self, ram: &[u8]) -> (i16, i16) {
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
            let sample = self.voices[index].clock(
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
            left += i64::from(sample) * i64::from(self.registers[base] as i8);
            right += i64::from(sample) * i64::from(self.registers[base + 1] as i8);
            self.registers[base + 8] = (self.voices[index].envelope >> 4) as u8;
            self.registers[base + 9] = (i32::from(sample) >> 8) as u8;
            if !self.voices[index].active && self.voices[index].header & 1 != 0 {
                self.registers[0x7c] |= 1 << index;
            }
        }
        if self.registers[0x6c] & 0x40 != 0 {
            return (0, 0);
        }
        let left = (left / 128) * i64::from(self.registers[0x0c] as i8) / 128;
        let right = (right / 128) * i64::from(self.registers[0x1c] as i8) / 128;
        (clamp_i16(left), clamp_i16(right))
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
    ) -> i16 {
        if !self.active {
            return 0;
        }

        self.advance_envelope(adsr1, adsr2, gain, counter);

        let waveform = noise.unwrap_or(self.samples[self.index]);
        let result = (i32::from(waveform) * self.envelope / 2048) as i16;
        self.fraction += u32::from(pitch);
        while self.fraction >= PITCH_UNIT && self.active {
            self.fraction -= PITCH_UNIT;
            self.index += 1;
            if self.index == BRR_SAMPLES_PER_BLOCK {
                self.index = 0;
                if self.header & 1 != 0 {
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
        result
    }

    /// Rate-controlled ADSR and GAIN. Gaussian interpolation, echo and
    /// hardware pipeline timing are separate outstanding fidelity tasks.
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
    fn looping_brr_voice_generates_sample_data_and_stereo_volume() {
        let (mut dsp, ram) = looping_sample();
        assert_eq!(dsp.next_stereo_pair(&ram), (0, 0));
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..64 {
            let (left, right) = dsp.next_stereo_pair(&ram);
            assert!(left > 1000);
            assert_eq!(left, right);
        }
        assert_eq!(dsp.read_register(0x7c), 0);
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
        ram[0x401..0x409].fill(0x88);
        // Silence voice zero and drive only voice one to the left channel.
        dsp.write_register(0x00, 0, &ram);
        dsp.write_register(0x10, 0x7f, &ram);
        dsp.write_register(0x11, 0, &ram);
        dsp.write_register(0x13, 0x10, &ram);
        dsp.write_register(0x14, 1, &ram);
        dsp.write_register(0x17, 0x7f, &ram);
        dsp.write_register(0x4c, 0b10, &ram);
        for _ in 0..48 {
            let (left, right) = dsp.next_stereo_pair(&ram);
            assert!(left < -1000, "voice one must retain its own loop source");
            assert_eq!(right, 0);
        }
        assert_eq!(dsp.read_register(0x7c) & 0b10, 0);
    }

    #[test]
    fn voice_end_sets_endx_and_muting_stops_output() {
        let (mut dsp, mut ram) = looping_sample();
        ram[0x300] = 0xc1; // End without loop.
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..16 {
            assert!(dsp.next_stereo_pair(&ram).0 > 0);
        }
        assert_eq!(dsp.next_stereo_pair(&ram), (0, 0));
        assert_eq!(dsp.read_register(0x7c) & 1, 1);
        dsp.write_register(0x7c, 0xff, &ram);
        assert_eq!(dsp.read_register(0x7c), 0);
    }

    #[test]
    fn key_off_releases_and_global_mute_silences() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x6c, 0x40, &ram);
        assert_eq!(dsp.next_stereo_pair(&ram), (0, 0));
        dsp.write_register(0x6c, 0x00, &ram);
        assert!(dsp.next_stereo_pair(&ram).0 > 0);
        dsp.write_register(0x5c, 1, &ram);
        for _ in 0..300 {
            let _ = dsp.next_stereo_pair(&ram);
        }
        assert_eq!(dsp.next_stereo_pair(&ram), (0, 0));
    }

    #[test]
    fn envelope_rate_table_and_gain_modes_follow_shared_sample_clock() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.voices[0].envelope, 0);

        // Direct GAIN always applies immediately, independent of rate.
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, 0x7f0);
        dsp.write_register(0x07, 0x9f, &ram); // GAIN linear down, rate 31.
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, 0x7d0);
        dsp.write_register(0x07, 0xbf, &ram); // Exponential down, rate 31.
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, 0x7d0 - 1 - (0x7d0 >> 8));
        dsp.write_register(0x07, 0xdf, &ram); // Linear up, rate 31.
        let before = dsp.voices[0].envelope;
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, before + 0x20);
        dsp.write_register(0x07, 0xff, &ram); // Bent increase, rate 31.
        let before = dsp.voices[0].envelope;
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, before + 8);
        dsp.write_register(0x07, 0x80, &ram); // Rate zero: never updates.
        let before = dsp.voices[0].envelope;
        for _ in 0..64 {
            dsp.next_stereo_pair(&ram);
        }
        assert_eq!(dsp.voices[0].envelope, before);
    }

    #[test]
    fn adsr_attack_decay_sustain_are_rate_controlled_and_serializable() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x05, 0xff, &ram); // Attack 15, decay 7.
        dsp.write_register(0x06, 0x00, &ram); // Sustain level 0, rate disabled.
        dsp.write_register(0x4c, 1, &ram);
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Attack);
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, 0x400);
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, 0x7ff);
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Decay);
        for _ in 0..4096 {
            dsp.next_stereo_pair(&ram);
            if dsp.voices[0].envelope_phase == super::EnvelopePhase::Sustain {
                break;
            }
        }
        assert_eq!(dsp.voices[0].envelope_phase, super::EnvelopePhase::Sustain);
        let frozen = dsp.voices[0].envelope;
        for _ in 0..64 {
            dsp.next_stereo_pair(&ram);
        }
        assert_eq!(dsp.voices[0].envelope, frozen);

        let saved = serde_json::to_string(&dsp).unwrap();
        let mut resumed: Dsp = serde_json::from_str(&saved).unwrap();
        dsp.write_register(0x06, 0x1f, &ram); // Enable rapid sustain decay.
        resumed.write_register(0x06, 0x1f, &ram);
        for _ in 0..128 {
            assert_eq!(dsp.next_stereo_pair(&ram), resumed.next_stereo_pair(&ram));
        }
        assert_eq!(dsp.voices[0].envelope, resumed.voices[0].envelope);
        assert!(dsp.voices[0].envelope < frozen);
    }

    #[test]
    fn slow_gain_rate_obeys_period_and_is_independent_of_audio_chunking() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.next_stereo_pair(&ram); // Initialize direct GAIN.
        dsp.write_register(0x07, 0x81, &ram); // Linear down at rate 1 (2048 ticks).
        let baseline = dsp.voices[0].envelope;
        for _ in 0..2046 {
            dsp.next_stereo_pair(&ram);
        }
        assert_eq!(dsp.voices[0].envelope, baseline);
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.voices[0].envelope, baseline - 0x20);
    }

    #[test]
    fn pitch_modulation_uses_previous_enveloped_voice_output() {
        let (mut base, ram) = looping_sample();
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
            let reference = base.next_stereo_pair(&ram);
            modulated.next_stereo_pair(&ram);
            assert_eq!(ignored_voice_zero.next_stereo_pair(&ram), reference);
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
        ram[0x301..0x309].fill(0x88);
        ram[0x204] = 0x00;
        ram[0x205] = 0x04;
        ram[0x206] = 0x00;
        ram[0x207] = 0x04;
        ram[0x400] = 0xc3;
        ram[0x401..0x409].fill(0x77);
        base.write_register(0x12, 0, &ram);
        base.write_register(0x13, 0x08, &ram);
        base.write_register(0x14, 1, &ram);
        base.write_register(0x17, 0x7f, &ram);
        base.write_register(0x4c, 0b11, &ram);

        let mut modulated = base.clone();
        modulated.write_register(0x2d, 0b10, &ram);
        base.next_stereo_pair(&ram);
        modulated.next_stereo_pair(&ram);
        assert!(modulated.voices[1].fraction < base.voices[1].fraction);

        let saved = serde_json::to_string(&modulated).unwrap();
        let mut restored: Dsp = serde_json::from_str(&saved).unwrap();
        for _ in 0..100 {
            assert_eq!(
                modulated.next_stereo_pair(&ram),
                restored.next_stereo_pair(&ram)
            );
        }
        assert_eq!(modulated.voices[1].index, restored.voices[1].index);
        assert_eq!(modulated.voices[1].fraction, restored.voices[1].fraction);
    }

    #[test]
    fn noise_lfsr_uses_shared_clock_and_serializes_without_reset() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x3d, 1, &ram); // NON voice 0.
        dsp.write_register(0x6c, 0x1f, &ram); // Highest noise clock rate.
        assert_eq!(dsp.noise_lfsr, 0x4000);
        let first = dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.noise_lfsr, 0x2000);
        assert!(first.0 > 0);
        assert_eq!(first.0, first.1);
        dsp.next_stereo_pair(&ram);
        assert_eq!(dsp.noise_lfsr, 0x1000);

        let snapshot = serde_json::to_string(&dsp).unwrap();
        let mut restored: Dsp = serde_json::from_str(&snapshot).unwrap();
        for _ in 0..128 {
            assert_eq!(dsp.next_stereo_pair(&ram), restored.next_stereo_pair(&ram));
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
            dsp.next_stereo_pair(&[]);
        }
        assert_eq!(dsp.noise_lfsr, 0x4000);
        assert!(!visited[0]);
    }

    #[test]
    fn noise_rate_zero_holds_lfsr_while_non_toggles_waveform() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        dsp.write_register(0x6c, 0, &ram); // Rate zero never clocks LFSR.
        dsp.write_register(0x3d, 1, &ram);
        let noise_output = dsp.next_stereo_pair(&ram);
        for _ in 0..4096 {
            let _ = dsp.next_stereo_pair(&ram);
        }
        assert_eq!(dsp.noise_lfsr, 0x4000);
        assert!(noise_output.0 < 0); // Signed 15-bit $4000.
        dsp.write_register(0x3d, 0, &ram);
        let sample_output = dsp.next_stereo_pair(&ram);
        assert!(sample_output.0 > 0); // Original positive BRR sample.
        assert_eq!(sample_output.0, sample_output.1);
    }

    #[test]
    fn two_non_voices_share_the_same_noise_waveform() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x10, 0x7f, &ram);
        dsp.write_register(0x11, 0, &ram);
        dsp.write_register(0x13, 0x10, &ram);
        dsp.write_register(0x17, 0x7f, &ram);
        dsp.write_register(0x3d, 0b11, &ram);
        dsp.write_register(0x6c, 0x1f, &ram);
        dsp.write_register(0x4c, 0b11, &ram);
        let both = dsp.next_stereo_pair(&ram);
        assert!(both.0 > 0);
        assert!(both.1 > 0);
        // Integer master-volume scaling rounds twice; allow two LSBs.
        assert!((i32::from(both.0) - 2 * i32::from(both.1)).abs() <= 2);
    }

    #[test]
    fn dsp_state_serializes_across_sample_boundaries() {
        let (mut dsp, ram) = looping_sample();
        dsp.write_register(0x4c, 1, &ram);
        for _ in 0..5 {
            let _ = dsp.next_stereo_pair(&ram);
        }
        let state = serde_json::to_string(&dsp).unwrap();
        let mut resumed: Dsp = serde_json::from_str(&state).unwrap();
        for _ in 0..50 {
            assert_eq!(dsp.next_stereo_pair(&ram), resumed.next_stereo_pair(&ram));
        }
    }
}
