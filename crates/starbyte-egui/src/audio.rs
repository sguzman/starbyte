//! Bounded, nonblocking desktop playback for synthesized 32 kHz stereo PCM.
//!
//! Sound device initialization and Rodio queueing run on a dedicated worker;
//! they must not freeze the egui event loop or enable commercial SPC execution.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rodio::{
    OutputStream, OutputStreamBuilder, Sink, buffer::SamplesBuffer, cpal::traits::HostTrait,
};
use tracing::warn;

use crate::pitch::PitchStretch;

const DSP_SAMPLE_RATE: u32 = 32_000;
const CHANNELS: u16 = 2;
/// A bounded producer backlog; excess frames are discarded, never blocked.
const MAX_PENDING_FRAMES: usize = 6;
const AUDIO_RETRY_DELAY: Duration = Duration::from_secs(10);
/// Check an already-open output for callback failures even without new PCM.
/// This does not open an unused audio device during silent startup.
const AUDIO_HEALTH_POLL: Duration = Duration::from_millis(250);
/// A slow device open must never replay seconds-old game audio.
const MAX_FRAME_AGE: Duration = Duration::from_millis(150);
/// The test tone occupies the real game output sink for a quarter second;
/// suppress regular emulated PCM until it has completed playing.
const TEST_TONE_EXCLUSIVE: Duration = Duration::from_millis(300);

/// Last known hardware output discovery state of the desktop worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDeviceStatus {
    /// The worker has not attempted to open an OS device yet.
    NotAttempted,
    /// Rodio successfully initialized the default OS playback device.
    Opened,
    /// The last device-open attempt failed; background retries remain armed.
    OpenFailed,
}

impl AudioDeviceStatus {
    const fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Opened,
            2 => Self::OpenFailed,
            _ => Self::NotAttempted,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Opened => "opened",
            Self::OpenFailed => "open_failed",
        }
    }
}

enum AudioMessage {
    Frame {
        generation: u64,
        queued_at: Instant,
        pcm: Vec<i16>,
        /// Emulated game speed when the sound was synthesized.
        speed: f32,
        /// Pitch multiplier independent of game tempo; 1.0 keeps SNES pitch.
        pitch: f32,
    },
    /// Original generated sine burst, routed through the *same* Rodio stream
    /// and sink as real game PCM (unlike a separately opened speaker probe).
    TestTone {
        generation: u64,
        queued_at: Instant,
    },
    Clear,
    /// Apply a live volume change to already-queued Rodio buffers even when
    /// the emulator is paused and sends no fresh game PCM.
    VolumeChanged,
    /// Release the current output route, even if CPAL never reported an
    /// error after a device/default-sink change.
    Reopen,
}

/// The UI owns only a bounded nonblocking sender, never a device or mixer.
/// Dropping the last sender allows the audio worker to shut down naturally.
pub struct AudioOutput {
    sender: SyncSender<AudioMessage>,
    generation: Arc<AtomicU64>,
    device_status: Arc<AtomicU8>,
    volume_bits: Arc<AtomicU32>,
    /// Number of fresh PCM frames actually appended to the OS mixer,
    /// not merely sent to the worker or synthesized by the emulator.
    submitted_frames: Arc<AtomicU64>,
    /// Frames with nonzero PCM actually appended to Rodio's mixer.
    /// Distinguishes stream activity from a steady flow of silent buffers.
    submitted_nonzero_frames: Arc<AtomicU64>,
    /// Number of separate test-tone bursts actually submitted to the same
    /// Rodio sink. Never included in game-PCM counters.
    test_tones_submitted: Arc<AtomicU64>,
    /// Game buffers discarded at the bounded producer queue.
    producer_dropped_frames: Arc<AtomicU64>,
    /// Game buffers discarded as stale or superseded by the worker.
    worker_dropped_frames: Arc<AtomicU64>,
    /// Times the Rodio sink was empty when the next game PCM buffer arrived.
    /// A starvation indicator, not proof of a physically audible underrun.
    sink_empty_on_pcm: Arc<AtomicU64>,
    /// CPAL callbacks reporting a formerly opened stream as invalid.
    stream_failures: Arc<AtomicU64>,
    /// An output opened successfully after at least one detected stream fault.
    stream_recoveries: Arc<AtomicU64>,
}

impl AudioOutput {
    pub fn start() -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_FRAMES);
        let generation = Arc::new(AtomicU64::new(0));
        let shared_generation = Arc::clone(&generation);
        let device_status = Arc::new(AtomicU8::new(0));
        let shared_device_status = Arc::clone(&device_status);
        let volume_bits = Arc::new(AtomicU32::new(1.0_f32.to_bits()));
        let shared_volume_bits = Arc::clone(&volume_bits);
        let submitted_frames = Arc::new(AtomicU64::new(0));
        let shared_submitted_frames = Arc::clone(&submitted_frames);
        let submitted_nonzero_frames = Arc::new(AtomicU64::new(0));
        let shared_nonzero_frames = Arc::clone(&submitted_nonzero_frames);
        let test_tones_submitted = Arc::new(AtomicU64::new(0));
        let shared_test_tones = Arc::clone(&test_tones_submitted);
        let producer_dropped_frames = Arc::new(AtomicU64::new(0));
        let worker_dropped_frames = Arc::new(AtomicU64::new(0));
        let shared_worker_drops = Arc::clone(&worker_dropped_frames);
        let sink_empty_on_pcm = Arc::new(AtomicU64::new(0));
        let shared_sink_empty = Arc::clone(&sink_empty_on_pcm);
        let stream_failures = Arc::new(AtomicU64::new(0));
        let shared_stream_failures = Arc::clone(&stream_failures);
        let stream_recoveries = Arc::new(AtomicU64::new(0));
        let shared_stream_recoveries = Arc::clone(&stream_recoveries);
        // The GUI must not join this thread: the bounded channel owns its
        // lifecycle and disconnects when the last AudioOutput is dropped.
        let _audio_thread = thread::Builder::new()
            .name("starbyte-audio".to_owned())
            .spawn(move || {
                audio_worker(
                    receiver,
                    shared_generation,
                    shared_device_status,
                    shared_volume_bits,
                    shared_submitted_frames,
                    shared_nonzero_frames,
                    shared_test_tones,
                    shared_worker_drops,
                    shared_sink_empty,
                    shared_stream_failures,
                    shared_stream_recoveries,
                )
            })
            .context("could not spawn background audio thread")?;
        Ok(Self {
            sender,
            generation,
            device_status,
            volume_bits,
            submitted_frames,
            submitted_nonzero_frames,
            test_tones_submitted,
            producer_dropped_frames,
            worker_dropped_frames,
            sink_empty_on_pcm,
            stream_failures,
            stream_recoveries,
        })
    }

    /// Query the background worker's last OS default-device open result.
    /// Opened does not prove sound reached the physical speakers.
    pub fn device_status(&self) -> AudioDeviceStatus {
        AudioDeviceStatus::from_raw(self.device_status.load(Ordering::Acquire))
    }

    /// Fresh PCM frames submitted to Rodio's playback sink by the worker.
    /// This still cannot verify physical speaker audibility.
    pub fn submitted_frames(&self) -> u64 {
        self.submitted_frames.load(Ordering::Acquire)
    }

    /// Nonzero-PCM frames accepted by the OS mixer. This still cannot prove
    /// a physical speaker was unmuted, routed correctly, or audible.
    pub fn submitted_nonzero_frames(&self) -> u64 {
        self.submitted_nonzero_frames.load(Ordering::Acquire)
    }

    /// Number of generated speaker checks delivered to the live game sink.
    /// A counter increase confirms software delivery, not physical audibility.
    pub fn test_tones_submitted(&self) -> u64 {
        self.test_tones_submitted.load(Ordering::Acquire)
    }

    /// Request a quarter-second, gentle 440 Hz beep through the same audio
    /// device, stream, sink and volume as the running game. Nonblocking; the
    /// worker briefly suppresses game PCM so queue trimming cannot cut it off.
    pub fn enqueue_live_test_tone(&self) -> bool {
        self.sender
            .try_send(AudioMessage::TestTone {
                generation: self.generation.load(Ordering::Acquire),
                queued_at: Instant::now(),
            })
            .is_ok()
    }

    /// Game buffers rejected by the full nonblocking UI-to-worker queue.
    pub fn producer_dropped_frames(&self) -> u64 {
        self.producer_dropped_frames.load(Ordering::Acquire)
    }

    /// Stale/superseded game buffers consumed but rejected by the worker.
    pub fn worker_dropped_frames(&self) -> u64 {
        self.worker_dropped_frames.load(Ordering::Acquire)
    }

    /// Empty-sink events observed immediately before an accepted game frame.
    /// Distinct from producer queue overflow or stale-frame discards.
    pub fn sink_empty_on_pcm(&self) -> u64 {
        self.sink_empty_on_pcm.load(Ordering::Acquire)
    }

    /// CPAL failures observed by the worker after a stream was opened.
    /// This is diagnostic evidence of a stream error, not speaker audibility.
    pub fn stream_failures(&self) -> u64 {
        self.stream_failures.load(Ordering::Acquire)
    }

    /// OS output streams reopened after a detected CPAL stream failure.
    /// The speaker can still be muted or routed elsewhere.
    pub fn stream_recoveries(&self) -> u64 {
        self.stream_recoveries.load(Ordering::Acquire)
    }

    /// Update active playback volume even when the game is paused and no
    /// further PCM arrives. Changes are coalesced into the shared atomic,
    /// then signalled nonblocking to the dedicated audio worker. If the
    /// bounded queue is full, a queued frame will still apply the latest
    /// atomic volume before appending audio.
    pub fn set_volume(&self, volume: f32) {
        let next = normalize_volume(volume).to_bits();
        if self.volume_bits.swap(next, Ordering::AcqRel) != next {
            let _ = self.sender.try_send(AudioMessage::VolumeChanged);
        }
    }

    /// Ask the existing sound worker to release its current CPAL/Rodio route.
    /// The next fresh game PCM or speaker check opens the current default
    /// output again. This is a nonblocking request, not proof of audibility.
    pub fn request_reopen(&self) -> bool {
        self.sender.try_send(AudioMessage::Reopen).is_ok()
    }

    /// Bounded try-send: full queues lose stale audio rather than blocking
    /// the UI or accumulating seconds of sound latency. Rodio follows the
    /// emulation rate and shifts pitch with the speed.
    pub fn enqueue(&self, samples: &[i16], speed: f32, pitch: f32) {
        if samples.is_empty() || samples.len() % 2 != 0 {
            return;
        }
        if let Err(mpsc::TrySendError::Full(_)) = self.sender.try_send(AudioMessage::Frame {
            generation: self.generation.load(Ordering::Acquire),
            queued_at: Instant::now(),
            pcm: samples.to_vec(),
            speed: normalize_playback_speed(speed),
            pitch: normalize_pitch(pitch),
        }) {
            self.producer_dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Mark every queued frame stale immediately, and wake an idle worker.
    /// Unlike a blocking send, this cannot freeze the UI on pause/ROM load.
    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.try_send(AudioMessage::Clear);
    }

    /// A standalone 440 Hz speaker check using the SAME worker, stream and
    /// sink that receive actual game PCM. No ROM, GUI or firmware is needed.
    /// Success means the live worker appended the tone, not that physical
    /// sound was heard. Wait until the tone has time to play before exit.
    pub fn test_speakers() -> Result<()> {
        let output = Self::start()?;
        output.set_volume(1.0);
        anyhow::ensure!(
            output.enqueue_live_test_tone(),
            "speaker-test worker queue is full"
        );
        let started = Instant::now();
        loop {
            if output.test_tones_submitted() > 0 {
                thread::sleep(Duration::from_millis(350));
                return Ok(());
            }
            if output.device_status() == AudioDeviceStatus::OpenFailed {
                anyhow::bail!("live speaker-test worker could not open an OS audio device");
            }
            anyhow::ensure!(
                started.elapsed() < Duration::from_secs(5),
                "live speaker-test worker did not submit PCM within five seconds"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Report callback failures with an atomic flag; the realtime CPAL thread
/// must never try to reopen devices, block, or allocate diagnostics.
fn signal_stream_failure(failed: Arc<AtomicBool>) -> impl FnMut(rodio::cpal::StreamError) + Clone {
    move |_error| {
        failed.store(true, Ordering::Release);
    }
}

/// Keep the mixer sink alive before releasing its backing output stream.
struct PlaybackDevice {
    sink: Sink,
    _stream: OutputStream,
    /// An OS stream may disconnect after it was successfully opened.
    /// Its real-time callback sets this flag; only the audio worker rebuilds.
    stream_failed: Arc<AtomicBool>,
}

impl PlaybackDevice {
    fn open_default() -> Result<Self> {
        // Every attempted OS stream needs its own fault flag. A callback
        // from a rejected fallback candidate must not poison a later healthy
        // stream and trigger an unnecessary immediate second reopening.
        let preferred = OutputStreamBuilder::from_default_device().and_then(|builder| {
            let failed = Arc::new(AtomicBool::new(false));
            builder
                .with_error_callback(signal_stream_failure(Arc::clone(&failed)))
                .open_stream_or_fallback()
                .map(|stream| (stream, failed))
        });
        let opened = match preferred {
            Ok(opened) => Ok(opened),
            Err(default_error) => {
                let fallback = match rodio::cpal::default_host().output_devices() {
                    Ok(mut devices) => devices.find_map(|device| {
                        let failed = Arc::new(AtomicBool::new(false));
                        OutputStreamBuilder::from_device(device)
                            .and_then(|builder| {
                                builder
                                    .with_error_callback(signal_stream_failure(Arc::clone(&failed)))
                                    .open_stream_or_fallback()
                            })
                            .map(|stream| (stream, failed))
                            .ok()
                    }),
                    Err(_) => None,
                };
                fallback.ok_or(default_error)
            }
        };
        let (mut stream, stream_failed) =
            opened.context("could not open a desktop audio output device")?;
        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());
        Ok(Self {
            sink,
            _stream: stream,
            stream_failed,
        })
    }

    fn stream_failed(&self) -> bool {
        self.stream_failed.load(Ordering::Acquire)
    }

    fn enqueue(&self, pcm: Vec<f32>, volume: f32, pitch: f32) {
        if pcm.is_empty() || pcm.len() % usize::from(CHANNELS) != 0 {
            return;
        }
        // Skip old individual buffers without clearing the live sink.
        let excess = self
            .sink
            .len()
            .saturating_sub(MAX_PENDING_FRAMES.saturating_sub(1));
        for _ in 0..excess {
            self.sink.skip_one();
        }
        self.sink.set_volume(volume);
        // Classic emulator tempo/pitch shift: 2x playback produces 2x pitch.
        set_sink_playback_speed(&self.sink, pitch);
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

/// A failed reopen remains pending across multiple OS retry attempts.
#[derive(Default)]
struct StreamRecovery {
    pending: bool,
}

impl StreamRecovery {
    fn on_fault(&mut self, faults: &AtomicU64) {
        self.pending = true;
        faults.fetch_add(1, Ordering::Relaxed);
    }

    fn on_open(&mut self, recoveries: &AtomicU64) {
        if self.pending {
            self.pending = false;
            recoveries.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn on_manual_reset(&mut self) {
        // A manually selected output is not an automatic fault recovery.
        self.pending = false;
    }
}

/// A paused emulator sends no PCM. Nevertheless, an already-open stream
/// can fault while idle; retry only that failed output, with the same cooldown
/// as frame-triggered retries. An unused or manually reset route stays closed.
fn idle_reopen_due(
    recovery: &StreamRecovery,
    device_present: bool,
    retry_after: Option<Instant>,
    now: Instant,
) -> bool {
    recovery.pending && !device_present && retry_ready(retry_after, now)
}

fn attempt_device_open(
    device: &mut Option<PlaybackDevice>,
    device_status: &AtomicU8,
    retry_after: &mut Option<Instant>,
    recovery: &mut StreamRecovery,
    stream_recoveries: &AtomicU64,
) {
    match PlaybackDevice::open_default() {
        Ok(output) => {
            device_status.store(1, Ordering::Release);
            *device = Some(output);
            *retry_after = None;
            recovery.on_open(stream_recoveries);
        }
        Err(error) => {
            device_status.store(2, Ordering::Release);
            warn!("desktop audio unavailable (video remains playable): {error}");
            *retry_after = Some(Instant::now() + AUDIO_RETRY_DELAY);
        }
    }
}

/// Keep mixer-continuity accounting independent of actual audio hardware.
#[derive(Default)]
struct PlaybackContinuity {
    has_submitted_game_pcm: bool,
}

impl PlaybackContinuity {
    fn reset(&mut self) {
        self.has_submitted_game_pcm = false;
    }

    fn observe_game_pcm(&mut self, sink_empty: bool) -> bool {
        let gap = self.has_submitted_game_pcm && sink_empty;
        self.has_submitted_game_pcm = true;
        gap
    }
}

fn audio_worker(
    receiver: Receiver<AudioMessage>,
    generation: Arc<AtomicU64>,
    device_status: Arc<AtomicU8>,
    volume_bits: Arc<AtomicU32>,
    submitted_frames: Arc<AtomicU64>,
    submitted_nonzero_frames: Arc<AtomicU64>,
    test_tones_submitted: Arc<AtomicU64>,
    worker_dropped_frames: Arc<AtomicU64>,
    sink_empty_on_pcm: Arc<AtomicU64>,
    stream_failures: Arc<AtomicU64>,
    stream_recoveries: Arc<AtomicU64>,
) {
    let mut device: Option<PlaybackDevice> = None;
    let mut retry_after: Option<Instant> = None;
    let mut active_generation = generation.load(Ordering::Acquire);
    let mut tone_until: Option<Instant> = None;
    let mut continuity = PlaybackContinuity::default();
    let mut stretch = PitchStretch::default();
    let mut recovery = StreamRecovery::default();

    loop {
        // Device faults must be observed even while the emulator is paused:
        // a plain receiver iterator would block indefinitely without PCM.
        let message = match receiver.recv_timeout(AUDIO_HEALTH_POLL) {
            Ok(message) => Some(message),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        // A previously healthy device can fail after unplug/restart. Reopen
        // here on the dedicated audio worker rather than the real-time CPAL
        // callback or GUI. Failed attempts still use the existing cooldown.
        if device.as_ref().is_some_and(PlaybackDevice::stream_failed) {
            warn!("desktop audio stream failed; reopening an output device");
            recovery.on_fault(&stream_failures);
            device = None;
            device_status.store(2, Ordering::Release);
            retry_after = None;
            tone_until = None;
            continuity.reset();
            stretch.reset();
        }
        let newest_generation = generation.load(Ordering::Acquire);
        if newest_generation != active_generation {
            if let Some(output) = &device {
                output.clear();
            }
            tone_until = None;
            continuity.reset();
            stretch.reset();
            active_generation = newest_generation;
        }

        let Some(message) = message else {
            // A saturated PCM queue can drop a VolumeChanged notification.
            // If the game is now paused, there may be no further PCM frame to
            // apply the new atomic gain. Reconcile the live Rodio sink on an
            // idle health tick without reopening a disabled/unused device.
            if let Some(output) = &device {
                apply_sink_volume(&output.sink, &volume_bits);
            }
            if idle_reopen_due(&recovery, device.is_some(), retry_after, Instant::now()) {
                attempt_device_open(
                    &mut device,
                    &device_status,
                    &mut retry_after,
                    &mut recovery,
                    &stream_recoveries,
                );
            }
            continue;
        };
        let (frame_generation, queued_at, pcm, speed, pitch, is_test_tone) = match message {
            AudioMessage::Frame {
                generation,
                queued_at,
                pcm,
                speed,
                pitch,
            } => (generation, queued_at, pcm, speed, pitch, false),
            AudioMessage::TestTone {
                generation,
                queued_at,
            } => (generation, queued_at, speaker_test_tone(), 1.0, 1.0, true),
            AudioMessage::Clear => {
                continuity.reset();
                stretch.reset();
                continue;
            }
            AudioMessage::VolumeChanged => {
                if let Some(output) = &device {
                    apply_sink_volume(&output.sink, &volume_bits);
                }
                continue;
            }
            AudioMessage::Reopen => {
                // A default-route change does not always trigger CPAL's
                // error callback. Explicit re-selection must not fabricate
                // a stream-fault or successful-recovery receipt.
                recovery.on_manual_reset();
                if let Some(output) = device.take() {
                    output.clear();
                }
                device_status.store(0, Ordering::Release);
                retry_after = None;
                tone_until = None;
                continuity.reset();
                stretch.reset();
                continue;
            }
        };
        let now = Instant::now();
        if frame_generation != active_generation
            || (!is_test_tone && !frame_is_fresh(queued_at, now))
        {
            if !is_test_tone {
                worker_dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
            continue;
        }
        if !is_test_tone && tone_until.is_some_and(|end| now < end) {
            worker_dropped_frames.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if device.is_none()
            && pcm.iter().any(|&sample| sample != 0)
            && (is_test_tone || retry_ready(retry_after, now))
        {
            attempt_device_open(
                &mut device,
                &device_status,
                &mut retry_after,
                &mut recovery,
                &stream_recoveries,
            );
        }
        if let Some(output) = &device {
            // Device initialization can take time. Drop a frame that became
            // invalid while the worker was waiting for system audio.
            let newest_generation = generation.load(Ordering::Acquire);
            if newest_generation != frame_generation {
                output.clear();
                active_generation = newest_generation;
                stretch.reset();
                if !is_test_tone {
                    worker_dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
            } else if is_test_tone || frame_is_fresh(queued_at, Instant::now()) {
                // A manually requested beep is still useful after a slow
                // initial device open; only game PCM has strict freshness.
                let volume = normalize_volume(f32::from_bits(volume_bits.load(Ordering::Acquire)));
                if is_test_tone {
                    // Remove queued gameplay sources and protect this short
                    // check from frame-pruning while it plays. Video and
                    // emulation continue at full speed.
                    output.clear();
                    output.enqueue(pcm16_stereo_to_float(&pcm).unwrap_or_default(), volume, 1.0);
                    continuity.reset();
                    stretch.reset();
                    tone_until = Some(Instant::now() + TEST_TONE_EXCLUSIVE);
                    test_tones_submitted.fetch_add(1, Ordering::Release);
                } else {
                    // Tempo and pitch are independent. WSOLA outputs fewer
                    // frames for faster play, while Rodio applies only the
                    // desired pitch multiplier to that output.
                    let float_pcm = pcm16_stereo_to_float(&pcm).unwrap_or_default();
                    let stretched = stretch.process(&float_pcm, speed / pitch);
                    if !stretched.is_empty() {
                        if continuity.observe_game_pcm(output.sink.empty()) {
                            sink_empty_on_pcm.fetch_add(1, Ordering::Relaxed);
                        }
                        output.enqueue(stretched, volume, pitch);
                        submitted_frames.fetch_add(1, Ordering::Release);
                        if pcm.iter().any(|&sample| sample != 0) {
                            submitted_nonzero_frames.fetch_add(1, Ordering::Release);
                        }
                    }
                }
            } else {
                worker_dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Apply the latest atomic gain to the *existing* mixer, even when game
/// emulation is paused and no new SamplesBuffer can refresh Sink::volume.
fn apply_sink_volume(sink: &Sink, volume_bits: &AtomicU32) {
    sink.set_volume(normalize_volume(f32::from_bits(
        volume_bits.load(Ordering::Acquire),
    )));
}

/// Set tempo and pitch together through Rodio's real sink control.
fn set_sink_playback_speed(sink: &Sink, speed: f32) {
    sink.set_speed(normalize_playback_speed(speed));
}

/// Bound the independent pitch, preserving 1.0 as the default.
fn normalize_pitch(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.5, 4.0)
    } else {
        1.0
    }
}

/// Keep audio pitch/speed finite and within the supported UI's 0.5-4x range.
fn normalize_playback_speed(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.5, 4.0)
    } else {
        1.0
    }
}

/// Keep untrusted persisted volumes within a usable, non-clipping range.
fn normalize_volume(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
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
        let sample =
            (std::f32::consts::TAU * TONE_HZ * time).sin() * amplitude * 0.15 * f32::from(i16::MAX);
        let sample = sample as i16;
        output.extend_from_slice(&[sample, sample]);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
            mpsc::sync_channel,
        },
        time::{Duration, Instant},
    };

    use super::{
        AUDIO_RETRY_DELAY, AudioDeviceStatus, AudioMessage, AudioOutput, PlaybackContinuity,
        StreamRecovery, apply_sink_volume, frame_is_fresh, idle_reopen_due, normalize_pitch,
        normalize_playback_speed, normalize_volume, pcm16_stereo_to_float, retry_ready,
        set_sink_playback_speed, speaker_test_tone,
    };

    #[test]
    fn device_open_status_is_distinct_from_worker_start_and_audio_emission() {
        let output = AudioOutput::start().unwrap();
        assert_eq!(output.device_status(), AudioDeviceStatus::NotAttempted);
        assert_eq!(output.submitted_frames(), 0);
        assert_eq!(output.submitted_nonzero_frames(), 0);
        assert_eq!(output.test_tones_submitted(), 0);
        assert_eq!(output.device_status().label(), "not_attempted");
        // Only the real playback thread can set these in production; avoid
        // requiring a physical speaker in this deterministic unit test.
        output.device_status.store(2, Ordering::Release);
        assert_eq!(output.device_status(), AudioDeviceStatus::OpenFailed);
        assert_eq!(output.device_status().label(), "open_failed");
        output.device_status.store(1, Ordering::Release);
        assert_eq!(output.device_status(), AudioDeviceStatus::Opened);
        assert_eq!(output.device_status().label(), "opened");
    }

    #[test]
    fn volume_changes_are_nonblocking_and_clamped_without_a_device() {
        let output = AudioOutput::start().unwrap();
        output.set_volume(0.25);
        assert_eq!(
            f32::from_bits(output.volume_bits.load(Ordering::Acquire)),
            0.25
        );
        output.set_volume(2.5);
        assert_eq!(
            f32::from_bits(output.volume_bits.load(Ordering::Acquire)),
            1.0
        );
        output.set_volume(f32::NAN);
        assert_eq!(
            f32::from_bits(output.volume_bits.load(Ordering::Acquire)),
            0.0
        );
        assert_eq!(normalize_volume(-1.0), 0.0);
        assert_eq!(normalize_volume(0.75), 0.75);
    }

    #[test]
    fn volume_change_wakes_idle_playback_without_opening_a_device() {
        let (sender, receiver) = sync_channel(4);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(0)),
            device_status: Arc::new(AtomicU8::new(0)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        output.set_volume(0.25);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            AudioMessage::VolumeChanged
        ));
        // No redundant wakeups when UI repeatedly applies the same volume.
        output.set_volume(0.25);
        assert!(receiver.try_recv().is_err());
        output.set_volume(0.0);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            AudioMessage::VolumeChanged
        ));
        assert_eq!(
            f32::from_bits(output.volume_bits.load(Ordering::Acquire)),
            0.0
        );
        assert_eq!(output.device_status(), AudioDeviceStatus::NotAttempted);
    }

    #[test]
    fn manual_audio_route_reopen_is_queued_nonblocking() {
        let (sender, receiver) = sync_channel(1);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(0)),
            device_status: Arc::new(AtomicU8::new(1)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        assert!(output.request_reopen());
        assert!(
            !output.request_reopen(),
            "full channel must refuse without blocking"
        );
        assert!(matches!(receiver.try_recv().unwrap(), AudioMessage::Reopen));
        assert_eq!(output.stream_failures(), 0);
        assert_eq!(output.stream_recoveries(), 0);
    }

    #[test]
    fn worker_reopen_clears_stale_route_status_without_fault_credit() {
        let output = AudioOutput::start().unwrap();
        // The actual worker loop can reset route selection without opening
        // an ALSA/PipeWire device or fabricating a stream failure.
        output.device_status.store(1, Ordering::Release);
        assert!(output.request_reopen());
        let deadline = Instant::now() + Duration::from_secs(2);
        while output.device_status() != AudioDeviceStatus::NotAttempted && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(output.device_status(), AudioDeviceStatus::NotAttempted);
        assert_eq!(output.stream_failures(), 0);
        assert_eq!(output.stream_recoveries(), 0);
    }

    #[test]
    fn full_queue_does_not_block_new_mute_level() {
        let (sender, receiver) = sync_channel(1);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(0)),
            device_status: Arc::new(AtomicU8::new(0)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        output.enqueue(&[1, -1], 1.0, 1.0);
        output.set_volume(0.0);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            AudioMessage::Frame { .. }
        ));
        assert_eq!(
            f32::from_bits(output.volume_bits.load(Ordering::Acquire)),
            0.0
        );
    }

    #[test]
    fn saturated_control_queue_can_still_reconcile_idle_sink_volume() {
        let (sender, receiver) = sync_channel(1);
        let (sink, _queue) = rodio::Sink::new();
        let volume = AtomicU32::new(1.0_f32.to_bits());
        // A pending audio/control message fills the nonblocking channel,
        // so the volume notification cannot be submitted by the GUI.
        sender.try_send(AudioMessage::Clear).unwrap();
        volume.store(0.0_f32.to_bits(), Ordering::Release);
        assert!(sender.try_send(AudioMessage::VolumeChanged).is_err());
        assert!(matches!(receiver.try_recv().unwrap(), AudioMessage::Clear));
        // The actual worker's idle health tick must read this shared atomic,
        // not rely exclusively on a VolumeChanged or new PCM message.
        apply_sink_volume(&sink, &volume);
        assert_eq!(sink.volume(), 0.0);
        volume.store(0.65_f32.to_bits(), Ordering::Release);
        apply_sink_volume(&sink, &volume);
        assert_eq!(sink.volume(), 0.65);
    }

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
        output.enqueue(&[0, 0], 1.0, 1.0);
        output.clear();
        let after = output.generation.load(Ordering::Acquire);
        assert_eq!(after, before + 1);
        output.enqueue(&[0, 0], 1.0, 1.0);
    }

    #[test]
    fn test_tone_is_queued_on_game_worker_without_counting_as_game_pcm() {
        // Intercept the bounded queue instead of spawning a worker that might
        // contact ALSA/PipeWire; CI never needs physical audio hardware.
        let (sender, receiver) = sync_channel(2);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(3)),
            device_status: Arc::new(AtomicU8::new(0)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        assert!(output.enqueue_live_test_tone());
        match receiver.try_recv().unwrap() {
            AudioMessage::TestTone { generation, .. } => assert_eq!(generation, 3),
            _ => panic!("the dedicated tone must travel through the live worker"),
        }
        assert_eq!(output.submitted_frames(), 0);
        assert_eq!(output.submitted_nonzero_frames(), 0);
        assert_eq!(output.test_tones_submitted(), 0);
    }

    #[test]
    fn producer_reports_full_queue_without_blocking_or_touching_speakers() {
        let (sender, _receiver) = sync_channel(1);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(0)),
            device_status: Arc::new(AtomicU8::new(0)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        output.enqueue(&[100, -100], 1.0, 1.0);
        output.enqueue(&[100, -100], 1.0, 1.0);
        assert_eq!(output.producer_dropped_frames(), 1);
        assert_eq!(output.worker_dropped_frames(), 0);
        assert_eq!(output.submitted_frames(), 0);
        output.enqueue(&[1], 1.0, 1.0); // invalid PCM never enters the queue
        assert_eq!(output.producer_dropped_frames(), 1);
    }

    #[test]
    fn idle_rodio_sink_applies_live_volume_without_new_pcm() {
        let (sink, _queue) = rodio::Sink::new();
        let volume = AtomicU32::new(1.0_f32.to_bits());
        apply_sink_volume(&sink, &volume);
        assert_eq!(sink.volume(), 1.0);
        volume.store(0.35_f32.to_bits(), Ordering::Release);
        apply_sink_volume(&sink, &volume);
        assert_eq!(sink.volume(), 0.35);
        volume.store(0.0_f32.to_bits(), Ordering::Release);
        apply_sink_volume(&sink, &volume);
        assert_eq!(sink.volume(), 0.0);
    }

    #[test]
    fn offline_rodio_sink_tracks_speed_and_returns_to_normal_pitch() {
        // Rodio's unconnected sink exposes the real playback-speed control
        // without opening ALSA, PipeWire or the system audio device.
        let (sink, _queue) = rodio::Sink::new();
        assert_eq!(sink.speed(), 1.0);
        set_sink_playback_speed(&sink, 3.0);
        assert_eq!(sink.speed(), 3.0);
        set_sink_playback_speed(&sink, 0.5);
        assert_eq!(sink.speed(), 0.5);
        set_sink_playback_speed(&sink, 1.0);
        assert_eq!(sink.speed(), 1.0);
    }

    #[test]
    fn game_audio_queue_carries_per_frame_pitch_speed_without_audio_hardware() {
        let (sender, receiver) = sync_channel(8);
        let output = AudioOutput {
            sender,
            generation: Arc::new(AtomicU64::new(0)),
            device_status: Arc::new(AtomicU8::new(0)),
            volume_bits: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            submitted_frames: Arc::new(AtomicU64::new(0)),
            submitted_nonzero_frames: Arc::new(AtomicU64::new(0)),
            test_tones_submitted: Arc::new(AtomicU64::new(0)),
            producer_dropped_frames: Arc::new(AtomicU64::new(0)),
            worker_dropped_frames: Arc::new(AtomicU64::new(0)),
            sink_empty_on_pcm: Arc::new(AtomicU64::new(0)),
            stream_failures: Arc::new(AtomicU64::new(0)),
            stream_recoveries: Arc::new(AtomicU64::new(0)),
        };
        for speed in [0.5, 1.0, 2.0, 3.0, 4.0] {
            output.enqueue(&[123, -123], speed, 1.0);
        }
        for speed in [0.5, 1.0, 2.0, 3.0, 4.0] {
            match receiver.try_recv().unwrap() {
                AudioMessage::Frame {
                    pcm,
                    speed: received,
                    pitch,
                    ..
                } => {
                    assert_eq!(pcm, vec![123, -123]);
                    assert_eq!(received, speed);
                    assert_eq!(pitch, 1.0);
                }
                _ => panic!("expected PCM frame with speed metadata"),
            }
        }
        assert_eq!(normalize_playback_speed(f32::NAN), 1.0);
        assert_eq!(normalize_playback_speed(0.0), 0.5);
        assert_eq!(normalize_playback_speed(12.0), 4.0);
        assert_eq!(normalize_playback_speed(f32::INFINITY), 1.0);
        assert_eq!(normalize_pitch(2.5), 2.5);
        assert_eq!(normalize_pitch(10.0), 4.0);
        assert_eq!(normalize_pitch(0.1), 0.5);
        assert_eq!(normalize_pitch(f32::NAN), 1.0);
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
    fn mixer_starvation_accounting_excludes_first_frame_and_intentional_clears() {
        let mut continuity = PlaybackContinuity::default();
        // The sink is necessarily empty before the first buffer.
        assert!(!continuity.observe_game_pcm(true));
        assert!(!continuity.observe_game_pcm(false));
        // An empty sink on the next frame indicates a probable audio gap.
        assert!(continuity.observe_game_pcm(true));
        // A pause, new game, save restore or speaker tone deliberately clears
        // audio; the next empty sink is not a gameplay underrun.
        continuity.reset();
        assert!(!continuity.observe_game_pcm(true));
        assert!(continuity.observe_game_pcm(true));
    }

    #[test]
    fn stream_failure_callback_signals_recovery_without_a_sound_device() {
        let failed = Arc::new(AtomicBool::new(false));
        let selected = Arc::new(AtomicBool::new(false));
        let mut failed_error = super::signal_stream_failure(Arc::clone(&failed));
        let mut selected_error = super::signal_stream_failure(Arc::clone(&selected));
        assert!(!failed.load(Ordering::Acquire));
        assert!(!selected.load(Ordering::Acquire));

        // Failed fallback attempts have distinct callbacks. An earlier
        // candidate's fault cannot falsely mark the selected stream dead.
        failed_error(rodio::cpal::StreamError::DeviceNotAvailable);
        assert!(failed.load(Ordering::Acquire));
        assert!(!selected.load(Ordering::Acquire));

        selected_error(rodio::cpal::StreamError::DeviceNotAvailable);
        assert!(selected.load(Ordering::Acquire));
    }

    #[test]
    fn stream_recovery_receipts_ignore_initial_open_and_failed_retries() {
        let faults = AtomicU64::new(0);
        let recoveries = AtomicU64::new(0);
        let mut tracking = StreamRecovery::default();

        tracking.on_open(&recoveries);
        assert_eq!(recoveries.load(Ordering::Relaxed), 0);

        tracking.on_fault(&faults);
        assert_eq!(faults.load(Ordering::Relaxed), 1);
        assert_eq!(recoveries.load(Ordering::Relaxed), 0);
        // Several failed OS reopening attempts do not fabricate recovery.
        assert!(tracking.pending);
        assert_eq!(recoveries.load(Ordering::Relaxed), 0);

        tracking.on_open(&recoveries);
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);
        assert!(!tracking.pending);
        tracking.on_open(&recoveries);
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);

        tracking.on_fault(&faults);
        tracking.on_open(&recoveries);
        assert_eq!(faults.load(Ordering::Relaxed), 2);
        assert_eq!(recoveries.load(Ordering::Relaxed), 2);

        // A fault followed by deliberate user output re-selection must not
        // inflate the count of automatic fault recoveries.
        tracking.on_fault(&faults);
        tracking.on_manual_reset();
        tracking.on_open(&recoveries);
        assert_eq!(faults.load(Ordering::Relaxed), 3);
        assert_eq!(recoveries.load(Ordering::Relaxed), 2);
        assert!(!tracking.pending);
    }

    #[test]
    fn faulted_output_retries_while_idle_but_unused_or_manual_routes_do_not() {
        let now = Instant::now();
        let faults = AtomicU64::new(0);
        let recoveries = AtomicU64::new(0);
        let mut tracking = StreamRecovery::default();
        // Merely starting a paused game must not initialize audio hardware.
        assert!(!idle_reopen_due(&tracking, false, None, now));
        // Nor should an explicit manual reset fabricate a fault retry.
        assert!(!idle_reopen_due(&tracking, false, None, now));
        tracking.on_fault(&faults);
        assert!(idle_reopen_due(&tracking, false, None, now));
        assert!(!idle_reopen_due(&tracking, true, None, now));
        let retry_at = now + AUDIO_RETRY_DELAY;
        assert!(!idle_reopen_due(&tracking, false, Some(retry_at), now));
        assert!(idle_reopen_due(&tracking, false, Some(retry_at), retry_at));
        tracking.on_open(&recoveries);
        assert!(!idle_reopen_due(&tracking, false, None, retry_at));
        assert_eq!(faults.load(Ordering::Relaxed), 1);
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);
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
