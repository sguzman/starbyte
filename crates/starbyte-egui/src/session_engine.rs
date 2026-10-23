//! Bridge from the window-independent clock to a single-owned SNES session.
//!
//! The GUI must command this worker rather than borrowing FrontendSession
//! directly. No egui or Wayland surface is required to execute a frame.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

use starbyte_core::{input::ControllerState, manifest::AssetConfig};
use starbyte_frontend::{
    CheatEntry, FrontendSession, SessionControlsSnapshot, SessionInputDiagnostics,
    SessionScreenshot, SessionSnapshot, SessionSoundDiagnostics, SoundSessionMode,
};

use crate::{
    audio::{AudioDeviceStatus, AudioOutput},
    scheduler::{EmulatorScheduler, NTSC_FRAME_INTERVAL},
};

const FRAME_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Controller bridge updated directly by the GUI. Held state replaces its
/// predecessor; short press edges are OR-latched until consumed by one SNES
/// frame. An independent heartbeat releases controls if Wayland stops the
/// UI repaint/input callback while a keyboard button is held.
#[derive(Clone)]
pub struct InputPort {
    state: Arc<InputPortState>,
}

struct InputPortState {
    held_bits: AtomicU16,
    pending_press_bits: AtomicU16,
    started: Instant,
    last_heartbeat_ms: AtomicU64,
}

impl Default for InputPort {
    fn default() -> Self {
        Self {
            state: Arc::new(InputPortState {
                held_bits: AtomicU16::new(0),
                pending_press_bits: AtomicU16::new(0),
                started: Instant::now(),
                last_heartbeat_ms: AtomicU64::new(0),
            }),
        }
    }
}

impl InputPort {
    /// A visibility change may stop egui callbacks while the last keyboard
    /// state still says "held". Avoid moving forever through a background
    /// game because an input release was never delivered.
    const STALE_AFTER_MS: u64 = 300;

    /// Submit the current controller state and *new* press edges separately.
    /// Call each visible UI update, even when no buttons are pressed.
    pub fn submit(&self, held: ControllerState, just_pressed: ControllerState) {
        let now_ms = self.state.started.elapsed().as_millis() as u64;
        let previous = self
            .state
            .last_heartbeat_ms
            .swap(now_ms, Ordering::AcqRel);
        if now_ms.saturating_sub(previous) > Self::STALE_AFTER_MS {
            // A previous press that was never consumed while Wayland hid
            // the window must not be replayed on the first fresh update.
            self.state.pending_press_bits.store(0, Ordering::Release);
        }
        self.state.held_bits.store(held.to_bits(), Ordering::Release);
        self.state
            .pending_press_bits
            .fetch_or(just_pressed.to_bits(), Ordering::AcqRel);
    }

    /// Called once by the worker immediately before emulating a frame.
    fn take_frame_state(&self) -> ControllerState {
        self.take_frame_state_at(self.state.started.elapsed().as_millis() as u64)
    }

    fn take_frame_state_at(&self, now_ms: u64) -> ControllerState {
        let recorded = self.state.last_heartbeat_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(recorded) > Self::STALE_AFTER_MS {
            // Do not clear the atomics from this thread: a fresh GUI submit
            // could otherwise race with clear() and lose a real keypress.
            // The next submit will discard the stale edges before updating.
            return ControllerState::default();
        }
        let held = self.state.held_bits.load(Ordering::Acquire);
        let edges = self.state.pending_press_bits.swap(0, Ordering::AcqRel);
        controller_from_bits(held | edges)
    }

    /// Flush held buttons and pending press edges on focus loss or ROM
    /// replacement. Background gameplay must not latch stale foreground keys.
    pub fn clear(&self) {
        self.state.held_bits.store(0, Ordering::Release);
        self.state.pending_press_bits.store(0, Ordering::Release);
    }
}

/// Whether the GUI is currently repainting. A hidden Wayland window can
/// stop receiving redraw callbacks without reporting "minimized"; suppress
/// needless RGBA copies after a short missed-present heartbeat, while the
/// independent emulation clock and audio continue uninterrupted.
#[derive(Clone)]
pub struct RenderPort {
    state: Arc<RenderState>,
}

struct RenderState {
    visible: AtomicBool,
    started: Instant,
    last_ui_heartbeat_ms: AtomicU64,
}

impl Default for RenderPort {
    fn default() -> Self {
        Self {
            state: Arc::new(RenderState {
                visible: AtomicBool::new(false),
                started: Instant::now(),
                last_ui_heartbeat_ms: AtomicU64::new(0),
            }),
        }
    }
}

impl RenderPort {
    const STALE_AFTER_MS: u64 = 300;

    /// Call once per actual GUI update. Stopping those updates automatically
    /// disables video copying, without touching SNES time or PCM production.
    pub fn set_visible(&self, visible: bool) {
        self.state.visible.store(visible, Ordering::Release);
        self.state.last_ui_heartbeat_ms.store(
            self.state.started.elapsed().as_millis() as u64,
            Ordering::Release,
        );
    }

    fn should_copy_framebuffer(&self) -> bool {
        self.should_copy_at(self.state.started.elapsed().as_millis() as u64)
    }

    fn should_copy_at(&self, now_ms: u64) -> bool {
        let last = self.state.last_ui_heartbeat_ms.load(Ordering::Acquire);
        self.state.visible.load(Ordering::Acquire)
            && now_ms.saturating_sub(last) <= Self::STALE_AFTER_MS
    }
}

pub(crate) fn controller_from_bits(bits: u16) -> ControllerState {
    ControllerState {
        b: bits & 0x8000 != 0,
        y: bits & 0x4000 != 0,
        select: bits & 0x2000 != 0,
        start: bits & 0x1000 != 0,
        up: bits & 0x0800 != 0,
        down: bits & 0x0400 != 0,
        left: bits & 0x0200 != 0,
        right: bits & 0x0100 != 0,
        a: bits & 0x0080 != 0,
        x: bits & 0x0040 != 0,
        l: bits & 0x0020 != 0,
        r: bits & 0x0010 != 0,
    }
}

/// Hardware-mixer diagnostic measured by the audio worker. Generated PCM
/// is not proof of audible playback; a nonzero submitted frame count confirms
/// actual delivery to Rodio's sink, not the physical speaker.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkerAudioDiagnostics {
    pub device_status: Option<AudioDeviceStatus>,
    pub submitted_frames: u64,
    pub submitted_nonzero_frames: u64,
    pub producer_dropped_frames: u64,
    pub worker_dropped_frames: u64,
    pub sink_empty_on_pcm: u64,
}

/// A snapshot for the renderer; video frames coalesce when the UI is hidden.
pub struct PublishedGameFrame {
    pub status: SessionSnapshot,
    pub rgba: Option<Vec<u8>>,
    pub last_frame_cpu: Duration,
    pub stereo_pairs: usize,
    pub nonzero_pairs: usize,
    pub audio_diagnostics: WorkerAudioDiagnostics,
    pub input_diagnostics: SessionInputDiagnostics,
}

/// Exclusive, thread-owned game state. All save mutations stay serialized
/// with emulated frames, including when a compositor hides the entire window.
pub struct OwnedSession {
    session: FrontendSession,
    audio: Option<AudioOutput>,
    sound_enabled: bool,
    muted: bool,
    volume: f32,
    speed_percent: u16,
    pitch_factor: f32,
    input: InputPort,
    render: RenderPort,
    last_sram_flush: Instant,
    next_audio_retry: Instant,
}

impl OwnedSession {
    pub fn create(assets: AssetConfig) -> anyhow::Result<Self> {
        Ok(Self::from_session(FrontendSession::new(assets)?))
    }

    pub fn from_session(session: FrontendSession) -> Self {
        Self {
            session,
            audio: None,
            sound_enabled: true,
            muted: false,
            volume: 0.15,
            speed_percent: 100,
            pitch_factor: 1.0,
            input: InputPort::default(),
            render: RenderPort::default(),
            last_sram_flush: Instant::now(),
            next_audio_retry: Instant::now(),
        }
    }

    pub fn spawn(self) -> io::Result<EmulatorScheduler<Self, PublishedGameFrame>> {
        EmulatorScheduler::spawn_with_pause_hook(
            self,
            NTSC_FRAME_INTERVAL,
            Self::advance,
            Self::clear_game_audio,
        )
    }

    fn clear_game_audio(&mut self) {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        // A long manual/background pause may prevent the normal 30-second
        // frame-driven SRAM flush. Commit cartridge progress at the pause
        // transition, without depending on any Wayland repaint callback.
        if let Err(error) = self.flush_sram() {
            tracing::warn!("could not persist SRAM while pausing worker: {error}");
        }
        self.last_sram_flush = Instant::now();
    }

    /// Transfer engine ownership while retaining the lock-free input port.
    pub fn spawn_with_input(
        self,
    ) -> io::Result<(EmulatorScheduler<Self, PublishedGameFrame>, InputPort)> {
        let input = self.input.clone();
        Ok((self.spawn()?, input))
    }

    /// Keep both lock-free ports outside the thread. The GUI may update
    /// visibility even when Wayland stops sending egui repaint callbacks.
    pub fn spawn_with_ports(
        self,
    ) -> io::Result<(
        EmulatorScheduler<Self, PublishedGameFrame>,
        InputPort,
        RenderPort,
    )> {
        let input = self.input.clone();
        let render = self.render.clone();
        Ok((self.spawn()?, input, render))
    }

    pub fn set_render_visible(&self, visible: bool) {
        self.render.set_visible(visible);
    }

    /// Held-controller updates routed through typed commands cannot infer
    /// genuine press edges. Prefer the direct InputPort for gameplay, where
    /// the UI supplies held buttons and newly pressed buttons separately.
    pub fn set_input(&mut self, input: ControllerState) {
        self.input.submit(input, ControllerState::default());
    }

    pub fn set_audio(&mut self, enabled: bool, muted: bool, volume: f32) {
        self.sound_enabled = enabled;
        self.muted = muted;
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(output) = &self.audio {
            output.set_volume(if enabled && !muted { self.volume } else { 0.0 });
            if !enabled || muted || self.volume <= 0.0 {
                // Do not keep PCM from before a mute/zero-volume interval:
                // raising the volume later must play new audio, not backlog.
                output.clear();
            }
        }
    }

    /// Explicitly request the native Rodio speaker check from the same
    /// worker that consumes game PCM. A muted or disabled game must never
    /// produce an unexpected test tone.
    pub fn speaker_check(&mut self) -> Result<(), String> {
        if !self.sound_enabled || self.muted || self.volume <= 0.0 {
            return Err("Enable and unmute sound before testing speakers".to_owned());
        }
        if self.audio.is_none() {
            self.audio = Some(AudioOutput::start().map_err(|error| error.to_string())?);
        }
        let audio = self.audio.as_ref().expect("audio initialized above");
        audio.set_volume(self.volume);
        if audio.enqueue_live_test_tone() {
            Ok(())
        } else {
            Err("Audio worker queue is full; speaker check was not submitted".to_owned())
        }
    }

    pub fn set_speed(&mut self, speed_percent: u16, pitch_factor: f32) {
        self.speed_percent = speed_percent.clamp(1, 1_600);
        self.pitch_factor = pitch_factor.clamp(0.25, 4.0);
        if let Some(output) = &self.audio {
            output.clear();
        }
    }

    pub fn load_rom(
        &mut self,
        path: &std::path::Path,
        mode: SoundSessionMode,
    ) -> anyhow::Result<()> {
        if let Some(output) = &self.audio {
            output.clear();
        }
        self.input.clear();
        self.last_sram_flush = Instant::now();
        match mode {
            SoundSessionMode::Compatibility => self.session.load_rom(path),
            SoundSessionMode::Isolated => self.session.load_rom_with_isolated_audio_probe(path),
            SoundSessionMode::Persistent => self.session.load_rom_with_persistent_audio(path),
        }
    }

    pub fn set_cheats(&mut self, cheats: &[CheatEntry]) -> usize {
        self.session.set_active_cheats(cheats)
    }

    pub fn quick_save(&mut self) -> anyhow::Result<()> {
        self.session.quick_save()
    }

    pub fn quick_load(&mut self) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.session.quick_load()
    }

    pub fn save_slot(&mut self, slot: u8) -> anyhow::Result<std::path::PathBuf> {
        self.session.save_state_slot(slot)
    }

    pub fn load_slot(&mut self, slot: u8) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.session.load_state_slot(slot)
    }

    pub fn flush_sram(&mut self) -> anyhow::Result<()> {
        self.session.flush_save_ram()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.session.snapshot()
    }
    pub fn audio_diagnostics(&self) -> WorkerAudioDiagnostics {
        let Some(output) = self.audio.as_ref() else {
            return WorkerAudioDiagnostics::default();
        };
        WorkerAudioDiagnostics {
            device_status: Some(output.device_status()),
            submitted_frames: output.submitted_frames(),
            submitted_nonzero_frames: output.submitted_nonzero_frames(),
            producer_dropped_frames: output.producer_dropped_frames(),
            worker_dropped_frames: output.worker_dropped_frames(),
            sink_empty_on_pcm: output.sink_empty_on_pcm(),
        }
    }

    /// Standalone screenshot capture uses the shared frontend API rather
    /// than introducing a second direct emulator framebuffer access path.
    pub fn capture_screenshot(&self) -> Result<SessionScreenshot, String> {
        self.session
            .capture_screenshot()
            .ok_or_else(|| "Load a ROM before taking a screenshot".to_owned())
    }

    /// Snapshot the current emulated framebuffer without advancing a frame.
    /// Needed after quick/disk state loads while the user is paused: the UI
    /// must display the restored picture immediately, not wait for Play.
    pub fn capture_frame(&self) -> PublishedGameFrame {
        let status = self.session.snapshot();
        let samples = &self.session.frame_audio_samples();
        let stereo_pairs = samples.len() / 2;
        let nonzero_pairs = samples
            .chunks_exact(2)
            .filter(|pair| pair[0] != 0 || pair[1] != 0)
            .count();
        PublishedGameFrame {
            status,
            rgba: Some(self.session.framebuffer_rgba().to_vec()),
            last_frame_cpu: Duration::ZERO,
            stereo_pairs,
            nonzero_pairs,
            audio_diagnostics: self.audio_diagnostics(),
            input_diagnostics: self.session.input_diagnostics(),
        }
    }

    /// Produce one game frame entirely on the scheduler thread. The Rodio
    /// queue is already bounded and independent of egui. Never enqueue PCM
    /// for the renderer to replay later after a hidden-window interval.
    pub fn advance(&mut self) -> Result<PublishedGameFrame, String> {
        let started = Instant::now();
        self.session.set_controller1(self.input.take_frame_state());
        self.session
            .run_frame_with_timeout(FRAME_TIMEOUT)
            .map_err(|error| error.to_string())?;

        // SRAM writes live with the emulator owner, not the egui renderer:
        // active background games keep their 30-second autosave cadence.
        if self.last_sram_flush.elapsed() >= Duration::from_secs(30) {
            self.last_sram_flush = Instant::now();
            if let Err(error) = self.session.flush_save_ram() {
                tracing::warn!("background SRAM autosave failed: {error}");
            }
        }

        let status = self.session.snapshot();
        let samples = &self.session.frame_audio_samples();
        let stereo_pairs = samples.len() / 2;
        let nonzero_pairs = samples.chunks_exact(2)
            .filter(|pair| pair[0] != 0 || pair[1] != 0)
            .count();

        if self.sound_enabled && !self.muted && self.volume > 0.0 {
            if self.audio.is_none()
                && nonzero_pairs > 0
                && Instant::now() >= self.next_audio_retry
            {
                match AudioOutput::start() {
                    Ok(output) => self.audio = Some(output),
                    Err(error) => {
                        // Missing/failed output devices must not trigger a
                        // fresh audio-thread spawn and warning at 60 Hz.
                        self.next_audio_retry = Instant::now() + Duration::from_secs(5);
                        tracing::warn!(
                            "emulation worker could not start audio (retry in 5s): {error}"
                        );
                    }
                }
            }
            if let Some(audio) = &self.audio {
                audio.set_volume(self.volume);
                audio.enqueue(
                    samples,
                    f32::from(self.speed_percent) / 100.0,
                    self.pitch_factor,
                );
            }
        }

        Ok(PublishedGameFrame {
            status,
            rgba: self.render.should_copy_framebuffer()
                .then(|| self.session.framebuffer_rgba().to_vec()),
            last_frame_cpu: started.elapsed(),
            stereo_pairs,
            nonzero_pairs,
            audio_diagnostics: self.audio_diagnostics(),
            input_diagnostics: self.session.input_diagnostics(),
        })
    }
}

impl Drop for OwnedSession {
    fn drop(&mut self) {
        if let Err(error) = self.flush_sram() {
            tracing::warn!("could not flush worker-owned SRAM on shutdown: {error}");
        }
        if let Some(audio) = &self.audio {
            audio.clear();
        }
    }
}

/// Typed API for the GUI handoff. Instead of locking the live emulator,
/// the GUI sends gamepad/audio state and explicit save commands to the owner.
impl EmulatorScheduler<OwnedSession, PublishedGameFrame> {
    pub fn set_gamepad_state(&self, state: ControllerState) -> Result<(), String> {
        self.post(move |engine| engine.set_input(state))
    }

    pub fn configure_sound(
        &self,
        enabled: bool,
        muted: bool,
        volume: f32,
    ) -> Result<(), String> {
        self.post(move |engine| engine.set_audio(enabled, muted, volume))
    }

    pub fn queue_speaker_check(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| engine.speaker_check())
    }

    pub fn configure_playback_speed(
        &self,
        speed_percent: u16,
        pitch_factor: f32,
    ) -> Result<(), String> {
        self.set_speed_percent(speed_percent);
        self.post(move |engine| engine.set_speed(speed_percent, pitch_factor))
    }

    pub fn load_game(
        &self,
        path: std::path::PathBuf,
        mode: SoundSessionMode,
    ) -> Result<(), String> {
        self.transact(move |engine| engine.load_rom(&path, mode).map_err(|e| e.to_string()))
    }
    /// A new cartridge is loaded by the exclusive emulator owner; ROM I/O
    /// and SRAM flushes must never block the renderer. The acknowledgment
    /// includes a still frame to replace the previous game's picture even
    /// when manual Pause is active.
    pub fn queue_load_game_with_frame(
        &self,
        path: std::path::PathBuf,
        mode: SoundSessionMode,
    ) -> Result<Receiver<Result<PublishedGameFrame, String>>, String> {
        self.submit_transaction(move |engine| {
            engine.load_rom(&path, mode).map_err(|error| error.to_string())?;
            Ok(engine.capture_frame())
        })
    }

    pub fn update_cheats(&self, cheats: Vec<CheatEntry>) -> Result<usize, String> {
        self.transact(move |engine| Ok(engine.set_cheats(&cheats)))
    }

    pub fn quick_save_game(&self) -> Result<(), String> {
        self.transact(|engine| engine.quick_save().map_err(|e| e.to_string()))
    }
    /// The GUI must never synchronously wait for a save that could be
    /// delayed by a slow emulation frame or disk flush.
    pub fn queue_quick_save(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| engine.quick_save().map_err(|e| e.to_string()))
    }

    pub fn queue_save_slot(
        &self,
        slot: u8,
    ) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(move |engine| {
            engine.save_slot(slot).map(|_| ()).map_err(|e| e.to_string())
        })
    }

    pub fn quick_load_game(&self) -> Result<(), String> {
        self.transact(|engine| engine.quick_load().map_err(|e| e.to_string()))
    }

    /// Restore memory state and return a fresh still image in one serialized
    /// transaction, even when the scheduler is deliberately paused.
    pub fn quick_load_with_frame(&self) -> Result<PublishedGameFrame, String> {
        self.transact(|engine| {
            engine.quick_load().map_err(|e| e.to_string())?;
            Ok(engine.capture_frame())
        })
    }
    pub fn queue_quick_load_with_frame(
        &self,
    ) -> Result<Receiver<Result<PublishedGameFrame, String>>, String> {
        self.submit_transaction(|engine| {
            engine.quick_load().map_err(|e| e.to_string())?;
            Ok(engine.capture_frame())
        })
    }

    pub fn save_game_slot(&self, slot: u8) -> Result<std::path::PathBuf, String> {
        self.transact(move |engine| engine.save_slot(slot).map_err(|e| e.to_string()))
    }

    pub fn load_game_slot(&self, slot: u8) -> Result<(), String> {
        self.transact(move |engine| engine.load_slot(slot).map_err(|e| e.to_string()))
    }

    pub fn load_game_slot_with_frame(
        &self,
        slot: u8,
    ) -> Result<PublishedGameFrame, String> {
        self.transact(move |engine| {
            engine.load_slot(slot).map_err(|e| e.to_string())?;
            Ok(engine.capture_frame())
        })
    }
    pub fn queue_load_slot_with_frame(
        &self,
        slot: u8,
    ) -> Result<Receiver<Result<PublishedGameFrame, String>>, String> {
        self.submit_transaction(move |engine| {
            engine.load_slot(slot).map_err(|e| e.to_string())?;
            Ok(engine.capture_frame())
        })
    }

    pub fn persist_sram(&self) -> Result<(), String> {
        self.transact(|engine| engine.flush_sram().map_err(|e| e.to_string()))
    }

    /// Capture the owning thread's framebuffer at a serialized point in
    /// game time. The caller writes/encodes PNG asynchronously, and the
    /// returned type is the same one used by the established Starbyte UI.
    pub fn queue_screenshot_pixels(
        &self,
    ) -> Result<Receiver<Result<SessionScreenshot, String>>, String> {
        self.submit_transaction(|engine| engine.capture_screenshot())
    }

    pub fn read_controls(&self) -> Result<SessionControlsSnapshot, String> {
        self.transact(|engine| Ok(engine.session.controls_snapshot()))
    }
    /// An on-demand diagnostic query, not a 60 Hz per-frame string
    /// allocation. Menu inspectors can use the same summary as the
    /// established player without borrowing emulator-private APU state.
    pub fn read_sound_diagnostics(&self) -> Result<SessionSoundDiagnostics, String> {
        self.transact(|engine| Ok(engine.session.sound_diagnostics()))
    }

    pub fn read_status(&self) -> Result<SessionSnapshot, String> {
        self.transact(|engine| Ok(engine.snapshot()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_button_edges_survive_a_quick_release() {
        let port = InputPort::default();
        let pressed = ControllerState {
            start: true,
            ..ControllerState::default()
        };
        port.submit(pressed, pressed);
        port.submit(ControllerState::default(), ControllerState::default());
        assert!(port.take_frame_state().start);
        assert!(!port.take_frame_state().start);
    }

    #[test]
    fn video_copying_can_be_suppressed_without_stopping_clock() {
        let port = RenderPort::default();
        assert!(!port.should_copy_framebuffer());
        port.set_visible(true);
        assert!(port.should_copy_framebuffer());
        let last = port.state.last_ui_heartbeat_ms.load(Ordering::Acquire);
        assert!(!port.should_copy_at(last + RenderPort::STALE_AFTER_MS + 1));
        port.set_visible(true);
        assert!(port.should_copy_framebuffer());
        port.set_visible(false);
        assert!(!port.should_copy_framebuffer());
    }

    #[test]
    fn losing_focus_flushes_latched_buttons() {
        let port = InputPort::default();
        let pressed = ControllerState {
            a: true,
            ..ControllerState::default()
        };
        port.submit(pressed, pressed);
        port.clear();
        assert_eq!(port.take_frame_state(), ControllerState::default());
    }

    #[test]
    fn stale_window_input_does_not_hold_buttons_forever() {
        let port = InputPort::default();
        let pressed = ControllerState {
            right: true,
            ..ControllerState::default()
        };
        port.submit(pressed, pressed);
        let since = port.state.last_heartbeat_ms.load(Ordering::Acquire);
        assert_eq!(
            port.take_frame_state_at(since + InputPort::STALE_AFTER_MS + 1),
            ControllerState::default()
        );
        assert_eq!(port.take_frame_state_at(since + InputPort::STALE_AFTER_MS + 2),
            ControllerState::default());
    }

    #[test]
    fn controller_bits_round_trip() {
        let all = ControllerState {
            b: true, y: true, select: true, start: true,
            up: true, down: true, left: true, right: true,
            a: true, x: true, l: true, r: true,
        };
        assert_eq!(controller_from_bits(all.to_bits()), all);
    }

    #[test]
    fn background_worker_runs_demo_and_captures_screenshot_without_window() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("built-in-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut engine = OwnedSession::create(assets).unwrap();
        engine.load_rom(&rom, SoundSessionMode::Persistent).unwrap();
        engine.set_audio(false, true, 0.0);
        let (worker, _input, _render) = engine.spawn_with_ports().unwrap();
        worker.set_pause_when_unfocused(false);
        worker.set_focused(false);
        worker.set_playing(true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while worker.completed_frames() < 5 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(15));
        }
        assert!(
            worker.completed_frames() >= 5,
            "independent clock did not produce background frames"
        );
        worker.set_playing(false);
        worker.invoke(|_| Ok(())).unwrap();
        let controls = worker.read_controls().unwrap();
        assert!(controls.session.has_rom);
        assert!(!controls.has_quick_save);
        let reply = worker.queue_screenshot_pixels().unwrap();
        let image = reply.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        assert!(image.frame >= 5);
        assert_eq!(
            image.rgba.len(),
            image.width as usize * image.height as usize * 4
        );
        worker.quick_save_game().unwrap();
        assert!(worker.read_controls().unwrap().has_quick_save);
        assert_eq!(worker.shutdown().unwrap().snapshot().rom_path, Some(rom));
    }

    #[test]
    fn detached_session_has_no_game_until_loaded() {
        let engine = OwnedSession::create(AssetConfig::default()).unwrap();
        assert!(!engine.snapshot().has_rom);
    }

    #[test]
    fn sound_and_speed_preferences_are_clamped() {
        let mut engine = OwnedSession::create(AssetConfig::default()).unwrap();
        engine.set_audio(true, false, 5.0);
        engine.set_speed(10_000, 40.0);
        assert_eq!(engine.volume, 1.0);
        assert_eq!(engine.speed_percent, 1_600);
        assert_eq!(engine.pitch_factor, 4.0);
    }
}
