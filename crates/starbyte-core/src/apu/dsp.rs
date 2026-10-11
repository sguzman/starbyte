//! Partial SNES S-DSP voice mixer: BRR block decoding and fixed-gain playback.
//!
//! The DSP consumes SPC700 audio RAM, not the main SNES CPU memory bus. This
//! implementation supports directory-based BRR samples, pitch stepping, signed
//! channel/master volumes, direct GAIN, sample end/loop flags, and a bounded
//! preliminary ADSR/release envelope. Gaussian interpolation, exact envelope
//! rate tables, noise, echo, pitch modulation and hardware pipeline latency
//! still need work. Never describe this as complete or bit-exact DSP audio.

use serde::{Deserialize, Serialize};

const VOICE_COUNT: usize = 8;
const REGISTER_COUNT: usize = 128;
const BRR_SAMPLES_PER_BLOCK: usize = 16;
const PITCH_UNIT: u32 = 0x1000;
/// Shared 32 kHz envelope clock wraps after 30,720 DSP sample periods.
const ENVELOPE_COUNTER_WRAP: u16 = 30_720;
/// SPC DSP envelope periods in audio samples. Rate zero never clocks.
const ENVELOPE_PERIODS: [u16; 32] = [
    0, 2048, 1536, 1280, 1024, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64,
    48, 40, 32, 24, 20, 16, 12, 10, 8, 6, 5, 4, 3, 2, 1,
];
const ENVELOPE_OFFSETS: [u16; 32] = [
    0, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 536,
    0, 1040, 536, 0, 1040, 536, 0, 1040, 536, 0, 1040, 0, 0,
];

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
}

impl Default for Dsp {
    fn default() -> Self {
        Self {
            registers: vec![0; REGISTER_COUNT],
            voices: vec![Voice::default(); VOICE_COUNT],
            envelope_counter: 0,
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
        let mut left = 0_i64;
        let mut right = 0_i64;
        for index in 0..VOICE_COUNT {
            let base = index * 16;
            let pitch = (u16::from(self.registers[base + 3] & 0x3f) << 8)
                | u16::from(self.registers[base + 2]);
            let adsr1 = self.registers[base + 5];
            let adsr2 = self.registers[base + 6];
            let gain = self.registers[base + 7];
            let sample = self.voices[index].clock(
                index,
                pitch,
                adsr1,
                adsr2,
                gain,
                self.envelope_counter,
                ram,
                &self.registers,
            );
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
        ram: &[u8],
        registers: &[u8],
    ) -> i16 {
        if !self.active {
            return 0;
        }

        self.advance_envelope(adsr1, adsr2, gain, counter);

        let result = (i32::from(self.samples[self.index]) * self.envelope / 2048) as i16;
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

    /// Rate-controlled ADSR and GAIN. Noise, interpolation and DSP hardware
    /// pipeline latency are separate outstanding fidelity tasks.
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
                0 => self.envelope - 0x20, // Linear decrease.
                1 => self.envelope - 1 - (self.envelope >> 8), // Exponential decrease.
                2 => self.envelope + 0x20, // Linear increase.
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
            let predicted = match filter {
                0 => decoded,
                1 => decoded + ((self.previous * 15) >> 4),
                2 => decoded + ((self.previous * 61) >> 5) - ((self.older * 15) >> 4),
                _ => decoded + ((self.previous * 115) >> 6) - ((self.older * 13) >> 4),
            };
            let output = predicted.clamp(-16384, 16383);
            self.samples[i] = (output << 1) as i16;
            self.older = self.previous;
            self.previous = output;
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
        for _ in 0..1024 {
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
