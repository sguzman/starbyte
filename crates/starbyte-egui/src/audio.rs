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

#[cfg(test)]
mod tests {
    use super::pcm16_stereo_to_float;

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
    fn malformed_or_empty_frames_are_ignored_without_opening_audio_device() {
        assert!(pcm16_stereo_to_float(&[]).is_none());
        assert!(pcm16_stereo_to_float(&[1]).is_none());
        assert!(pcm16_stereo_to_float(&[1, 2, 3]).is_none());
        assert_eq!(pcm16_stereo_to_float(&[0, 0]), Some(vec![0.0, 0.0]));
    }
}
