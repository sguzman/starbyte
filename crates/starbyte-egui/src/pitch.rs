//! CPU-only streaming WSOLA pitch-preserving time stretch.
//!
//! Game DSP provides 32 kHz stereo PCM in short SNES-frame chunks. Rodio's
//! varispeed alone changes both tempo and pitch. This module shortens/lengthens
//! audio by waveform-similarity overlap-add so tempo and pitch are independent.
//! It is deliberately processed on Starbyte's existing audio worker thread.

const CHANNELS: usize = 2;
const WINDOW: usize = 384; // 12 ms at 32 kHz
const HOP: usize = WINDOW / 2;
const SEARCH: usize = 32; // 1 ms correction to minimize boundary artifacts

/// Streaming stereo waveform-similarity overlap-add (WSOLA).
///
/// Each complete window contributes half an overlapped output window. The
/// analysis cursor moves by HOP * tempo, while the output cursor moves by HOP.
/// Thus tempo 2.0 consumes twice as much source audio per output second without
/// changing the sample rate or moving the waveform's local fundamental pitch.
pub struct PitchStretch {
    input: Vec<f32>,
    base_frame: usize,
    next_analysis: f64,
    last_tail: Option<Vec<f32>>,
    tempo: f32,
}

impl Default for PitchStretch {
    fn default() -> Self {
        Self {
            input: Vec::new(),
            base_frame: 0,
            next_analysis: 0.0,
            last_tail: None,
            tempo: 1.0,
        }
    }
}

impl PitchStretch {
    /// Release all pending input and overlap state on ROM or speed/pitch change.
    pub fn reset(&mut self) {
        self.input.clear();
        self.base_frame = 0;
        self.next_analysis = 0.0;
        self.last_tail = None;
        self.tempo = 1.0;
    }

    /// Stretch a short interleaved stereo buffer at an independent tempo.
    ///
    /// For a sound pitch multiplier P and game speed S, call with tempo S/P,
    /// then ask the sink to play the output at P. The total output duration
    /// becomes original_duration/S while the perceived pitch becomes P.
    /// Tempo 1 is losslessly bypassed to preserve the native DSP waveform.
    pub fn process(&mut self, pcm: &[f32], tempo: f32) -> Vec<f32> {
        if pcm.is_empty() || pcm.len() % CHANNELS != 0 {
            return Vec::new();
        }
        let tempo = if tempo.is_finite() {
            tempo.clamp(0.25, 8.0)
        } else {
            1.0
        };
        if (self.tempo - tempo).abs() > 0.0001 {
            self.reset();
            self.tempo = tempo;
        }
        if (tempo - 1.0).abs() < 0.0001 {
            return pcm.to_vec();
        }
        self.input.extend_from_slice(pcm);
        let mut produced = Vec::new();
        // At 0.25x a normal 533-frame SPC buffer generates ~2k output
        // frames. This bound is above any plausible input/tempo combination.
        for _ in 0..128 {
            let available = self.base_frame + self.input.len() / CHANNELS;
            let ideal = self.next_analysis.round().max(0.0) as usize;
            let low = ideal.saturating_sub(SEARCH).max(self.base_frame);
            if available < low.saturating_add(WINDOW) {
                break;
            }
            let high = ideal
                .saturating_add(SEARCH)
                .min(available.saturating_sub(WINDOW));
            let start = if let Some(previous) = &self.last_tail {
                best_overlap_start(&self.input, self.base_frame, low, high, previous)
            } else {
                ideal.min(high).max(low)
            };
            let slice_start = (start - self.base_frame) * CHANNELS;
            let window = &self.input[slice_start..slice_start + WINDOW * CHANNELS];
            if let Some(old_tail) = &self.last_tail {
                for frame in 0..HOP {
                    let w = (frame as f32 + 0.5) / HOP as f32;
                    for channel in 0..CHANNELS {
                        let index = frame * CHANNELS + channel;
                        produced.push(old_tail[index].mul_add(1.0 - w, window[index] * w));
                    }
                }
            } else {
                produced.extend_from_slice(&window[..HOP * CHANNELS]);
            }
            self.last_tail = Some(window[HOP * CHANNELS..].to_vec());
            self.next_analysis += HOP as f64 * f64::from(tempo);
        }
        // Bound memory during long game sessions. A lookback margin permits
        // cross-correlation on the next chunk without retaining old minutes.
        let keep_from = (self.next_analysis.floor() as usize)
            .saturating_sub(SEARCH)
            .min(self.base_frame + self.input.len() / CHANNELS);
        let dropped = keep_from.saturating_sub(self.base_frame);
        if dropped > 0 {
            self.input.drain(..dropped * CHANNELS);
            self.base_frame += dropped;
        }
        produced
    }
}

fn best_overlap_start(
    input: &[f32],
    base_frame: usize,
    low: usize,
    high: usize,
    tail: &[f32],
) -> usize {
    let mut best = low;
    let mut best_score = f64::NEG_INFINITY;
    for start in low..=high {
        let offset = (start - base_frame) * CHANNELS;
        let current = &input[offset..offset + HOP * CHANNELS];
        let mut dot = 0.0f64;
        let mut energy = 1e-12f64;
        // Compare both channels so stereo phase is not reduced to mono.
        // A 2-frame stride bounds CPU work without materially changing
        // position selection for normal SNES sound frequencies.
        for i in (0..HOP * CHANNELS).step_by(4) {
            let x = f64::from(tail[i]);
            let y = f64::from(current[i]);
            dot += x * y;
            energy += y * y;
        }
        let score = dot / energy.sqrt();
        if score > best_score {
            best_score = score;
            best = start;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::PitchStretch;
    use std::f32::consts::TAU;

    fn stereo_tone(frames: usize, frequency: f32) -> Vec<f32> {
        let mut samples = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let value = (TAU * frequency * index as f32 / 32_000.0).sin() * 0.25;
            samples.extend_from_slice(&[value, value]);
        }
        samples
    }

    fn zero_crossing_hz(samples: &[f32]) -> f32 {
        let mono: Vec<f32> = samples.chunks_exact(2).map(|pair| pair[0]).collect();
        let crossings = mono.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        crossings as f32 * 32_000.0 / mono.len() as f32
    }

    #[test]
    fn original_speed_is_bit_identical_and_invalid_data_is_safe() {
        let source = stereo_tone(533, 440.0);
        let mut processor = PitchStretch::default();
        assert_eq!(processor.process(&source, 1.0), source);
        assert!(processor.process(&[1.0], 1.0).is_empty());
        assert_eq!(processor.process(&source, f32::NAN), source);
    }

    #[test]
    fn pitch_preserving_slow_motion_and_fast_forward_change_length_not_tone() {
        let source = stereo_tone(32_000, 440.0);
        for (tempo, expected_frames) in [(0.5, 64_000), (2.0, 16_000), (4.0, 8_000)] {
            let mut processor = PitchStretch::default();
            let mut rendered = Vec::new();
            for chunk in source.chunks(533 * 2) {
                rendered.extend(processor.process(chunk, tempo));
            }
            let actual = rendered.len() / 2;
            assert!(
                actual.abs_diff(expected_frames) < 1_000,
                "tempo {tempo}, got {actual} output frames instead of {expected_frames}"
            );
            let hz = zero_crossing_hz(&rendered);
            assert!(
                (hz - 440.0).abs() < 90.0,
                "tempo {tempo}, synthesized pitch {hz} Hz instead of near 440 Hz"
            );
        }
    }

    #[test]
    fn change_of_tempo_discards_old_overlap_but_keeps_audio_streaming() {
        let mut processor = PitchStretch::default();
        let source = stereo_tone(533 * 4, 220.0);
        assert!(!processor.process(&source, 2.0).is_empty());
        assert!(!processor.process(&source, 0.5).is_empty());
        assert_eq!(processor.process(&source, 1.0), source);
    }
}
