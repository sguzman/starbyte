//! Bounded, nonblocking desktop playback for synthesized 32 kHz stereo PCM.
//!
//! Sound device initialization and Rodio queueing run on a dedicated worker;
//! they must not freeze the egui event loop or enable commercial SPC execution.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamBuilder, Sink, buffer::SamplesBuffer};
use tracing::warn;

const DSP_SAMPLE_RATE: u32 = 32_000;
const CHANNELS: u16 = 2;
/// A bounded producer backlog; excess frames are discarded, never blocked.
const MAX_PENDING_FRAMES: usize = 6;
const AUDIO_RETRY_DELAY: Duration = Duration::from_secs(10);
/// A slow device open must never replay seconds-old game audio.
const MAX_FRAME_AGE: Duration = Duration::from_millis(150);

enum AudioMessage {
    Frame {
        generation: u64,
        queued_at: Instant,
        pcm: Vec<i16>,
    },
    Clear,
}

/// The UI owns only a bounded nonblocking sender, never a device or mixer.
/// Dropping the last sender allows the audio worker to shut down naturally.
pub struct AudioOutput {
    sender: SyncSender<AudioMessage>,
    generation: Arc<AtomicU64>,
}

impl AudioOutput {
    pub fn start() -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_FRAMES);
        let generation = Arc::new(AtomicU64::new(0));
        let shared_generation = Arc::clone(&generation);
        // The GUI must not join this thread: the bounded channel owns its
        // lifecycle and disconnects when the last AudioOutput is dropped.
        let _audio_thread = thread::Builder::new()
            .name("starbyte-audio".to_owned())
            .spawn(move || audio_worker(receiver, shared_generation))
            .context("could not spawn background audio thread")?;
        Ok(Self { sender, generation })
    }

    /// Bounded try-send: full queues lose stale audio rather than blocking
    /// the UI or accumulating seconds of sound latency.
    pub fn enqueue(&self, samples: &[i16]) {
        if samples.is_empty() || samples.len() % 2 != 0 {
            return;
        }
        let _ = self.sender.try_send(AudioMessage::Frame {
            generation: self.generation.load(Ordering::Acquire),
            queued_at: Instant::now(),
            pcm: samples.to_vec(),
        });
    }

    /// Mark every queued frame stale immediately, and wake an idle worker.
    /// Unlike a blocking send, this cannot freeze the UI on pause/ROM load.
    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.try_send(AudioMessage::Clear);
    }

    /// A standalone speaker probe. It uses original synthetic samples rather
    /// than ROM data and never starts the production emulator or window.
    pub fn test_speakers() -> Result<()> {
        let output = PlaybackDevice::open_default()?;
        output.enqueue(&speaker_test_tone());
        thread::sleep(Duration::from_millis(350));
        Ok(())
    }
}

/// Keep the mixer sink alive before releasing its backing output stream.
struct PlaybackDevice {
    sink: Sink,
    _stream: OutputStream,
}

impl PlaybackDevice {
    fn open_default() -> Result<Self> {
        let mut stream = OutputStreamBuilder::open_default_stream()
            .context("could not open a desktop audio output device")?;
        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());
        Ok(Self {
            sink,
            _stream: stream,
        })
    }

    fn enqueue(&self, samples: &[i16]) {
        let Some(pcm) = pcm16_stereo_to_float(samples) else {
            return;
        };
        // Skip old individual buffers without clearing the live sink.
        let excess = self
            .sink
            .len()
            .saturating_sub(MAX_PENDING_FRAMES.saturating_sub(1));
        for _ in 0..excess {
            self.sink.skip_one();
        }
        self.sink
            .append(SamplesBuffer::new(CHANNELS, DSP_SAMPLE_RATE, pcm));
        self.sink.play();
    }

    fn clear(&self) {
        self.sink.clear();
    }
}

fn retry_ready(retry_after: Option<Instant>, now: Instant) -> bool {
    retry_after.is_none_or(|deadline| now >= deadline)
}

fn frame_is_fresh(queued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(queued_at) <= MAX_FRAME_AGE
}

fn audio_worker(receiver: Receiver<AudioMessage>, generation: Arc<AtomicU64>) {
    let mut device: Option<PlaybackDevice> = None;
    let mut retry_after: Option<Instant> = None;
    let mut active_generation = generation.load(Ordering::Acquire);

    for message in receiver {
        let newest_generation = generation.load(Ordering::Acquire);
        if newest_generation != active_generation {
            if let Some(output) = &device {
                output.clear();
            }
            active_generation = newest_generation;
        }

        let AudioMessage::Frame {
            generation: frame_generation,
            queued_at,
            pcm,
        } = message
        else {
            continue;
        };
        if frame_generation != active_generation || !frame_is_fresh(queued_at, Instant::now()) {
            continue;
        }
        if device.is_none()
            && pcm.iter().any(|&sample| sample != 0)
            && retry_ready(retry_after, Instant::now())
        {
            match PlaybackDevice::open_default() {
                Ok(output) => {
                    device = Some(output);
                    retry_after = None;
                }
                Err(error) => {
                    warn!("desktop audio unavailable (video remains playable): {error}");
                    retry_after = Some(Instant::now() + AUDIO_RETRY_DELAY);
                }
            }
        }
        if let Some(output) = &device {
            // Device initialization can take time. Drop a frame that became
            // invalid while the worker was waiting for system audio.
            if generation.load(Ordering::Acquire) == frame_generation
                && frame_is_fresh(queued_at, Instant::now())
            {
                output.enqueue(&pcm);
            } else {
                output.clear();
                active_generation = generation.load(Ordering::Acquire);
            }
        }
    }
}

/// Return interleaved stereo floats in [-1, 1], or nothing for an empty or
/// malformed frame. Conversion is deterministic and hardware-independent.
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

/// Original low-volume stereo sine burst with short fade-in and fade-out.
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
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };

    use super::{
        AudioOutput, frame_is_fresh, pcm16_stereo_to_float, retry_ready, speaker_test_tone,
    };

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

    #[test]
    fn queue_invalidation_does_not_need_a_working_speaker() {
        let output = AudioOutput::start().unwrap();
        let before = output.generation.load(Ordering::Acquire);
        output.enqueue(&[0, 0]);
        output.clear();
        let after = output.generation.load(Ordering::Acquire);
        assert_eq!(after, before + 1);
        output.enqueue(&[0, 0]);
    }

    #[test]
    fn delayed_audio_frames_are_expired_before_hardware_queueing() {
        let t0 = Instant::now();
        assert!(frame_is_fresh(t0, t0));
        assert!(frame_is_fresh(t0, t0 + Duration::from_millis(150)));
        assert!(!frame_is_fresh(t0, t0 + Duration::from_millis(151)));
        assert!(!frame_is_fresh(t0, t0 + Duration::from_secs(2)));
    }

    #[test]
    fn device_retries_respect_cooldown() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(10);
        assert!(retry_ready(None, now));
        assert!(!retry_ready(Some(deadline), now));
        assert!(!retry_ready(Some(deadline), now + Duration::from_secs(9)));
        assert!(retry_ready(Some(deadline), deadline));
    }
}
