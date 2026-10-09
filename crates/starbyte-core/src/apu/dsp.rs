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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dsp {
    registers: Vec<u8>,
    voices: Vec<Voice>,
}

impl Default for Dsp {
    fn default() -> Self {
        Self {
            registers: vec![0; REGISTER_COUNT],
            voices: vec![Voice::default(); VOICE_COUNT],
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
        let mut left = 0_i64;
        let mut right = 0_i64;
        for index in 0..VOICE_COUNT {
            let base = index * 16;
            let pitch = (u16::from(self.registers[base + 3] & 0x3f) << 8)
                | u16::from(self.registers[base + 2]);
            let adsr1 = self.registers[base + 5];
            let gain = self.registers[base + 7];
            let sample = self.voices[index].clock(index, pitch, adsr1, gain, ram, &self.registers);
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
        gain: u8,
        ram: &[u8],
        registers: &[u8],
    ) -> i16 {
        if !self.active {
            return 0;
        }

        // Direct GAIN is the first supported envelope mode. The simplified
        // ADSR attack is intentionally bounded, not hardware rate-accurate.
        if self.releasing {
            self.envelope = (self.envelope - 8).max(0);
            if self.envelope == 0 {
                self.active = false;
                return 0;
            }
        } else if adsr1 & 0x80 != 0 {
            let attack = i32::from(adsr1 & 0x0f) + 1;
            self.envelope = (self.envelope + attack).min(2047);
        } else if gain & 0x80 == 0 {
            self.envelope = i32::from(gain) << 4;
        }

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
                2 => {
                    decoded + ((self.previous * 61) >> 5) - ((self.older * 15) >> 4)
                }
                _ => {
                    decoded + ((self.previous * 115) >> 6) - ((self.older * 13) >> 4)
                }
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
    u16::from(read_ram(ram, address))
        | (u16::from(read_ram(ram, address.wrapping_add(1))) << 8)
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
