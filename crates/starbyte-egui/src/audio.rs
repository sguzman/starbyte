//! Best-effort desktop audio output for the emulator's 32 kHz stereo PCM.
//!
//! Starbyte does not require an audio device to play. The output is created
//! only when real nonzero PCM arrives, and never enables the experimental
//! SPC700 driver in an ordinary emulation session.

use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamBuilder, Sink, buffer::SamplesBuffer};

const DSP_SAMPLE_RATE: u32 = 32_000;
const CHANNELS: u16 = 2;
/// Roughly six NTSC frames of headroom; stale sound must never accumulate
/// seconds of latency if the GUI or the game runs ahead of the audio device.
const MAX_PENDING_FRAMES: usize = 6;

pub struct AudioOutput {
    // The stream must remain alive for as long as the sink is playing.
    _stream: OutputStream,
    sink: Sink,
}

impl AudioOutput {
    pub fn open_default() -> Result<Self> {
        let mut stream = OutputStreamBuilder::open_default_stream()
            .context("could not open a desktop audio output device")?;
        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());
        Ok(Self {
            _stream: stream,
            sink,
        })
    }

    /// Queue one emulator frame without blocking for playback. Rodio converts
    /// 32 kHz stereo to the physical device format on its audio thread.
    pub fn enqueue(&self, samples: &[i16]) {
        let Some(pcm) = pcm16_stereo_to_float(samples) else {
            return;
        };
        // If rendering falls behind output for any reason, discard stale
        // buffers instead of making game sound increasingly delayed.
        let excess = self
            .sink
            .len()
            .saturating_sub(MAX_PENDING_FRAMES.saturating_sub(1));
        for _ in 0..excess {
            // Skipping stale queued frames is nonblocking; clearing the
            // entire sink here could pause the GUI during normal playback.
            self.sink.skip_one();
        }
        self.sink
            .append(SamplesBuffer::new(CHANNELS, DSP_SAMPLE_RATE, pcm));
        self.sink.play();
    }

    /// A standalone speaker smoke test. It uses synthesized samples rather
    /// than a commercial ROM and does not change the production APU path.
    pub fn test_speakers() -> Result<()> {
        let output = Self::open_default()?;
        output.enqueue(&speaker_test_tone());
        // Allow the output stream to consume the quarter-second tone before
        // its owner is dropped. This is invoked only by an explicit CLI flag.
        std::thread::sleep(std::time::Duration::from_millis(350));
        Ok(())
    }

    /// Drop queued audio on pause, state restore, cartridge change, or error.
    pub fn clear(&self) {
        self.sink.clear();
    }
}

/// Return interleaved stereo floats in [-1, 1], or nothing for an empty or
/// malformed frame. Conversion is deterministic and independent of hardware.
fn pcm16_stereo_to_float(samples: &[i16]) -> Option<Vec<f32>> {
    if samples.is_empty() || samples.len() % 2 != 0 {
        return None;
    }
    Some(
        samples
            .iter()
            .map(|&sample| f32::from(sample) / 32_768.0)
            .collect(),
    )
}

/// Copyright-free, low-amplitude stereo sine burst with short fades.
/// Audio hardware is not needed to validate its length and channel layout.
fn speaker_test_tone() -> Vec<i16> {
    const TONE_HZ: f32 = 440.0;
    const TONE_FRAMES: usize = DSP_SAMPLE_RATE as usize / 4;
    const FADE_FRAMES: f32 = 256.0;
    let mut output = Vec::with_capacity(TONE_FRAMES * usize::from(CHANNELS));
    for index in 0..TONE_FRAMES {
        let time = index as f32 / DSP_SAMPLE_RATE as f32;
        let attack = (index as f32 / FADE_FRAMES).min(1.0);
        let release = ((TONE_FRAMES - index) as f32 / FADE_FRAMES).min(1.0);
        let amplitude = attack.min(release);
        let sample = (std::f32::consts::TAU * TONE_HZ * time).sin()
            * amplitude
            * 0.15
            * f32::from(i16::MAX);
        let sample = sample as i16;
        output.extend_from_slice(&[sample, sample]);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{pcm16_stereo_to_float, speaker_test_tone};

    #[test]
    fn stereo_conversion_preserves_channel_order_and_signed_full_scale() {
        let samples = [-32_768, 32_767, -16_384, 16_384, 0, 0];
        let result = pcm16_stereo_to_float(&samples).unwrap();
        assert_eq!(result.len(), samples.len());
        assert_eq!(result[0], -1.0);
        assert!(result[1] < 1.0);
        assert!(result[1] > 0.999);
        assert_eq!(result[2], -0.5);
        assert_eq!(result[3], 0.5);
        assert_eq!(result[4], 0.0);
    }

    #[test]
    fn speaker_test_is_bounded_symmetric_and_does_not_need_a_device() {
        let pcm = speaker_test_tone();
        assert_eq!(pcm.len(), 32_000 / 4 * 2);
        assert_eq!(&pcm[..2], &[0, 0]);
        assert!(pcm.chunks_exact(2).all(|pair| pair[0] == pair[1]));
        assert!(pcm.iter().any(|&sample| sample > 1_000));
        assert!(pcm.iter().any(|&sample| sample < -1_000));
        assert!(pcm.iter().all(|&sample| sample.unsigned_abs() <= 5_000));
    }

    #[test]
    fn malformed_or_empty_frames_are_ignored_without_opening_audio_device() {
        assert!(pcm16_stereo_to_float(&[]).is_none());
        assert!(pcm16_stereo_to_float(&[1]).is_none());
        assert!(pcm16_stereo_to_float(&[1, 2, 3]).is_none());
        assert_eq!(pcm16_stereo_to_float(&[0, 0]), Some(vec![0.0, 0.0]));
    }
}
