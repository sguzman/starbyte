//! Bridge from the window-independent clock to a single-owned SNES session.
//!
//! The GUI must command this worker rather than borrowing FrontendSession
//! directly. No egui or Wayland surface is required to execute a frame.

use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

use starbyte_core::{input::ControllerState, manifest::AssetConfig};
use starbyte_frontend::{
    CheatEntry, FrontendSession, SessionControlsSnapshot, SessionInputDiagnostics,
    SessionLaunchRequest, SessionSaveAction, SessionSaveOutcome, SessionScreenshot,
    SessionSnapshot, SessionSoundDiagnostics, SoundSessionMode,
};

use crate::{
    audio::{AudioDeviceStatus, AudioOutput, AudioToneError},
    blackout::BlackScreenMonitor,
    scheduler::{EmulatorScheduler, NTSC_FRAME_INTERVAL},
};

const FRAME_TIMEOUT: Duration = Duration::from_millis(1_500);
const SRAM_AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);
const SRAM_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(5);

/// A failed write must remain pending for a prompt retry. Only successful
/// writes (including no-op flushes for unchanged SRAM) earn a full interval.
fn next_sram_flush_deadline(now: Instant, write_succeeded: bool) -> Instant {
    now + if write_succeeded {
        SRAM_AUTOSAVE_INTERVAL
    } else {
        SRAM_FAILURE_RETRY_DELAY
    }
}

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
        let previous = self.state.last_heartbeat_ms.swap(now_ms, Ordering::AcqRel);
        if now_ms.saturating_sub(previous) > Self::STALE_AFTER_MS {
            // A previous press that was never consumed while Wayland hid
            // the window must not be replayed on the first fresh update.
            self.state.pending_press_bits.store(0, Ordering::Release);
        }
        self.state
            .held_bits
            .store(held.to_bits(), Ordering::Release);
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
    pub test_tones_submitted: u64,
    pub expired_test_tones: u64,
    pub stream_failures: u64,
    pub stream_recoveries: u64,
}

/// Read-only latest menu facts. The worker updates this only on explicit
/// state mutations; UI redraws never touch the emulator or disk save slots.
#[derive(Clone)]
pub struct ControlsPort {
    state: Arc<Mutex<SessionControlsSnapshot>>,
}

impl ControlsPort {
    fn new(initial: SessionControlsSnapshot) -> Self {
        Self {
            state: Arc::new(Mutex::new(initial)),
        }
    }

    fn publish(&self, next: SessionControlsSnapshot) {
        *self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = next;
    }

    pub fn latest(&self) -> SessionControlsSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

/// A snapshot for the renderer; video frames coalesce when the UI is hidden.
pub struct PublishedGameFrame {
    /// Monotonic timestamp when the owned frame was ready for presentation.
    /// The GUI can measure delivery delay without conflating it with SNES CPU time.
    pub ready_at: Instant,
    pub status: SessionSnapshot,
    pub rgba: Option<Vec<u8>>,
    pub last_frame_cpu: Duration,
    pub stereo_pairs: usize,
    pub nonzero_pairs: usize,
    pub audio_diagnostics: WorkerAudioDiagnostics,
    pub input_diagnostics: SessionInputDiagnostics,
}

/// A serialized save acknowledgement with an optional restored still image.
/// Normal saves avoid copying a framebuffer; quick/disk loads return the new
/// picture so a manually paused game can update without another SNES frame.
pub struct WorkerSaveResponse {
    pub outcome: SessionSaveOutcome,
    pub controls: SessionControlsSnapshot,
    pub restored_frame: Option<PublishedGameFrame>,
}

/// A cartridge launch acknowledgement with the authoritative new cartridge's
/// menu state and still video. A failed launch must preserve the previous UI
/// state and cannot fabricate an acknowledgement.
pub struct WorkerLaunchResponse {
    pub controls: SessionControlsSnapshot,
    pub frame: PublishedGameFrame,
}

/// Exclusive, thread-owned game state. All save mutations stay serialized
/// with emulated frames, including when a compositor hides the entire window.
pub struct OwnedSession {
    session: FrontendSession,
    blackout_monitor: BlackScreenMonitor,
    audio: Option<AudioOutput>,
    sound_enabled: bool,
    muted: bool,
    volume: f32,
    speed_percent: u16,
    pitch_factor: f32,
    input: InputPort,
    render: RenderPort,
    controls_port: ControlsPort,
    next_sram_flush: Instant,
    sram_retry_pending: bool,
    next_audio_retry: Instant,
    /// True only after the shutdown hook has finished on the emulation
    /// thread. Returning the owner to the GUI must not flush SRAM again.
    shutdown_finalized: bool,
}

impl OwnedSession {
    pub fn create(assets: AssetConfig) -> anyhow::Result<Self> {
        Ok(Self::from_session(FrontendSession::new(assets)?))
    }

    pub fn from_session(session: FrontendSession) -> Self {
        let controls_port = ControlsPort::new(session.controls_snapshot());
        Self {
            session,
            blackout_monitor: BlackScreenMonitor::default(),
            audio: None,
            sound_enabled: true,
            muted: false,
            volume: 0.15,
            speed_percent: 100,
            pitch_factor: 1.0,
            input: InputPort::default(),
            render: RenderPort::default(),
            controls_port,
            next_sram_flush: Instant::now() + SRAM_AUTOSAVE_INTERVAL,
            sram_retry_pending: false,
            next_audio_retry: Instant::now(),
            shutdown_finalized: false,
        }
    }

    pub fn spawn(mut self) -> io::Result<EmulatorScheduler<Self, PublishedGameFrame>> {
        // An owner returned by shutdown may later be launched again. A
        // previous finalization must not suppress its new save lifecycle.
        self.shutdown_finalized = false;
        EmulatorScheduler::spawn_with_idle_hooks(
            self,
            NTSC_FRAME_INTERVAL,
            Self::advance,
            Self::clear_game_audio,
            Self::retry_failed_sram_while_paused,
            Self::finalize_on_worker,
        )
    }

    /// Read only after the owning scheduler has shut down and joined.
    /// This does not claim an external audio device was physically audible.
    pub fn finalization_state(&self) -> (bool, bool) {
        (self.shutdown_finalized, self.audio.is_none())
    }

    fn finalize_on_worker(&mut self) {
        self.clear_game_audio();
        // Drop the Rodio output on the owning thread too; joining an audio
        // worker must never happen while the GUI is drawing or closing.
        self.audio.take();
        self.shutdown_finalized = true;
    }

    fn record_sram_flush_result(&mut self, succeeded: bool) {
        self.sram_retry_pending = !succeeded;
        self.next_sram_flush = next_sram_flush_deadline(Instant::now(), succeeded);
    }

    fn clear_game_audio(&mut self) {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        // Persist when background/manual pause begins. An unsuccessful write
        // retains a five-second retry even if the window never repaints again.
        let result = self.flush_sram();
        self.record_sram_flush_result(result.is_ok());
        if let Err(error) = result {
            tracing::warn!("could not persist SRAM while pausing worker: {error}");
        }
    }

    fn retry_failed_sram_while_paused(&mut self) {
        if !self.sram_retry_pending || Instant::now() < self.next_sram_flush {
            return;
        }
        // The scheduler calls this only from its idle owner loop. Do not
        // retry on a GUI thread and do not reset the 30-second success timer.
        let result = self.flush_sram();
        self.record_sram_flush_result(result.is_ok());
        if let Err(error) = result {
            tracing::warn!("failed to retry paused worker SRAM persistence: {error}");
        }
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

    /// Clone before handing the session to the worker. The receiver is
    /// valid while the owner is running, including with no GUI redraws.
    pub fn controls_port(&self) -> ControlsPort {
        self.controls_port.clone()
    }

    fn publish_controls(&self) -> SessionControlsSnapshot {
        let controls = self.session.controls_snapshot();
        self.controls_port.publish(controls.clone());
        controls
    }

    /// Held-controller updates routed through typed commands cannot infer
    /// genuine press edges. Prefer the direct InputPort for gameplay, where
    /// the UI supplies held buttons and newly pressed buttons separately.
    pub fn set_input(&mut self, input: ControllerState) {
        self.input.submit(input, ControllerState::default());
    }

    pub fn set_audio(&mut self, enabled: bool, muted: bool, volume: f32) {
        let was_audible = self.sound_enabled && !self.muted && self.volume > 0.0;
        self.sound_enabled = enabled;
        self.muted = muted;
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let audible = self.sound_enabled && !self.muted && self.volume > 0.0;
        if let Some(output) = &self.audio {
            // Volume-only adjustments must reach the live mixer without
            // waiting for a new gameplay buffer or egui repaint.
            output.set_volume(if audible { self.volume } else { 0.0 });
            if was_audible && !audible {
                // A transition to silence must discard queued old PCM so
                // unmuting never replays sound from a prior game moment.
                // Do not repeatedly clear on an already-muted update.
                output.clear();
            }
        }
    }

    /// A CPAL device can be silently left on an obsolete route after a
    /// default-output change. Reset only the live audio output, without
    /// touching the emulator, SRAM, or current cartridge.
    pub fn request_audio_reopen(&mut self) -> Result<(), String> {
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| "No game-audio output has been started yet".to_owned())?;
        if audio.request_reopen() {
            return Ok(());
        }

        // A stopped playback receiver cannot process the route selection.
        // Recreate its software worker on the SNES owner thread, even if the
        // game is manually paused and no further PCM will arrive. Starting
        // the worker is lazy: this does not open an OS speaker device until
        // fresh nonzero game PCM or an explicit speaker check is submitted.
        self.audio.take();
        self.next_audio_retry = Instant::now() + Duration::from_secs(5);
        let replacement = AudioOutput::start()
            .map_err(|error| format!("Could not restart audio worker: {error}"))?;
        let audible = self.sound_enabled && !self.muted && self.volume > 0.0;
        replacement.set_volume(if audible { self.volume } else { 0.0 });
        self.audio = Some(replacement);
        Ok(())
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
        match audio.enqueue_live_test_tone() {
            Ok(()) => Ok(()),
            Err(AudioToneError::QueueFull) => {
                Err("Audio worker queue is full; speaker check was not submitted".to_owned())
            }
            Err(AudioToneError::WorkerStopped) => {
                Err("Audio worker stopped; reselect desktop audio output and retry".to_owned())
            }
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
        let result = match mode {
            SoundSessionMode::Compatibility => self.session.load_rom(path),
            SoundSessionMode::Isolated => self.session.load_rom_with_isolated_audio_probe(path),
            SoundSessionMode::Persistent => self.session.load_rom_with_persistent_audio(path),
        };
        if result.is_ok() {
            self.blackout_monitor = BlackScreenMonitor::default();
            self.next_sram_flush = Instant::now() + SRAM_AUTOSAVE_INTERVAL;
            self.sram_retry_pending = false;
            self.publish_controls();
        }
        result
    }

    /// One serialized launch with its matching cheats and save namespace.
    /// No game frame can execute between ROM installation and cheat setup.
    pub fn launch_game(&mut self, request: &SessionLaunchRequest) -> anyhow::Result<()> {
        self.input.clear();
        if let Some(output) = &self.audio {
            output.clear();
        }
        self.session.launch_game(request)?;
        self.blackout_monitor = BlackScreenMonitor::default();
        self.next_sram_flush = Instant::now() + SRAM_AUTOSAVE_INTERVAL;
        self.sram_retry_pending = false;
        self.publish_controls();
        Ok(())
    }

    pub fn set_cheats(&mut self, cheats: &[CheatEntry]) -> usize {
        let applied = self.session.set_active_cheats(cheats);
        self.publish_controls();
        applied
    }

    fn process_launch(
        &mut self,
        request: &SessionLaunchRequest,
    ) -> Result<WorkerLaunchResponse, String> {
        self.launch_game(request)
            .map_err(|error| error.to_string())?;
        Ok(WorkerLaunchResponse {
            controls: self.controls_port.latest(),
            frame: self.capture_frame(),
        })
    }

    fn process_save_action(
        &mut self,
        action: SessionSaveAction,
        capture_restored_frame: bool,
    ) -> Result<WorkerSaveResponse, String> {
        let outcome = self
            .session
            .apply_save_action(action)
            .map_err(|error| error.to_string())?;
        let restoring = matches!(
            action,
            SessionSaveAction::QuickLoad | SessionSaveAction::LoadSlot(_)
        );
        if restoring {
            // A restored APU belongs to a different timeline. Clear the
            // old PCM before acknowledging, not after the UI repaints.
            if let Some(audio) = &self.audio {
                audio.clear();
            }
        }
        Ok(WorkerSaveResponse {
            outcome,
            controls: self.publish_controls(),
            restored_frame: if restoring && capture_restored_frame {
                Some(self.capture_frame())
            } else {
                None
            },
        })
    }

    pub fn quick_save(&mut self) -> anyhow::Result<()> {
        let result = self
            .session
            .apply_save_action(SessionSaveAction::QuickSave)
            .map(|_| ());
        if result.is_ok() {
            self.publish_controls();
        }
        result
    }

    pub fn quick_load(&mut self) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        let result = self
            .session
            .apply_save_action(SessionSaveAction::QuickLoad)
            .map(|_| ());
        if result.is_ok() {
            self.publish_controls();
        }
        result
    }

    pub fn save_slot(&mut self, slot: u8) -> anyhow::Result<std::path::PathBuf> {
        let outcome = self
            .session
            .apply_save_action(SessionSaveAction::SaveSlot(slot))?;
        self.publish_controls();
        match outcome {
            SessionSaveOutcome::SavedTo(path) => Ok(path),
            SessionSaveOutcome::Completed => {
                anyhow::bail!("disk save returned no slot path")
            }
        }
    }

    pub fn load_slot(&mut self, slot: u8) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        let result = self
            .session
            .apply_save_action(SessionSaveAction::LoadSlot(slot))
            .map(|_| ());
        if result.is_ok() {
            self.publish_controls();
        }
        result
    }

    pub fn flush_sram(&mut self) -> anyhow::Result<()> {
        self.session
            .apply_save_action(SessionSaveAction::FlushSram)
            .map(|_| ())
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.session.snapshot()
    }

    pub fn controls_snapshot(&self) -> SessionControlsSnapshot {
        // This explicit owner-side query stays live for diagnostics. The
        // separately published port intentionally freezes menu facts until
        // a state-changing command, avoiding 60 Hz save-slot disk checks.
        self.session.controls_snapshot()
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
            test_tones_submitted: output.test_tones_submitted(),
            expired_test_tones: output.expired_test_tones(),
            stream_failures: output.stream_failures(),
            stream_recoveries: output.stream_recoveries(),
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
        let metrics = self.session.audio_frame_metrics();
        let stereo_pairs = metrics.stereo_pairs;
        let nonzero_pairs = metrics.nonzero_pairs;
        PublishedGameFrame {
            status,
            rgba: Some(self.session.framebuffer_rgba().to_vec()),
            last_frame_cpu: Duration::ZERO,
            stereo_pairs,
            nonzero_pairs,
            audio_diagnostics: self.audio_diagnostics(),
            input_diagnostics: self.session.input_diagnostics(),
            ready_at: Instant::now(),
        }
    }

    /// Advance once while manually paused. Keep the DSP emulation active,
    /// but never queue this diagnostic frame's PCM for speaker playback.
    /// Return full still video even if the window was temporarily hidden.
    pub fn step_frame(&mut self) -> Result<PublishedGameFrame, String> {
        let audio_enabled = self.sound_enabled;
        self.sound_enabled = false;
        let stepped = self.advance();
        self.sound_enabled = audio_enabled;
        let mut frame = stepped?;
        if frame.rgba.is_none() {
            frame.rgba = Some(self.session.framebuffer_rgba().to_vec());
            frame.ready_at = Instant::now();
        }
        Ok(frame)
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
        if Instant::now() >= self.next_sram_flush {
            let result = self.session.flush_save_ram();
            self.record_sram_flush_result(result.is_ok());
            if let Err(error) = result {
                tracing::warn!("background SRAM autosave failed: {error}");
            }
        }

        let status = self.session.snapshot();
        let samples = self.session.frame_audio_samples();
        let metrics = self.session.audio_frame_metrics();
        let stereo_pairs = metrics.stereo_pairs;
        let nonzero_pairs = metrics.nonzero_pairs;
        // Diagnostic CPU/PPU inspection belongs on the same thread as the
        // emulator. It must not require a UI-thread borrow or depend on
        // a Wayland frame callback while a game runs off-workspace.
        self.blackout_monitor
            .observe(&self.session, stereo_pairs, nonzero_pairs);

        if self.sound_enabled && !self.muted && self.volume > 0.0 {
            if self.audio.is_none() && nonzero_pairs > 0 && Instant::now() >= self.next_audio_retry
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
            let stopped = if let Some(audio) = &self.audio {
                audio.set_volume(self.volume);
                !audio.enqueue(
                    samples,
                    f32::from(self.speed_percent) / 100.0,
                    self.pitch_factor,
                )
            } else {
                false
            };
            if stopped {
                // A panicked or exited audio thread drops its receiver. A
                // live sender cannot deliver PCM to it, even if the last OS
                // device status said Opened. Recreate only on the independent
                // emulator clock, with a cooldown to avoid 60 spawns/second
                // if the host sound service remains unhealthy.
                self.audio.take();
                self.next_audio_retry = Instant::now() + Duration::from_secs(5);
                tracing::warn!("background audio worker stopped; retrying output in 5 seconds");
            }
        }

        Ok(PublishedGameFrame {
            status,
            rgba: self
                .render
                .should_copy_framebuffer()
                .then(|| self.session.framebuffer_rgba().to_vec()),
            last_frame_cpu: started.elapsed(),
            stereo_pairs,
            nonzero_pairs,
            audio_diagnostics: self.audio_diagnostics(),
            input_diagnostics: self.session.input_diagnostics(),
            ready_at: Instant::now(),
        })
    }
}

impl Drop for OwnedSession {
    fn drop(&mut self) {
        // A normal scheduler shutdown has already flushed SRAM and disposed
        // of the audio output on the emulation thread. This fallback is only
        // for an unspawned owner or one dropped without successful shutdown.
        if !self.shutdown_finalized {
            if let Err(error) = self.flush_sram() {
                tracing::warn!("could not flush worker-owned SRAM on shutdown: {error}");
            }
            if let Some(audio) = &self.audio {
                audio.clear();
            }
        }
    }
}

/// Typed API for the GUI handoff. Instead of locking the live emulator,
/// the GUI sends gamepad/audio state and explicit save commands to the owner.
impl EmulatorScheduler<OwnedSession, PublishedGameFrame> {
    pub fn set_gamepad_state(&self, state: ControllerState) -> Result<(), String> {
        self.post(move |engine| engine.set_input(state))
    }

    pub fn configure_sound(&self, enabled: bool, muted: bool, volume: f32) -> Result<(), String> {
        // User mute/unmute must not fail because a separate save/cheat job
        // queue is saturated; keep the command ordered with the emulator.
        self.update_live_control(move |engine| engine.set_audio(enabled, muted, volume))
    }

    pub fn queue_speaker_check(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| engine.speaker_check())
    }

    /// Route re-selection runs asynchronously on the exclusive SNES owner
    /// and then on the separate audio thread. Receipt means queued only;
    /// it does not claim that OS opening or speaker audibility succeeded.
    pub fn queue_audio_reopen(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| engine.request_audio_reopen())
    }

    pub fn configure_playback_speed(
        &self,
        speed_percent: u16,
        pitch_factor: f32,
    ) -> Result<(), String> {
        self.set_speed_with_machine_update(speed_percent, move |engine| {
            engine.set_speed(speed_percent, pitch_factor);
        })
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
        self.queue_launch_with_frame(SessionLaunchRequest {
            rom_path: path,
            sound_mode: mode,
            cheats: Vec::new(),
        })
    }

    pub fn queue_launch_with_frame(
        &self,
        request: SessionLaunchRequest,
    ) -> Result<Receiver<Result<PublishedGameFrame, String>>, String> {
        self.submit_transaction(move |engine| {
            engine.process_launch(&request).map(|reply| reply.frame)
        })
    }

    /// The complete library can update ROM identity, save-slot availability,
    /// sound mode and still video from one serialized completion. The UI does
    /// not need to block the worker for follow-up menu reads.
    pub fn queue_launch_with_state(
        &self,
        request: SessionLaunchRequest,
    ) -> Result<Receiver<Result<WorkerLaunchResponse, String>>, String> {
        self.submit_transaction(move |engine| engine.process_launch(&request))
    }

    pub fn update_cheats(&self, cheats: Vec<CheatEntry>) -> Result<usize, String> {
        self.transact(move |engine| Ok(engine.set_cheats(&cheats)))
    }
    /// The full library's cheat toggle must not wait for the next SNES frame
    /// on the UI thread. Apply all patches atomically between frames and
    /// acknowledge the number of active runtime patches.
    pub fn queue_update_cheats(
        &self,
        cheats: Vec<CheatEntry>,
    ) -> Result<Receiver<Result<usize, String>>, String> {
        self.submit_transaction(move |engine| Ok(engine.set_cheats(&cheats)))
    }

    pub fn queue_clear_cheats(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| {
            engine.session.clear_active_cheats();
            engine.publish_controls();
            Ok(())
        })
    }

    /// The same save-command surface used by the existing StarbyteApp,
    /// serialized against worker frames and acknowledged asynchronously.
    /// A UI that does not display restored video can use the smaller receipt.
    pub fn queue_save_action(
        &self,
        action: SessionSaveAction,
    ) -> Result<Receiver<Result<SessionSaveOutcome, String>>, String> {
        self.submit_transaction(move |engine| {
            engine
                .process_save_action(action, false)
                .map(|reply| reply.outcome)
        })
    }

    /// A single completion path for quick/durable saves and restores.
    /// Restores include the framebuffer captured *after* the save state is
    /// applied, even when the worker clock is manually paused. This lets
    /// the full library GUI eventually use one asynchronous command API.
    pub fn queue_save_action_with_frame(
        &self,
        action: SessionSaveAction,
    ) -> Result<Receiver<Result<WorkerSaveResponse, String>>, String> {
        self.submit_transaction(move |engine| engine.process_save_action(action, true))
    }

    pub fn quick_save_game(&self) -> Result<(), String> {
        self.transact(|engine| engine.quick_save().map_err(|e| e.to_string()))
    }
    /// The GUI must never synchronously wait for a save that could be
    /// delayed by a slow emulation frame or disk flush.
    pub fn queue_quick_save(&self) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(|engine| engine.quick_save().map_err(|e| e.to_string()))
    }

    pub fn queue_save_slot(&self, slot: u8) -> Result<Receiver<Result<(), String>>, String> {
        self.submit_transaction(move |engine| {
            engine
                .save_slot(slot)
                .map(|_| ())
                .map_err(|e| e.to_string())
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

    pub fn load_game_slot_with_frame(&self, slot: u8) -> Result<PublishedGameFrame, String> {
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

    /// Explicit frame stepping is serialized on the worker even when normal
    /// playback is stopped. The GUI can await the resulting still frame.
    pub fn queue_step_frame(&self) -> Result<Receiver<Result<PublishedGameFrame, String>>, String> {
        self.submit_transaction(|engine| engine.step_frame())
    }

    pub fn read_controls(&self) -> Result<SessionControlsSnapshot, String> {
        self.transact(|engine| Ok(engine.session.controls_snapshot()))
    }
    /// Query the owner's SPC700 state without blocking a GUI repaint.
    /// The sound panel requests this at a bounded cadence only while open.
    pub fn queue_sound_diagnostics(
        &self,
    ) -> Result<Receiver<Result<SessionSoundDiagnostics, String>>, String> {
        self.submit_transaction(|engine| Ok(engine.session.sound_diagnostics()))
    }

    /// Read the SPC state and live audio-worker counters together on the
    /// owner thread. A paused game publishes no new video frames, so the Sound
    /// panel must not rely on the last frame's mixer diagnostics.
    pub fn queue_sound_panel_diagnostics(
        &self,
    ) -> Result<Receiver<Result<(SessionSoundDiagnostics, WorkerAudioDiagnostics), String>>, String>
    {
        self.submit_transaction(|engine| {
            Ok((
                engine.session.sound_diagnostics(),
                engine.audio_diagnostics(),
            ))
        })
    }

    /// Paused gameplay produces no new video frames, but output devices and
    /// speaker-test counters can still change on the independent audio thread.
    /// Keep that observation asynchronous and separate from game rendering.
    pub fn queue_audio_diagnostics(
        &self,
    ) -> Result<Receiver<Result<WorkerAudioDiagnostics, String>>, String> {
        self.submit_transaction(|engine| Ok(engine.audio_diagnostics()))
    }

    /// Synchronous on-demand query for non-GUI diagnostics/tests; never use
    /// this method from an egui redraw callback.
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
    fn failed_sram_persistence_retries_soon_without_delaying_successful_saves() {
        let now = Instant::now();
        assert_eq!(
            next_sram_flush_deadline(now, true),
            now + SRAM_AUTOSAVE_INTERVAL
        );
        assert_eq!(
            next_sram_flush_deadline(now, false),
            now + SRAM_FAILURE_RETRY_DELAY
        );
        assert!(SRAM_FAILURE_RETRY_DELAY < SRAM_AUTOSAVE_INTERVAL);
    }

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
        assert_eq!(
            port.take_frame_state_at(since + InputPort::STALE_AFTER_MS + 2),
            ControllerState::default()
        );
    }

    #[test]
    fn controller_bits_round_trip() {
        let all = ControllerState {
            b: true,
            y: true,
            select: true,
            start: true,
            up: true,
            down: true,
            left: true,
            right: true,
            a: true,
            x: true,
            l: true,
            r: true,
        };
        assert_eq!(controller_from_bits(all.to_bits()), all);
    }

    #[test]
    fn worker_preserves_pitch_speed_mute_and_volume_across_sound_modes() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("audio-policy-worker.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        // All command-path assertions run with audio muted so CI does not
        // require, or imply success opening, an actual speaker device.
        owner.set_audio(true, true, 0.35);
        let (worker, _, _) = owner.spawn_with_ports().unwrap();
        for mode in [
            SoundSessionMode::Compatibility,
            SoundSessionMode::Persistent,
            SoundSessionMode::Isolated,
        ] {
            let loaded = worker
                .queue_launch_with_state(SessionLaunchRequest {
                    rom_path: rom.clone(),
                    sound_mode: mode,
                    cheats: Vec::new(),
                })
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            assert_eq!(loaded.controls.sound_mode, mode);
            for (speed, pitch) in [
                (100_u16, 1.0_f32),
                (100, 1.25),
                (150, 1.5),
                (160, 1.0),
                (75, 0.75),
            ] {
                // Includes a custom-pitch-only change at fixed 100% speed
                // and pitch-preserving speed changes, not just fast-forward.
                worker.configure_playback_speed(speed, pitch).unwrap();
                worker.configure_sound(true, true, 0.35).unwrap();
                let actual = worker
                    .transact(|engine| {
                        Ok((
                            engine.speed_percent,
                            engine.pitch_factor,
                            engine.sound_enabled,
                            engine.muted,
                            engine.volume,
                            engine.session.controls_snapshot().sound_mode,
                        ))
                    })
                    .unwrap();
                assert_eq!(actual.0, speed);
                assert!((actual.1 - pitch).abs() < f32::EPSILON);
                assert!(actual.2);
                assert!(actual.3);
                assert!((actual.4 - 0.35).abs() < f32::EPSILON);
                assert_eq!(actual.5, mode);
            }
        }
        worker.shutdown().unwrap();
    }

    #[test]
    fn worker_sound_diagnostics_are_queried_asynchronously_after_mode_changes() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("worker-sound-diagnostics.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        owner.set_audio(false, true, 0.0);
        let (worker, _, _) = owner.spawn_with_ports().unwrap();
        for (mode, expected_driver) in [
            (SoundSessionMode::Persistent, true),
            (SoundSessionMode::Compatibility, false),
        ] {
            worker
                .queue_launch_with_state(SessionLaunchRequest {
                    rom_path: rom.clone(),
                    sound_mode: mode,
                    cheats: Vec::new(),
                })
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let diagnostics = worker
                .queue_sound_diagnostics()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap();
            assert_eq!(diagnostics.isolated_driver_enabled, expected_driver);
        }
        worker.shutdown().unwrap();
    }

    #[test]
    fn worker_input_port_reaches_real_snes_auto_joypad_and_releases() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("worker-joypad.sfc");
        let mut rom = starbyte_core::testing::demo::demo_rom_bytes();
        // Enable SNES automatic joypad polling, then run a tight loop.
        rom[..7].copy_from_slice(&[0xA9, 0x01, 0x8D, 0x00, 0x42, 0x80, 0xFE]);
        std::fs::write(&rom_path, rom).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        owner.set_audio(false, true, 0.0);
        owner
            .load_rom(&rom_path, SoundSessionMode::Persistent)
            .unwrap();
        let (worker, input, _) = owner.spawn_with_ports().unwrap();
        let pressed = ControllerState {
            start: true,
            right: true,
            ..ControllerState::default()
        };
        input.submit(pressed, pressed);
        let mut last = None;
        for _ in 0..3 {
            last = Some(
                worker
                    .queue_step_frame()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap(),
            );
        }
        let latched = last.unwrap().input_diagnostics;
        assert_eq!(latched.host_bits, pressed.to_bits());
        assert_eq!(latched.latched_bits, pressed.to_bits());
        assert!(!latched.reading);

        // The same independent port must release the actual emulated
        // register state, not only the presenter-side held-key overlay.
        input.submit(ControllerState::default(), ControllerState::default());
        let mut released = None;
        for _ in 0..3 {
            released = Some(
                worker
                    .queue_step_frame()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap(),
            );
        }
        let released = released.unwrap().input_diagnostics;
        assert_eq!(released.host_bits, 0);
        assert_eq!(released.latched_bits, 0);
        assert!(!released.reading);
        worker.shutdown().unwrap();
    }

    #[test]
    fn failed_background_sram_write_retries_after_disk_path_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("sram-retry.sfc");
        let mut rom = starbyte_core::testing::demo::demo_rom_bytes();
        rom[0x7FD8] = 0x01;
        std::fs::write(&rom_path, rom).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("battery")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        owner.set_audio(false, true, 0.0);
        owner
            .load_rom(&rom_path, SoundSessionMode::Persistent)
            .unwrap();
        owner.set_cheats(&[CheatEntry {
            id: "retry-sram".to_owned(),
            game_id: "sram-retry".to_owned(),
            name: "Set battery byte".to_owned(),
            code: "700000:5A".to_owned(),
            source: "test".to_owned(),
            kind: "Action Replay".to_owned(),
            enabled: true,
        }]);
        owner.step_frame().unwrap();
        owner.session.clear_active_cheats();
        assert_eq!(owner.session.host_read_u8(0x700000), 0x5A);
        let path = owner.session.save_ram_path().unwrap();

        // A directory at the intended .srm path forces atomic persist() to
        // fail without permission assumptions (works under Windows CI too).
        std::fs::create_dir_all(&path).unwrap();
        owner.next_sram_flush = Instant::now();
        owner.advance().unwrap();
        let failed_deadline = owner.next_sram_flush;
        assert!(failed_deadline > Instant::now());
        assert!(failed_deadline <= Instant::now() + SRAM_FAILURE_RETRY_DELAY);
        assert!(path.is_dir());

        std::fs::remove_dir(&path).unwrap();
        // Advance the retry timer rather than sleeping five seconds.
        owner.next_sram_flush = Instant::now();
        owner.advance().unwrap();
        assert_eq!(std::fs::read(&path).unwrap()[0], 0x5A);
        assert!(
            owner.next_sram_flush >= Instant::now() + SRAM_FAILURE_RETRY_DELAY,
            "successful write should restore the normal longer autosave cadence"
        );
    }

    #[test]
    fn failed_paused_sram_write_recovers_without_emulating_any_new_frames() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("paused-retry.sfc");
        let mut rom = starbyte_core::testing::demo::demo_rom_bytes();
        rom[0x7FD8] = 0x01;
        std::fs::write(&rom_path, rom).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("battery")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        owner.set_audio(false, true, 0.0);
        owner
            .load_rom(&rom_path, SoundSessionMode::Persistent)
            .unwrap();
        owner.set_cheats(&[CheatEntry {
            id: "paused-retry".to_owned(),
            game_id: "paused-retry".to_owned(),
            name: "Set battery byte".to_owned(),
            code: "700000:5A".to_owned(),
            source: "test".to_owned(),
            kind: "Action Replay".to_owned(),
            enabled: true,
        }]);
        owner.step_frame().unwrap();
        owner.session.clear_active_cheats();
        let path = owner.session.save_ram_path().unwrap();
        std::fs::create_dir_all(&path).unwrap();

        owner.clear_game_audio();
        assert!(owner.sram_retry_pending);
        assert!(path.is_dir());
        std::fs::remove_dir(&path).unwrap();
        // A clock still paused after focus loss has no advance() calls.
        // Simulate expiry and use exactly the scheduler's idle callback.
        owner.next_sram_flush = Instant::now();
        owner.retry_failed_sram_while_paused();
        assert_eq!(std::fs::read(&path).unwrap()[0], 0x5A);
        assert!(!owner.sram_retry_pending);
        assert!(owner.next_sram_flush > Instant::now() + SRAM_FAILURE_RETRY_DELAY);
    }

    #[test]
    fn worker_shutdown_persists_battery_sram_without_gui_flush() {
        let dir = tempfile::tempdir().unwrap();
        let rom_path = dir.path().join("battery-worker.sfc");
        // The built-in demo normally has no SRAM. Give the synthetic LoROM
        // a 2 KiB battery region without bundling any commercial ROM.
        let mut rom = starbyte_core::testing::demo::demo_rom_bytes();
        rom[0x7FD8] = 0x01;
        std::fs::write(&rom_path, rom).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("battery")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets.clone()).unwrap();
        owner.set_audio(false, true, 0.0);
        owner
            .load_rom(&rom_path, SoundSessionMode::Persistent)
            .unwrap();
        let (worker, _, _) = owner.spawn_with_ports().unwrap();
        let applied = worker
            .queue_update_cheats(vec![CheatEntry {
                id: "worker-sram".to_owned(),
                game_id: "battery-worker".to_owned(),
                name: "Write battery byte".to_owned(),
                code: "700000:5A".to_owned(),
                source: "test".to_owned(),
                kind: "Action Replay".to_owned(),
                enabled: true,
            }])
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert_eq!(applied, 1);
        let _ = worker
            .queue_step_frame()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        worker
            .queue_clear_cheats()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        let stored = worker
            .transact(|engine| Ok(engine.session.host_read_u8(0x700000)))
            .unwrap();
        assert_eq!(stored, 0x5A);
        let save_path = worker
            .transact(|engine| {
                engine
                    .session
                    .save_ram_path()
                    .ok_or_else(|| "test cartridge has no writable battery SRAM".to_owned())
            })
            .unwrap();
        assert!(
            !save_path.exists(),
            "test must exercise shutdown's first SRAM flush"
        );
        let returned = worker.shutdown().unwrap();
        assert!(returned.shutdown_finalized);
        assert_eq!(std::fs::read(&save_path).unwrap()[0], 0x5A);

        let mut reopened = FrontendSession::new(assets).unwrap();
        reopened.load_rom_with_persistent_audio(&rom_path).unwrap();
        assert_eq!(reopened.host_read_u8(0x700000), 0x5A);
    }

    #[test]
    fn paused_worker_steps_one_frame_without_enqueuing_audio() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("step-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let mut owner = OwnedSession::create(assets).unwrap();
        owner.load_rom(&rom, SoundSessionMode::Persistent).unwrap();
        let (worker, _, _) = owner.spawn_with_ports().unwrap();
        let before = worker.read_status().unwrap().frame;
        let stepped = worker
            .queue_step_frame()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert_eq!(stepped.status.frame, before + 1);
        assert!(stepped.rgba.is_some());
        assert_eq!(stepped.audio_diagnostics.submitted_frames, 0);
        std::thread::sleep(Duration::from_millis(75));
        assert_eq!(worker.read_status().unwrap().frame, stepped.status.frame);
        worker.shutdown().unwrap();
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
        let (worker, _input, render) = engine.spawn_with_ports().unwrap();
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
        let diagnosed = worker
            .transact(|engine| Ok(engine.blackout_monitor.last_observed_frame()))
            .unwrap();
        assert!(
            diagnosed >= 5,
            "background emulation must also drive worker-owned diagnostics"
        );
        // No GUI heartbeat: game frames continue, but the mailbox must not
        // carry expensive RGBA copies that a hidden renderer cannot display.
        let hidden = worker.take_latest_frame().unwrap();
        assert!(hidden.rgba.is_none(), "hidden renderer still copied video");

        // Restoring a visible renderer enables the next complete video
        // frame without restarting or taking ownership of the SNES clock.
        render.set_visible(true);
        let video_deadline = Instant::now() + Duration::from_secs(2);
        let mut showed_video = false;
        while Instant::now() < video_deadline {
            if let Some(frame) = worker.take_latest_frame() {
                if frame.rgba.is_some() {
                    showed_video = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            showed_video,
            "renderer did not receive a fresh frame after resurfacing"
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
        // Use the same serialized action + optional picture receipt that
        // the library GUI will consume. Paused restores must show a complete
        // frame, while saves must not allocate a redundant framebuffer.
        let saved = worker
            .queue_save_action_with_frame(SessionSaveAction::QuickSave)
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(matches!(saved.outcome, SessionSaveOutcome::Completed));
        assert!(saved.restored_frame.is_none());
        assert!(saved.controls.has_quick_save);
        assert!(worker.read_controls().unwrap().has_quick_save);
        let restored = worker
            .queue_save_action_with_frame(SessionSaveAction::QuickLoad)
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(matches!(restored.outcome, SessionSaveOutcome::Completed));
        assert!(restored.controls.has_quick_save);
        let still = restored
            .restored_frame
            .expect("paused restore should capture video");
        assert!(still.status.has_rom);
        assert_eq!(
            still.rgba.as_ref().unwrap().len(),
            still.status.framebuffer_width as usize * still.status.framebuffer_height as usize * 4
        );
        let durable = worker
            .queue_save_action_with_frame(SessionSaveAction::SaveSlot(1))
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(matches!(durable.outcome, SessionSaveOutcome::SavedTo(_)));
        assert!(durable.controls.state_slots[0]);
        assert!(durable.restored_frame.is_none());
        let slot_restore = worker
            .queue_save_action_with_frame(SessionSaveAction::LoadSlot(1))
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(slot_restore.controls.state_slots[0]);
        assert!(slot_restore.restored_frame.is_some());
        let reply = worker.queue_clear_cheats().unwrap();
        reply.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        assert_eq!(worker.read_controls().unwrap().rom_cheat_read_hits, 0);

        // A launch request couples cartridge, sound mode and cheat list in
        // one worker-owned operation; reloading resets the old quick save.
        let reloaded = worker
            .queue_launch_with_state(SessionLaunchRequest {
                rom_path: rom.clone(),
                sound_mode: SoundSessionMode::Persistent,
                cheats: Vec::new(),
            })
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(reloaded.frame.status.has_rom);
        assert!(!reloaded.controls.has_quick_save);
        assert!(reloaded.controls.state_slots[0]);
        assert!(!worker.read_controls().unwrap().has_quick_save);
        let missing = dir.path().join("not-a-cartridge.sfc");
        let failed = worker
            .queue_launch_with_state(SessionLaunchRequest {
                rom_path: missing,
                sound_mode: SoundSessionMode::Persistent,
                cheats: Vec::new(),
            })
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(failed.is_err());
        let returned_owner = worker.shutdown().unwrap();
        assert_eq!(returned_owner.snapshot().rom_path, Some(rom));
        assert!(returned_owner.shutdown_finalized);
        assert!(returned_owner.audio.is_none());
    }

    #[test]
    fn published_controls_follow_mutations_without_ui_thread_transactions() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("controls-port-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            cache_dir: Some(dir.path().join("cache")),
            config_path: None,
        };
        let owner = OwnedSession::create(assets).unwrap();
        let published = owner.controls_port();
        assert!(!published.latest().session.has_rom);
        let (worker, _, _) = owner.spawn_with_ports().unwrap();
        let launch = worker
            .queue_launch_with_state(SessionLaunchRequest {
                rom_path: rom.clone(),
                sound_mode: SoundSessionMode::Persistent,
                cheats: Vec::new(),
            })
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert_eq!(published.latest().session.rom_path, Some(rom.clone()));
        assert_eq!(
            published.latest().session.rom_path,
            launch.controls.session.rom_path
        );
        assert!(!published.latest().has_quick_save);
        let saved = worker
            .queue_save_action_with_frame(SessionSaveAction::QuickSave)
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(saved.controls.has_quick_save);
        assert!(published.latest().has_quick_save);
        let missing = dir.path().join("missing.sfc");
        let failure = worker
            .queue_launch_with_state(SessionLaunchRequest {
                rom_path: missing,
                sound_mode: SoundSessionMode::Persistent,
                cheats: Vec::new(),
            })
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(failure.is_err());
        assert_eq!(published.latest().session.rom_path, Some(rom));
        assert!(published.latest().has_quick_save);
        worker.shutdown().unwrap();
    }

    #[test]
    fn scheduler_reuse_reopens_a_new_shutdown_lifecycle() {
        let owner = OwnedSession::create(AssetConfig::default()).unwrap();
        let worker = owner.spawn().unwrap();
        let returned = worker.shutdown().unwrap();
        assert!(returned.shutdown_finalized);
        let worker = returned.spawn().unwrap();
        let returned = worker.shutdown().unwrap();
        assert!(returned.shutdown_finalized);
    }

    #[test]
    fn detached_session_has_no_game_until_loaded() {
        let engine = OwnedSession::create(AssetConfig::default()).unwrap();
        assert!(!engine.snapshot().has_rom);
    }

    #[test]
    fn paused_sound_panel_refreshes_audio_counters_without_game_frames() {
        let mut owner = OwnedSession::create(AssetConfig::default()).unwrap();
        owner.audio = Some(AudioOutput::start().unwrap());
        owner
            .audio
            .as_ref()
            .unwrap()
            .enqueue_expired_tone_for_test();
        let worker = owner.spawn().unwrap();

        // No game is running. The normal video mailbox cannot publish updated
        // diagnostic counters, but the Sound panel's owner query still can.
        let deadline = Instant::now() + Duration::from_secs(2);
        let audio = loop {
            let (_, audio) = worker
                .queue_sound_panel_diagnostics()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap();
            if audio.expired_test_tones == 1 {
                break audio;
            }
            assert!(Instant::now() < deadline, "audio counter never advanced");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(audio.test_tones_submitted, 0);
        assert_eq!(audio.device_status, Some(AudioDeviceStatus::NotAttempted));
        assert_eq!(worker.completed_frames(), 0);
        worker.shutdown().unwrap();
    }

    #[test]
    fn owner_reports_expired_speaker_check_from_real_worker_without_os_audio() {
        let mut owner = OwnedSession::create(AssetConfig::default()).unwrap();
        owner.audio = Some(AudioOutput::start().unwrap());
        let output = owner.audio.as_ref().unwrap();
        output.enqueue_expired_tone_for_test();
        let deadline = Instant::now() + Duration::from_secs(2);
        while owner.audio_diagnostics().expired_test_tones == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let diagnostic = owner.audio_diagnostics();
        assert_eq!(diagnostic.expired_test_tones, 1);
        assert_eq!(diagnostic.test_tones_submitted, 0);
        assert_eq!(
            diagnostic.device_status,
            Some(AudioDeviceStatus::NotAttempted)
        );
    }

    #[test]
    fn paused_owner_reselects_dead_audio_worker_without_starting_speakers() {
        let mut owner = OwnedSession::create(AssetConfig::default()).unwrap();
        owner.set_audio(true, false, 0.35);
        owner.audio = Some(AudioOutput::start().unwrap());
        owner.audio.as_ref().unwrap().stop_worker_for_test();
        assert_eq!(
            owner.audio.as_ref().unwrap().device_status(),
            AudioDeviceStatus::NotAttempted
        );

        // No emulation frames are advancing: explicit route selection must
        // recover a dead receiver without waiting for Play to resume.
        owner.request_audio_reopen().unwrap();
        let output = owner.audio.as_ref().expect("recreated playback worker");
        assert_eq!(output.device_status(), AudioDeviceStatus::NotAttempted);
        assert!(output.request_reopen());
        assert_eq!(output.device_status(), AudioDeviceStatus::NotAttempted);
        assert_eq!(owner.next_audio_retry <= Instant::now(), false);
        assert_eq!(output.stream_failures(), 0);
        assert_eq!(output.stream_recoveries(), 0);
    }

    #[test]
    fn muted_worker_speaker_check_rejects_test_tone_without_opening_device() {
        let mut owner = OwnedSession::create(AssetConfig::default()).unwrap();
        owner.set_audio(true, true, 0.5);
        let worker = owner.spawn().unwrap();
        let response = worker.queue_speaker_check().unwrap();
        assert!(
            response
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
        assert_eq!(worker.frame_stats().fps, None);
        worker.shutdown().unwrap();
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
