use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};
use gilrs::{Axis, EventType, GamepadId, Gilrs};
use image::ImageReader;
use tracing::{debug, info, warn};

use crate::{
    audio::{AudioDeviceStatus, AudioOutput},
    logging::SharedLogBuffer,
    worker::{AppWorker, WorkerCommand, WorkerCommandKind, WorkerEvent},
};

use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, InputDeviceMode, LibraryViewMode, RuntimeConfig},
};
use starbyte_frontend::{
    CheatEntry, FrontendSession, InstalledStatus, LibraryEntry, LibraryFilter, LibraryService,
    LibrarySnapshot, LibraryTarget, SoundSessionMode,
};

// NTSC SNES video: 341 dots/scanline × 262 lines × 4 master clocks/dot
// at 21,477,272 master clocks/second ≈ 60.0985 frames/second.
// Pacing at a generic 60.0 Hz slowly desynchronizes eventual DSP audio.
const FRAME_INTERVAL: Duration = Duration::from_nanos(16_639_357);
// A bad game frame can run on the egui thread: return control to Wayland
// with a diagnostic rather than freezing until the process is killed.
const GUI_FRAME_TIMEOUT: Duration = Duration::from_millis(1_500);
/// Log only actionable spikes rather than streaming telemetry every 60 Hz frame.
const FRAME_SPIKE_LOG_THRESHOLD_MS: f32 = 40.0;
/// Log intermittent sub-40ms pacing gaps that can still lower game FPS.
const FRAME_PACING_LOG_THRESHOLD_MS: f32 = 20.0;
const FRAME_SPIKE_LOG_INTERVAL: Duration = Duration::from_secs(5);
/// Display frame completion rate over a bounded rolling window, not 1 / CPU-frame time.
const FPS_WINDOW: Duration = Duration::from_secs(2);
const FPS_WARMUP: Duration = Duration::from_millis(250);
/// A whole-frame completion gap above this is a perceptible hitch.
const FRAME_HITCH_THRESHOLD: Duration = Duration::from_millis(40);
const COMPACT_LAYOUT_WIDTH: f32 = 960.0;
/// Treat a deflected primary analog stick like the SNES D-pad, including
/// controllers that report directional input as axes rather than buttons.
const GAMEPAD_STICK_DEADZONE: f32 = 0.5;
/// Avoid binding from minor stick drift or calibration noise.
const GAMEPAD_BIND_AXIS_THRESHOLD: f32 = 0.75;

#[derive(Debug, Default, Clone, Copy)]
struct StickAxes {
    x: f32,
    y: f32,
}

impl StickAxes {
    fn dpad_buttons(self) -> BTreeSet<String> {
        let mut buttons = BTreeSet::new();
        if self.x <= -GAMEPAD_STICK_DEADZONE {
            buttons.insert("DPadLeft".to_owned());
        } else if self.x >= GAMEPAD_STICK_DEADZONE {
            buttons.insert("DPadRight".to_owned());
        }
        // Gilrs' normalized LeftStickY is positive when pushed upward.
        if self.y >= GAMEPAD_STICK_DEADZONE {
            buttons.insert("DPadUp".to_owned());
        } else if self.y <= -GAMEPAD_STICK_DEADZONE {
            buttons.insert("DPadDown".to_owned());
        }
        buttons
    }

    /// Bindable positive/negative stick axes share the same names used by
    /// gilrs event capture. The left stick also supplies virtual D-pad input.
    fn axis_bindings(self, prefix: &str) -> BTreeSet<String> {
        let mut active = BTreeSet::new();
        if self.x <= -GAMEPAD_STICK_DEADZONE {
            active.insert(format!("{prefix}X-"));
        } else if self.x >= GAMEPAD_STICK_DEADZONE {
            active.insert(format!("{prefix}X+"));
        }
        if self.y <= -GAMEPAD_STICK_DEADZONE {
            active.insert(format!("{prefix}Y-"));
        } else if self.y >= GAMEPAD_STICK_DEADZONE {
            active.insert(format!("{prefix}Y+"));
        }
        active
    }
}

/// Capture stick direction only when an axis first crosses the intentional
/// binding threshold; small drift must never create a binding.
fn gamepad_axis_binding(axis: Axis, before: f32, after: f32) -> Option<&'static str> {
    gamepad_axis_crossing(axis, before, after, GAMEPAD_BIND_AXIS_THRESHOLD)
}

fn gamepad_axis_crossing(
    axis: Axis,
    before: f32,
    after: f32,
    threshold: f32,
) -> Option<&'static str> {
    let (positive, negative) = match axis {
        Axis::LeftStickX => ("LeftStickX+", "LeftStickX-"),
        Axis::LeftStickY => ("LeftStickY+", "LeftStickY-"),
        Axis::RightStickX => ("RightStickX+", "RightStickX-"),
        Axis::RightStickY => ("RightStickY+", "RightStickY-"),
        Axis::DPadX => ("DPadRight", "DPadLeft"),
        Axis::DPadY => ("DPadUp", "DPadDown"),
        _ => return None,
    };
    if before < threshold && after >= threshold {
        Some(positive)
    } else if before > -threshold && after <= -threshold {
        Some(negative)
    } else {
        None
    }
}

/// Which virtual directional button would be triggered by a left-stick
/// gesture? Suppress both forms until that axis returns to neutral.
fn virtual_dpad_for_axis(binding: &str) -> Option<&'static str> {
    match binding {
        "LeftStickX+" => Some("DPadRight"),
        "LeftStickX-" => Some("DPadLeft"),
        "LeftStickY+" => Some("DPadUp"),
        "LeftStickY-" => Some("DPadDown"),
        _ => None,
    }
}

fn release_captured_axis(suppressed: &mut BTreeSet<String>, axis: Axis, value: f32) {
    if value.abs() >= GAMEPAD_STICK_DEADZONE {
        return;
    }
    let names: &[&str] = match axis {
        Axis::LeftStickX => &["LeftStickX+", "LeftStickX-", "DPadLeft", "DPadRight"],
        Axis::LeftStickY => &["LeftStickY+", "LeftStickY-", "DPadUp", "DPadDown"],
        Axis::RightStickX => &["RightStickX+", "RightStickX-"],
        Axis::RightStickY => &["RightStickY+", "RightStickY-"],
        Axis::DPadX => &["DPadLeft", "DPadRight"],
        Axis::DPadY => &["DPadUp", "DPadDown"],
        _ => &[],
    };
    for name in names {
        suppressed.remove(*name);
    }
}

fn frame_spike_reportable(stats: FramePerformance) -> bool {
    stats.last_ms >= FRAME_SPIKE_LOG_THRESHOLD_MS
        || stats.last_wake_late_ms >= FRAME_SPIKE_LOG_THRESHOLD_MS
        || stats.last_texture_ms >= FRAME_SPIKE_LOG_THRESHOLD_MS
}

fn is_compact_layout(width: f32, height: f32) -> bool {
    width < COMPACT_LAYOUT_WIDTH || height < 640.0
}

/// A separate, bounded OS sound check; game audio and saved progress are never
/// touched. Keep the result until another explicit speaker test replaces it.
enum SpeakerProbeState {
    Idle,
    Running(Receiver<std::result::Result<(), String>>),
    Finished(std::result::Result<(), String>),
}

impl SpeakerProbeState {
    /// A polling receive must never block an egui frame on audio hardware.
    fn poll(&mut self) -> bool {
        let Self::Running(receiver) = self else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                Err("Speaker test ended without reporting its result".to_owned())
            }
        };
        *self = Self::Finished(result);
        true
    }
}

/// Counts successful emulated frames completed per *wall-clock* second.
/// Unlike 1 / core time, this includes frame scheduling, texture submission
/// and audio queueing. It cannot measure GPU presentation or screen refresh.
#[derive(Debug, Default)]
struct FrameRateMeter {
    completed_at: VecDeque<Instant>,
    hitches: u64,
    last_gap_ms: f32,
    peak_gap_ms: f32,
}

impl FrameRateMeter {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn record(&mut self, now: Instant) {
        if let Some(previous) = self.completed_at.back() {
            let gap = now.saturating_duration_since(*previous);
            self.last_gap_ms = gap.as_secs_f32() * 1_000.0;
            self.peak_gap_ms = self.peak_gap_ms.max(self.last_gap_ms);
            if gap >= FRAME_HITCH_THRESHOLD {
                self.hitches = self.hitches.saturating_add(1);
            }
        }
        self.completed_at.push_back(now);
        while self
            .completed_at
            .front()
            .is_some_and(|first| now.saturating_duration_since(*first) > FPS_WINDOW)
        {
            self.completed_at.pop_front();
        }
    }

    fn fps(&self, now: Instant) -> Option<f32> {
        let cutoff = now.checked_sub(FPS_WINDOW).unwrap_or(now);
        let first = self.completed_at.iter().position(|time| *time >= cutoff)?;
        let frames = self.completed_at.len() - first;
        if frames < 2 {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.completed_at[first]);
        if elapsed < FPS_WARMUP {
            return None;
        }
        Some((frames - 1) as f32 / elapsed.as_secs_f32())
    }
}

/// Track real emulation-frame runtime, excluding texture uploads/UI rendering.
#[derive(Debug, Default, Clone, Copy)]
struct FramePerformance {
    samples: u64,
    avg_ms: f32,
    last_ms: f32,
    peak_ms: f32,
    slow_frames: u64,
    /// UI wake scheduling delay, distinct from core emulation time.
    last_wake_late_ms: f32,
    peak_wake_late_ms: f32,
    wakes_over_budget: u64,
    /// CPU-side pixel copy and egui texture submission, not GPU presentation.
    last_texture_ms: f32,
    peak_texture_ms: f32,
}

impl FramePerformance {
    fn record(&mut self, elapsed: Duration) {
        let ms = elapsed.as_secs_f32() * 1_000.0;
        self.avg_ms = if self.samples == 0 {
            ms
        } else {
            self.avg_ms * 0.9 + ms * 0.1
        };
        self.samples += 1;
        self.last_ms = ms;
        self.peak_ms = self.peak_ms.max(ms);
        if elapsed > FRAME_INTERVAL {
            self.slow_frames += 1;
        }
    }

    fn record_wake_lateness(&mut self, late: Duration) {
        self.last_wake_late_ms = late.as_secs_f32() * 1_000.0;
        self.peak_wake_late_ms = self.peak_wake_late_ms.max(self.last_wake_late_ms);
        if late > FRAME_INTERVAL {
            self.wakes_over_budget = self.wakes_over_budget.saturating_add(1);
        }
    }

    fn record_texture_upload(&mut self, elapsed: Duration) {
        self.last_texture_ms = elapsed.as_secs_f32() * 1_000.0;
        self.peak_texture_ms = self.peak_texture_ms.max(self.last_texture_ms);
    }
}

/// Schedule no more than one frame per UI update; never pile up catch-up work.
#[derive(Debug, Clone, Copy)]
struct FrameClock {
    next_frame_at: Instant,
}

impl FrameClock {
    fn new(now: Instant) -> Self {
        Self { next_frame_at: now }
    }

    fn reset(&mut self, now: Instant) {
        self.next_frame_at = now;
    }

    fn take_due_frame(&mut self, now: Instant) -> bool {
        if now < self.next_frame_at {
            return false;
        }
        // Keep the NTSC cadence despite ordinary UI wake-up jitter.
        // When we miss a whole frame, resynchronize instead of catching up
        // with an expensive burst of emulated frames.
        let following_deadline = self.next_frame_at + FRAME_INTERVAL;
        self.next_frame_at = if now < following_deadline {
            following_deadline
        } else {
            now + FRAME_INTERVAL
        };
        true
    }

    fn time_until_next_frame(&self, now: Instant) -> Duration {
        self.next_frame_at.saturating_duration_since(now)
    }
}

/// Cumulative PCM evidence for an explicitly bounded native GUI smoke.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct NativeSmokeAudioMetrics {
    stereo_pairs: u64,
    nonzero_pairs: u64,
    peak_absolute: u16,
    first_nonzero_frame: Option<u64>,
}

impl NativeSmokeAudioMetrics {
    fn record(&mut self, frame: u64, samples: &[i16]) {
        for pair in samples.chunks_exact(2) {
            self.stereo_pairs = self.stereo_pairs.saturating_add(1);
            if pair[0] != 0 || pair[1] != 0 {
                self.nonzero_pairs = self.nonzero_pairs.saturating_add(1);
                self.first_nonzero_frame.get_or_insert(frame);
            }
            self.peak_absolute = self
                .peak_absolute
                .max(pair[0].unsigned_abs())
                .max(pair[1].unsigned_abs());
        }
    }
}

#[derive(Debug, Default)]
struct QueuedKeyboardPresses {
    actions: BTreeSet<String>,
}

impl QueuedKeyboardPresses {
    /// Collect press edges even when a UI update does not run the emulator.
    fn capture(
        &mut self,
        ctx: &egui::Context,
        bindings: &BTreeMap<String, String>,
        allow_keyboard_gameplay: bool,
    ) {
        if !allow_keyboard_gameplay {
            self.actions.clear();
            return;
        }
        for &action in input_actions() {
            if let Some(binding) = bindings.get(action)
                && let Some(key) = parse_egui_key(binding)
                && ctx.input(|input| input.key_pressed(key))
            {
                self.actions.insert(action.to_owned());
            }
        }
    }

    fn contains(&self, action: &str) -> bool {
        self.actions.contains(action)
    }

    fn clear(&mut self) {
        self.actions.clear();
    }
}

#[derive(Debug, Clone)]
struct JobRecord {
    id: u64,
    label: String,
    state: &'static str,
    detail: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CheatSort {
    #[default]
    Name,
    Kind,
    Source,
    Code,
    Status,
}

impl CheatSort {
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Kind => "Type",
            Self::Source => "Source",
            Self::Code => "Code",
            Self::Status => "Enabled",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CheatVisibility {
    #[default]
    All,
    Enabled,
    Available,
    Unavailable,
}

impl CheatVisibility {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Enabled => "Enabled",
            Self::Available => "Usable",
            Self::Unavailable => "Unsupported",
        }
    }
}

fn filtered_sorted_cheats<'a>(
    cheats: &'a [CheatEntry],
    query: &str,
    visibility: CheatVisibility,
    sort: CheatSort,
    descending: bool,
) -> Vec<&'a CheatEntry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut results: Vec<&CheatEntry> = cheats
        .iter()
        .filter(|cheat| match visibility {
            CheatVisibility::All => true,
            CheatVisibility::Enabled => cheat.enabled,
            CheatVisibility::Available => cheat.can_execute(),
            CheatVisibility::Unavailable => !cheat.can_execute(),
        })
        .filter(|cheat| {
            words.is_empty() || {
                let fields = format!(
                    "{} {} {} {}",
                    cheat.name, cheat.code, cheat.kind, cheat.source
                )
                .to_lowercase();
                words.iter().all(|word| fields.contains(word))
            }
        })
        .collect();
    results.sort_by(|left, right| {
        let primary = match sort {
            CheatSort::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            CheatSort::Kind => left.kind.to_lowercase().cmp(&right.kind.to_lowercase()),
            CheatSort::Source => left.source.to_lowercase().cmp(&right.source.to_lowercase()),
            CheatSort::Code => left.code.to_lowercase().cmp(&right.code.to_lowercase()),
            CheatSort::Status => left.enabled.cmp(&right.enabled),
        };
        let primary = if descending {
            primary.reverse()
        } else {
            primary
        };
        primary
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    results
}

pub struct StarbyteApp {
    assets: AssetConfig,
    config: RuntimeConfig,
    cache_root: PathBuf,
    session: FrontendSession,
    audio_output: Option<AudioOutput>,
    speaker_probe: SpeakerProbeState,
    /// Session mute defaults from persisted startup preference, then follows
    /// the live in-game Sound menu without rewriting that preference.
    audio_muted: bool,
    /// Confirmation is deliberately per-session and not persisted.
    confirm_experimental_audio: bool,
    last_audio_pairs: usize,
    last_audio_nonzero_pairs: usize,
    /// Cumulative produced PCM signal in the current ROM/session. A quiet
    /// frame must not erase evidence that real samples were generated.
    audio_nonzero_pairs_total: u64,
    worker: AppWorker,
    library_snapshot: LibrarySnapshot,
    framebuffer_texture: Option<TextureHandle>,
    cover_textures: BTreeMap<String, TextureHandle>,
    failed_cover_ids: BTreeSet<String>,
    held_input: ControllerState,
    queued_keyboard_presses: QueuedKeyboardPresses,
    status_line: String,
    playback_error: Option<String>,
    search_query: String,
    cheat_search_query: String,
    cheat_sort: CheatSort,
    cheat_sort_descending: bool,
    cheat_visibility: CheatVisibility,
    /// Search needs durable keyboard focus; a menu popup closes on text clicks.
    show_cheats_window: bool,
    selected_game_id: Option<String>,
    loaded_game_id: Option<String>,
    show_properties: bool,
    rom_dir_input: String,
    cheatarium_root_input: String,
    logs: SharedLogBuffer,
    jobs: Vec<JobRecord>,
    next_job_id: u64,
    gilrs: Option<Gilrs>,
    gamepad_buttons_down: BTreeSet<String>,
    gamepad_buttons_by_id: HashMap<GamepadId, BTreeSet<String>>,
    /// Unconsumed button press edges by physical controller.
    queued_gamepad_presses: HashMap<GamepadId, BTreeSet<String>>,
    gamepad_sticks_by_id: HashMap<GamepadId, StickAxes>,
    gamepad_right_sticks_by_id: HashMap<GamepadId, StickAxes>,
    gamepad_hat_axes_by_id: HashMap<GamepadId, StickAxes>,
    /// A newly captured input must be released before it can reach the game.
    captured_gamepad_inputs: HashMap<GamepadId, BTreeSet<String>>,
    pending_keyboard_bind: Option<String>,
    pending_gamepad_bind: Option<String>,
    is_playing: bool,
    play_view: bool,
    frame_clock: FrameClock,
    frame_performance: FramePerformance,
    frame_rate: FrameRateMeter,
    last_frame_spike_log: Instant,
    show_performance_overlay: bool,
    show_input_overlay: bool,
    pending_step_frames: u32,
    /// Explicitly requested bounded native window smoke; never persistent.
    exit_after_frames: Option<u64>,
    native_smoke_audio: NativeSmokeAudioMetrics,
    last_sram_flush: Instant,
    show_compact_settings: bool,
    show_compact_session: bool,
    show_compact_logs: bool,
}

// Regular desktop game loads use audible emulation with writable SRAM.
// Keep the old silent compatibility path available only as an explicit fallback.
fn load_rom_with_default_sound(session: &mut FrontendSession, path: &Path) -> Result<()> {
    session.load_rom_with_persistent_audio(path)
}

impl StarbyteApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets: AssetConfig,
        mut config: RuntimeConfig,
        rom_path: Option<PathBuf>,
        startup_rom_dirs: Vec<PathBuf>,
        experimental_sound_driver: bool,
        prefer_dark_mode_override: Option<bool>,
        exit_after_frames: Option<u64>,
        logs: SharedLogBuffer,
    ) -> Result<Self> {
        if let Some(prefer_dark_mode) = prefer_dark_mode_override {
            config.prefer_dark_mode = prefer_dark_mode;
        }
        for rom_dir in startup_rom_dirs {
            if !config.library.rom_dirs.contains(&rom_dir) {
                config.library.rom_dirs.push(rom_dir);
            }
        }
        if let Some(path) = &rom_path
            && let Some(parent) = path.parent()
        {
            let parent = parent.to_path_buf();
            if !config.library.rom_dirs.contains(&parent) {
                config.library.rom_dirs.push(parent);
            }
        }

        // A checkout already present on disk should work on first launch.
        // Persist discovery once; never replace a path the player configured.
        if config.cheats.cheatarium_root.is_none()
            && let Some(home) = std::env::var_os("HOME")
            && let Some(found) =
                starbyte_core::manifest::CheatSettings::discover_local_checkout(Path::new(&home))
        {
            info!(path = %found.display(), "discovered local Cheatarium checkout");
            config.cheats.cheatarium_root = Some(found);
        }
        let cache_root = resolve_cache_root(&config, &assets);
        apply_theme(&cc.egui_ctx, config.prefer_dark_mode);
        info!(
            config_path = %assets.config_path().display(),
            cache_root = %cache_root.display(),
            prefer_dark_mode = config.prefer_dark_mode,
            rom_dirs = ?config.library.rom_dirs,
            providers_enabled = config.advanced.providers.enable_network,
            "initialized gui app state"
        );

        let mut session = FrontendSession::new(assets.clone())?;
        let mut status_line = "Waiting for library scan...".to_owned();
        let mut startup_error = None;
        let mut native_smoke_audio = NativeSmokeAudioMetrics::default();
        if let Some(path) = rom_path {
            if experimental_sound_driver {
                session.load_rom_with_isolated_audio_probe(&path)?;
            } else {
                load_rom_with_default_sound(&mut session, &path)?;
            }
            record_recent_rom(&mut config.library.recent_roms, path.as_path());
            match session.run_frame_with_timeout(GUI_FRAME_TIMEOUT) {
                Ok(()) => {
                    status_line = format!("Loaded {}", path.display());
                    if exit_after_frames.is_some() {
                        native_smoke_audio.record(
                            session.snapshot().frame,
                            &session.emulator().audio_samples().samples,
                        );
                    }
                }
                Err(error) => {
                    let message = format!("Emulation stopped: {error}");
                    status_line = message.clone();
                    startup_error = Some(message);
                }
            }
        }

        let has_loaded_rom = session.snapshot().has_rom;
        let start_playing = has_loaded_rom && startup_error.is_none();
        let worker = AppWorker::spawn(assets.clone());
        let gilrs = Gilrs::new().ok();
        let has_connected_gamepad = gilrs
            .as_ref()
            .is_some_and(|backend| backend.gamepads().any(|(_, pad)| pad.is_connected()));
        let cached_snapshot = LibraryService::new(config.clone(), assets.clone())
            .ok()
            .and_then(|service| service.load_cached_snapshot().ok().flatten());
        let status_line = if cached_snapshot.is_some() {
            "Loaded cached library snapshot.".to_owned()
        } else {
            status_line
        };
        let audio_muted = config.audio.mute_on_startup;
        let cheatarium_root_input = config
            .cheats
            .cheatarium_root
            .as_ref()
            .map(|root| root.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut app = Self {
            assets,
            config,
            cache_root,
            session,
            audio_output: None,
            speaker_probe: SpeakerProbeState::Idle,
            audio_muted,
            confirm_experimental_audio: false,
            last_audio_pairs: 0,
            last_audio_nonzero_pairs: 0,
            audio_nonzero_pairs_total: 0,
            worker,
            library_snapshot: cached_snapshot.unwrap_or_else(empty_snapshot),
            framebuffer_texture: None,
            cover_textures: BTreeMap::new(),
            failed_cover_ids: BTreeSet::new(),
            held_input: ControllerState::default(),
            queued_keyboard_presses: QueuedKeyboardPresses::default(),
            status_line,
            playback_error: startup_error,
            search_query: String::new(),
            cheat_search_query: String::new(),
            cheat_sort: CheatSort::default(),
            cheat_sort_descending: false,
            cheat_visibility: CheatVisibility::default(),
            show_cheats_window: false,
            selected_game_id: None,
            loaded_game_id: None,
            show_properties: false,
            rom_dir_input: String::new(),
            cheatarium_root_input,
            logs,
            jobs: Vec::new(),
            next_job_id: 1,
            gilrs,
            gamepad_buttons_down: BTreeSet::new(),
            gamepad_buttons_by_id: HashMap::new(),
            queued_gamepad_presses: HashMap::new(),
            gamepad_sticks_by_id: HashMap::new(),
            gamepad_right_sticks_by_id: HashMap::new(),
            gamepad_hat_axes_by_id: HashMap::new(),
            captured_gamepad_inputs: HashMap::new(),
            pending_keyboard_bind: None,
            pending_gamepad_bind: None,
            is_playing: start_playing,
            play_view: has_loaded_rom,
            frame_clock: FrameClock::new(Instant::now()),
            frame_performance: FramePerformance::default(),
            frame_rate: FrameRateMeter::default(),
            last_frame_spike_log: Instant::now(),
            show_performance_overlay: false,
            show_input_overlay: has_connected_gamepad,
            pending_step_frames: 0,
            exit_after_frames,
            native_smoke_audio,
            last_sram_flush: Instant::now(),
            show_compact_settings: false,
            show_compact_session: false,
            show_compact_logs: false,
        };
        app.persist_config();
        if app.config.advanced.refresh_on_startup {
            app.queue_job(WorkerCommandKind::RefreshMetadata);
        } else {
            // Cached snapshots provide instant startup, but they must not
            // prevent a fresh, asynchronous scan of newly added ZIP games.
            app.queue_job(WorkerCommandKind::RefreshSnapshot);
        }
        // Cheatarium is a local data source, so changes to its checkout
        // become visible on the next launch without a network request or a
        // separate manual refresh. Keep this work off the UI thread.
        if app
            .config
            .cheats
            .cheatarium_snes_index()
            .is_some_and(|index| index.is_file())
        {
            app.queue_job(WorkerCommandKind::RefreshCheats {
                target: LibraryTarget {
                    installed_only: true,
                    ..LibraryTarget::default()
                },
            });
        }
        Ok(app)
    }

    fn current_filter(&self) -> LibraryFilter {
        LibraryFilter {
            query: String::new(),
            installed_only: false,
            view_mode: self.config.library.active_view,
        }
    }

    fn visible_entries(&self) -> Vec<LibraryEntry> {
        let mut entries = self.library_snapshot.entries.clone();
        if self.config.library.show_installed_only {
            entries.retain(|entry| entry.installed_status == InstalledStatus::Installed);
        }
        if !self.search_query.trim().is_empty() {
            let needle = normalize_query(&self.search_query);
            entries.retain(|entry| normalize_query(&entry.display_title).contains(&needle));
        }
        entries
    }

    fn persist_config(&mut self) {
        if let Err(error) = self.config.save_to_path(self.assets.config_path()) {
            self.status_line = error.to_string();
            warn!("{error}");
        } else {
            debug!(path = %self.assets.config_path().display(), "persisted GUI config");
        }
    }

    fn queue_job(&mut self, kind: WorkerCommandKind) {
        let job_id = self.next_job_id;
        self.next_job_id += 1;
        let label = job_label(&kind).to_owned();
        self.jobs.push(JobRecord {
            id: job_id,
            label: label.clone(),
            state: "queued",
            detail: "Queued".to_owned(),
        });
        self.status_line = format!("{label} queued...");
        self.worker.submit(WorkerCommand {
            job_id,
            config: self.config.clone(),
            filter: self.current_filter(),
            kind,
        });
    }

    fn poll_worker_events(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.worker.try_recv() {
            match event {
                WorkerEvent::JobStarted { job_id, label } => {
                    self.update_job(job_id, &label, "running", "Running");
                    self.status_line = format!("{label} in progress...");
                }
                WorkerEvent::SnapshotReady {
                    job_id,
                    snapshot,
                    config,
                    status,
                } => {
                    // Only merge non-destructive refresh bookkeeping. Worker
                    // configs were cloned before the player's latest edits.
                    merge_worker_refresh_bookkeeping(&mut self.config, &config);
                    self.persist_config();
                    self.library_snapshot = snapshot;
                    self.failed_cover_ids.clear();
                    // A ROM opened directly from a ZIP was not necessarily
                    // indexed when it first loaded. Associate it after the
                    // asynchronous library scan, then apply matching cheats.
                    if self.loaded_game_id.is_none()
                        && let Some(loaded_path) = self.session.loaded_rom_path()
                    {
                        let recent = self.config.library.recent_roms.first();
                        self.loaded_game_id = self
                            .library_snapshot
                            .entries
                            .iter()
                            .find(|entry| {
                                entry.local.as_ref().is_some_and(|local| {
                                    recent.is_some_and(|path| path == &local.rom_path)
                                        || local.rom_path == loaded_path
                                        || local.extracted_cache_path.as_deref()
                                            == Some(loaded_path)
                                })
                            })
                            .map(|entry| entry.game_id.clone());
                    }
                    self.update_snapshot_cheat_flags();
                    self.sync_loaded_game_cheats();
                    self.update_job(job_id, "Library", "done", &status);
                    self.status_line = status;
                }
                WorkerEvent::RomReady {
                    job_id,
                    entry,
                    rom_path,
                } => match {
                    self.clear_audio();
                    load_rom_with_default_sound(&mut self.session, &rom_path)
                } {
                    Ok(()) => {
                        self.loaded_game_id = Some(entry.game_id.clone());
                        let _ = self.session.set_active_cheats(&entry.cheats);
                        self.remember_recent_rom(&rom_path);
                        if let Err(error) = self.session.run_frame_with_timeout(GUI_FRAME_TIMEOUT) {
                            let error = error.to_string();
                            self.record_playback_error(&error);
                            self.play_view = true;
                            self.refresh_framebuffer(ctx);
                            self.update_job(job_id, "Load Game", "failed", &error);
                            continue;
                        }
                        self.playback_error = None;
                        self.refresh_framebuffer(ctx);
                        self.is_playing = true;
                        self.play_view = true;
                        self.frame_clock.reset(Instant::now());
                        self.frame_performance = FramePerformance::default();
                        self.frame_rate.reset();
                        self.pending_step_frames = 0;
                        self.last_sram_flush = Instant::now();
                        let detail = format!("Loaded {}", rom_path.display());
                        self.update_job(job_id, "Load Game", "done", &detail);
                        self.status_line = detail;
                    }
                    Err(error) => {
                        self.update_job(job_id, "Load Game", "failed", &error.to_string());
                        self.status_line = error.to_string();
                    }
                },
                WorkerEvent::JobFailed {
                    job_id,
                    label,
                    error,
                } => {
                    self.update_job(job_id, &label, "failed", &error);
                    self.status_line = error;
                }
            }
        }
    }

    fn update_job(&mut self, job_id: u64, label: &str, state: &'static str, detail: &str) {
        if let Some(job) = self.jobs.iter_mut().find(|job| job.id == job_id) {
            job.label = label.to_owned();
            job.state = state;
            job.detail = detail.to_owned();
        } else {
            self.jobs.push(JobRecord {
                id: job_id,
                label: label.to_owned(),
                state,
                detail: detail.to_owned(),
            });
        }
        if self.jobs.len() > 24 {
            let excess = self.jobs.len().saturating_sub(24);
            self.jobs.drain(0..excess);
        }
    }

    fn selected_entry(&self) -> Option<LibraryEntry> {
        let selected = self.selected_game_id.as_deref()?;
        self.visible_entries()
            .into_iter()
            .find(|entry| entry.game_id == selected)
            .or_else(|| {
                self.library_snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.game_id == selected)
                    .cloned()
            })
    }

    fn sync_loaded_game_cheats(&mut self) {
        let Some(game_id) = self.loaded_game_id.clone() else {
            self.session.clear_active_cheats();
            return;
        };
        if let Some(entry) = self
            .library_snapshot
            .entries
            .iter()
            .find(|entry| entry.game_id == game_id)
        {
            let _ = self.session.set_active_cheats(&entry.cheats);
        } else {
            self.session.clear_active_cheats();
        }
    }

    fn update_snapshot_cheat_flags(&mut self) {
        for entry in &mut self.library_snapshot.entries {
            let enabled = self
                .config
                .cheats
                .enabled_by_game
                .get(&entry.game_id)
                .cloned()
                .unwrap_or_default();
            for cheat in &mut entry.cheats {
                cheat.enabled =
                    cheat.can_execute() && enabled.iter().any(|value| value == &cheat.id);
            }
        }
    }

    fn refresh_framebuffer(&mut self, ctx: &egui::Context) {
        let snapshot = self.session.snapshot();
        let image = ColorImage::from_rgba_unmultiplied(
            [
                snapshot.framebuffer_width as usize,
                snapshot.framebuffer_height as usize,
            ],
            self.session.framebuffer_rgba(),
        );

        if let Some(texture) = &mut self.framebuffer_texture {
            texture.set(image, TextureOptions::NEAREST);
        } else {
            self.framebuffer_texture =
                Some(ctx.load_texture("starbyte-framebuffer", image, TextureOptions::NEAREST));
        }
    }

    fn effective_controller_state(&self, ctx: &egui::Context) -> ControllerState {
        // Never route live controller input to gameplay when Hyprland has
        // moved native window focus elsewhere.
        if !ctx.input(|input| input.focused) {
            return ControllerState::default();
        }
        // Keyboard focus must not suppress an independent gamepad when the
        // default Auto input mode is active.
        let (keyboard_enabled, gamepad_enabled) = controller_input_sources(
            self.config.input.active_device,
            ctx.wants_keyboard_input(),
            self.pending_keyboard_bind.is_some(),
        );
        let mut state = self.held_input;
        if keyboard_enabled {
            for &action in input_actions() {
                if let Some(binding) = self.config.input.keyboard_bindings.get(action)
                    && let Some(key) = parse_egui_key(binding)
                    && (self.queued_keyboard_presses.contains(action)
                        || ctx.input(|input| gameplay_key_active(input, key)))
                {
                    set_controller_flag(&mut state, action, true);
                }
            }
        }
        if gamepad_enabled
            && self.pending_gamepad_bind.is_none()
            && self.pending_keyboard_bind.is_none()
        {
            for &action in input_actions() {
                if let Some(binding) = self.config.input.gamepad_bindings.get(action)
                    && gamepad_button_active(
                        &self.gamepad_buttons_down,
                        &self.queued_gamepad_presses,
                        binding,
                    )
                {
                    set_controller_flag(&mut state, action, true);
                }
            }
        }
        state
    }

    fn capture_gameplay_key_presses(&mut self, ctx: &egui::Context, was_recording_binding: bool) {
        let (keyboard_enabled, _) = controller_input_sources(
            self.config.input.active_device,
            ctx.wants_keyboard_input(),
            was_recording_binding || self.pending_keyboard_bind.is_some(),
        );
        self.queued_keyboard_presses.capture(
            ctx,
            &self.config.input.keyboard_bindings,
            keyboard_enabled
                && self.session.snapshot().has_rom
                && (self.is_playing || self.pending_step_frames > 0),
        );
    }

    fn capture_keyboard_binding(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_keyboard_bind.clone() else {
            return;
        };
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            if let egui::Event::Key {
                key, pressed: true, ..
            } = event
            {
                self.config
                    .input
                    .keyboard_bindings
                    .insert(action.clone(), format!("{key:?}"));
                self.pending_keyboard_bind = None;
                self.persist_config();
                self.status_line = format!("Bound {} to {key:?}.", action_label(&action));
                break;
            }
        }
    }

    fn poll_gamepad_events(&mut self, window_focused: bool) {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return;
        };
        let mut new_binding: Option<(String, String)> = None;
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(button, _) => {
                    let name = format!("{button:?}");
                    self.gamepad_buttons_by_id
                        .entry(event.id)
                        .or_default()
                        .insert(name.clone());
                    if window_focused
                        && self.pending_gamepad_bind.is_none()
                        && self.pending_keyboard_bind.is_none()
                        && (self.is_playing || self.pending_step_frames > 0)
                        && matches!(
                            self.config.input.active_device,
                            InputDeviceMode::Auto | InputDeviceMode::Gamepad
                        )
                    {
                        // A press followed by release in this very poll must
                        // remain visible for the next SNES controller sample.
                        self.queued_gamepad_presses
                            .entry(event.id)
                            .or_default()
                            .insert(name.clone());
                    }
                    if let Some(action) = self.pending_gamepad_bind.clone()
                        && window_focused
                        && new_binding.is_none()
                    {
                        new_binding = Some((action, name.clone()));
                        self.captured_gamepad_inputs
                            .entry(event.id)
                            .or_default()
                            .insert(name);
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    let name = format!("{button:?}");
                    if let Some(buttons) = self.gamepad_buttons_by_id.get_mut(&event.id) {
                        buttons.remove(&name);
                    }
                    if let Some(suppressed) = self.captured_gamepad_inputs.get_mut(&event.id) {
                        suppressed.remove(&name);
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    let before = match axis {
                        Axis::LeftStickX => {
                            let stick = self.gamepad_sticks_by_id.entry(event.id).or_default();
                            let before = stick.x;
                            stick.x = value;
                            before
                        }
                        Axis::LeftStickY => {
                            let stick = self.gamepad_sticks_by_id.entry(event.id).or_default();
                            let before = stick.y;
                            stick.y = value;
                            before
                        }
                        Axis::RightStickX => {
                            let stick =
                                self.gamepad_right_sticks_by_id.entry(event.id).or_default();
                            let before = stick.x;
                            stick.x = value;
                            before
                        }
                        Axis::RightStickY => {
                            let stick =
                                self.gamepad_right_sticks_by_id.entry(event.id).or_default();
                            let before = stick.y;
                            stick.y = value;
                            before
                        }
                        Axis::DPadX => {
                            let hat = self.gamepad_hat_axes_by_id.entry(event.id).or_default();
                            let before = hat.x;
                            hat.x = value;
                            before
                        }
                        Axis::DPadY => {
                            let hat = self.gamepad_hat_axes_by_id.entry(event.id).or_default();
                            let before = hat.y;
                            hat.y = value;
                            before
                        }
                        _ => continue,
                    };
                    if let Some(suppressed) = self.captured_gamepad_inputs.get_mut(&event.id) {
                        release_captured_axis(suppressed, axis, value);
                    }
                    if window_focused
                        && self.pending_gamepad_bind.is_none()
                        && self.pending_keyboard_bind.is_none()
                        && (self.is_playing || self.pending_step_frames > 0)
                        && matches!(
                            self.config.input.active_device,
                            InputDeviceMode::Auto | InputDeviceMode::Gamepad
                        )
                        && let Some(name) =
                            gamepad_axis_crossing(axis, before, value, GAMEPAD_STICK_DEADZONE)
                    {
                        // A complete axis press/release within one UI poll must
                        // still reach one emulation frame, including D-pad hats.
                        let queued = self.queued_gamepad_presses.entry(event.id).or_default();
                        queued.insert(name.to_owned());
                        if let Some(dpad) = virtual_dpad_for_axis(name) {
                            queued.insert(dpad.to_owned());
                        }
                    }
                    if let Some(action) = self.pending_gamepad_bind.clone()
                        && window_focused
                        && new_binding.is_none()
                        && let Some(name) = gamepad_axis_binding(axis, before, value)
                    {
                        new_binding = Some((action, name.to_owned()));
                        let captured = self.captured_gamepad_inputs.entry(event.id).or_default();
                        captured.insert(name.to_owned());
                        // Left stick also emulates DPad; never let the
                        // gesture used for capture move Mario afterward.
                        if let Some(dpad) = virtual_dpad_for_axis(name) {
                            captured.insert(dpad.to_owned());
                        }
                    }
                }
                EventType::Disconnected => {
                    // A controller cannot emit release events after unplugging.
                    // Clear only its held buttons, not those on other pads.
                    self.gamepad_buttons_by_id.remove(&event.id);
                    self.queued_gamepad_presses.remove(&event.id);
                    self.gamepad_sticks_by_id.remove(&event.id);
                    self.gamepad_right_sticks_by_id.remove(&event.id);
                    self.gamepad_hat_axes_by_id.remove(&event.id);
                    self.captured_gamepad_inputs.remove(&event.id);
                }
                _ => {}
            }
        }
        let mut by_id = self.gamepad_buttons_by_id.clone();
        for (id, stick) in &self.gamepad_sticks_by_id {
            by_id.entry(*id).or_default().extend(stick.dpad_buttons());
            by_id
                .entry(*id)
                .or_default()
                .extend(stick.axis_bindings("LeftStick"));
        }
        for (id, stick) in &self.gamepad_right_sticks_by_id {
            by_id
                .entry(*id)
                .or_default()
                .extend(stick.axis_bindings("RightStick"));
        }
        for (id, hat) in &self.gamepad_hat_axes_by_id {
            by_id.entry(*id).or_default().extend(hat.dpad_buttons());
        }
        self.gamepad_buttons_down =
            merged_gamepad_buttons_excluding(&by_id, &self.captured_gamepad_inputs);
        if !window_focused {
            // Transient presses from another workspace must not be replayed
            // when the player returns to the emulator window.
            self.queued_gamepad_presses.clear();
        }
        if let Some((action, name)) = new_binding {
            self.config
                .input
                .gamepad_bindings
                .insert(action.clone(), name.clone());
            self.pending_gamepad_bind = None;
            self.persist_config();
            self.status_line = format!("Bound {} to {}.", action_label(&action), name);
        }
    }

    /// Apply the existing persisted audio preferences to the live mixer.
    /// The session mute control is intentionally not saved as mute-on-startup.
    fn sync_audio_volume(&self) {
        if let Some(output) = &self.audio_output {
            output.set_volume(audible_output_volume(
                self.config.audio.enabled,
                self.audio_muted,
                self.config.audio.volume,
            ));
        }
    }

    /// Restart a game in the selected sound/save policy.
    /// The frontend flushes writable SRAM before switching modes. The
    /// read-only diagnostic remains opt-in; silent compatibility is a fallback.
    fn restart_game_with_audio_mode(&mut self, mode: SoundSessionMode, ctx: &egui::Context) {
        let Some(path) = self.session.loaded_rom_path().map(Path::to_path_buf) else {
            self.status_line = "Load a game before testing experimental audio.".to_owned();
            return;
        };
        let old_cheats = self.loaded_game_id.as_ref().and_then(|game_id| {
            self.library_snapshot
                .entries
                .iter()
                .find(|entry| &entry.game_id == game_id)
                .map(|entry| entry.cheats.clone())
        });
        self.clear_audio();
        // Discard the old worker's device/submission counters. A sound test
        // must report *this session's* PCM delivery, not a prior ROM's.
        self.audio_output = None;
        let result = match mode {
            SoundSessionMode::Compatibility => self.session.load_rom(&path),
            SoundSessionMode::Isolated => self.session.load_rom_with_isolated_audio_probe(&path),
            SoundSessionMode::Persistent => self.session.load_rom_with_persistent_audio(&path),
        };
        match result {
            Ok(()) => {
                if let Some(cheats) = old_cheats {
                    self.session.set_active_cheats(&cheats);
                }
                self.playback_error = None;
                self.frame_clock.reset(Instant::now());
                self.frame_performance = FramePerformance::default();
                self.frame_rate.reset();
                self.pending_step_frames = 0;
                self.queued_gamepad_presses.clear();
                self.queued_keyboard_presses.clear();
                self.last_audio_pairs = 0;
                self.last_audio_nonzero_pairs = 0;
                self.audio_nonzero_pairs_total = 0;
                self.is_playing = true;
                self.play_view = true;
                self.refresh_framebuffer(ctx);
                self.status_line = match mode {
                    SoundSessionMode::Compatibility => {
                        "Silent compatibility gameplay restored with existing save progress."
                            .to_owned()
                    }
                    SoundSessionMode::Isolated => {
                        "Isolated sound: SRAM read-only; disk save slots disabled.".to_owned()
                    }
                    SoundSessionMode::Persistent => {
                        "Sound + saves preview: battery SRAM enabled; sound-specific disk slots."
                            .to_owned()
                    }
                };
            }
            Err(error) => {
                self.record_playback_error(&error.to_string());
            }
        }
        self.confirm_experimental_audio = false;
    }

    fn draw_audio_controls(&mut self, ui: &mut egui::Ui) {
        let enabled_changed = ui
            .checkbox(&mut self.config.audio.enabled, "Sound enabled")
            .changed();
        let mute_changed = ui
            .add_enabled_ui(self.config.audio.enabled, |ui| {
                ui.checkbox(&mut self.audio_muted, "Mute")
            })
            .inner
            .changed();
        let volume_changed = ui
            .add_enabled_ui(self.config.audio.enabled, |ui| {
                ui.add(egui::Slider::new(&mut self.config.audio.volume, 0.0..=1.0).text("Volume"))
            })
            .inner
            .changed();
        if enabled_changed || mute_changed || volume_changed {
            self.sync_audio_volume();
            if !self.config.audio.enabled || self.audio_muted || self.config.audio.volume <= 0.0 {
                self.clear_audio();
            }
            if enabled_changed || volume_changed {
                self.persist_config();
            }
        }
        ui.separator();
        let mode = self.session.sound_session_mode();
        match mode {
            SoundSessionMode::Compatibility => {
                ui.label("Compatibility mode: normal game sound is not yet enabled.");
            }
            SoundSessionMode::Isolated => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Sound isolated: SRAM read-only; no disk saves.",
                );
            }
            SoundSessionMode::Persistent => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "SOUND + SAVES PREVIEW: SRAM persists; separate sound-mode disk slots.",
                );
            }
        }
        let isolated = mode != SoundSessionMode::Compatibility;
        ui.label(format!(
            "DSP last frame: {} stereo pairs, {} nonzero; session total nonzero: {}",
            self.last_audio_pairs, self.last_audio_nonzero_pairs, self.audio_nonzero_pairs_total
        ));
        ui.label(format!(
            "OS audio device: {}; mixer frames submitted: {} (nonzero PCM: {})",
            self.audio_output
                .as_ref()
                .map_or("not attempted", |output| output.device_status().label()),
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::submitted_frames),
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::submitted_nonzero_frames),
        ));
        ui.separator();
        ui.label("Live game-audio output check:");
        if ui
            .add_enabled(
                self.config.audio.enabled
                    && !self.audio_muted
                    && self.config.audio.volume > 0.0,
                egui::Button::new("Play 440 Hz through game audio worker"),
            )
            .on_hover_text(
                "A short original test tone uses the SAME Rodio output stream, sink, and volume as the game's PCM. Game audio pauses briefly, not video or saved progress.",
            )
            .clicked()
        {
            if self.audio_output.is_none() {
                match AudioOutput::start() {
                    Ok(output) => {
                        self.audio_output = Some(output);
                        self.sync_audio_volume();
                    }
                    Err(error) => {
                        self.status_line = format!("Could not start audio worker: {error}");
                    }
                }
            }
            if let Some(output) = &self.audio_output {
                self.status_line = if output.enqueue_live_test_tone() {
                    "440 Hz live-worker tone queued. Listen for a short beep.".to_owned()
                } else {
                    "Live-worker audio queue busy; tone not submitted. Try once more.".to_owned()
                };
            }
        }
        ui.label(format!(
            "Live-worker test tones submitted to mixer: {} (hearing is still unverified)",
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::test_tones_submitted)
        ));
        ui.label(format!(
            "Game PCM losses: {} full-queue drops, {} stale/expired worker drops",
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::producer_dropped_frames),
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::worker_dropped_frames),
        ));
        ui.label(format!(
            "Mixer empty before next game PCM: {} (possible playback gaps, not speaker proof)",
            self.audio_output
                .as_ref()
                .map_or(0, AudioOutput::sink_empty_on_pcm),
        ));
        ui.separator();
        ui.label("Independent OS speaker check (separate stream):");
        let speaker_test_running = matches!(&self.speaker_probe, SpeakerProbeState::Running(_));
        if ui
            .add_enabled(
                !speaker_test_running,
                egui::Button::new("Test speakers (440 Hz)"),
            )
            .clicked()
        {
            let (sender, receiver) = mpsc::channel();
            let started = std::thread::Builder::new()
                .name("starbyte-speaker-probe".to_owned())
                .spawn(move || {
                    let result = AudioOutput::test_speakers().map_err(|error| error.to_string());
                    let _ = sender.send(result);
                });
            self.speaker_probe = match started {
                Ok(_) => SpeakerProbeState::Running(receiver),
                Err(error) => SpeakerProbeState::Finished(Err(format!(
                    "Could not start speaker test: {error}"
                ))),
            };
        }
        match &self.speaker_probe {
            SpeakerProbeState::Idle => {}
            SpeakerProbeState::Running(_) => {
                ui.label("Testing the default OS sound device...");
            }
            SpeakerProbeState::Finished(Ok(())) => {
                ui.label("Speaker test completed without an OS error. Only listening can confirm audibility.");
            }
            SpeakerProbeState::Finished(Err(error)) => {
                ui.colored_label(egui::Color32::RED, format!("Speaker test failed: {error}"));
            }
        }
        if isolated {
            if let Some(upload) = self.session.emulator().isolated_apu_upload() {
                ui.label(format!(
                    "Sound program uploaded: {} bytes",
                    upload.bytes_written()
                ));
            }
            if let Some(status) = self.session.emulator().isolated_apu_spc700_status() {
                ui.label(format!(
                    "SPC700: {} instructions, PC ${:04X}, {:?}",
                    status.instructions, status.pc, status.halt_state
                ));
            } else {
                ui.label("SPC700: waiting for the game's sound-program handoff");
            }
        }
        match mode {
            SoundSessionMode::Isolated => {
                ui.label(
                    "Your SRAM was copied read-only at startup. Test changes are never saved.",
                );
                if ui
                    .button("Return to silent compatibility and saves")
                    .clicked()
                {
                    self.restart_game_with_audio_mode(SoundSessionMode::Compatibility, ui.ctx());
                }
            }
            SoundSessionMode::Persistent => {
                ui.label("Experimental sound runs with writable battery SRAM. Disk save slots are separate from the old silent slots.");
                if ui.button("Use silent compatibility fallback").clicked() {
                    self.restart_game_with_audio_mode(SoundSessionMode::Compatibility, ui.ctx());
                }
            }
            SoundSessionMode::Compatibility if self.session.snapshot().has_rom => {
                if !self.confirm_experimental_audio {
                    if ui.button("Try experimental game audio...").clicked() {
                        self.confirm_experimental_audio = true;
                    }
                } else {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Preview: sound can stutter or fail. Both choices restart the loaded ROM. Existing SRAM and old disk slots are preserved.",
                    );
                    if ui.button("Sound + saves (preview)").clicked() {
                        self.restart_game_with_audio_mode(SoundSessionMode::Persistent, ui.ctx());
                    }
                    if ui.button("Read-only sound diagnostic").clicked() {
                        self.restart_game_with_audio_mode(SoundSessionMode::Isolated, ui.ctx());
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_experimental_audio = false;
                    }
                }
            }
            SoundSessionMode::Compatibility => {}
        }
    }

    /// Start the host audio device only after DSP synthesis produces sound.
    /// A missing device must never prevent a game from playing.
    fn enqueue_current_audio(&mut self) {
        let output_volume = audible_output_volume(
            self.config.audio.enabled,
            self.audio_muted,
            self.config.audio.volume,
        );
        if output_volume <= 0.0 {
            return;
        }
        let samples = &self.session.emulator().audio_samples().samples;
        if samples.is_empty() || samples.len() % 2 != 0 {
            return;
        }
        if self.audio_output.is_none() && samples.iter().any(|&sample| sample != 0) {
            match AudioOutput::start() {
                Ok(output) => {
                    output.set_volume(output_volume);
                    self.audio_output = Some(output);
                }
                Err(error) => {
                    warn!("could not start desktop audio thread (video remains playable): {error}");
                }
            }
        }
        if let Some(output) = &self.audio_output {
            output.set_volume(output_volume);
            output.enqueue(samples);
        }
    }

    fn clear_audio(&self) {
        if let Some(output) = &self.audio_output {
            output.clear();
        }
    }

    fn record_playback_error(&mut self, error: &str) {
        self.clear_audio();
        warn!("emulation stopped: {error}");
        let message = format!("Emulation stopped: {error}");
        self.status_line = message.clone();
        self.playback_error = Some(message);
        self.is_playing = false;
        self.pending_step_frames = 0;
        self.frame_rate.reset();
    }

    fn run_frame(&mut self, ctx: &egui::Context) {
        self.session
            .set_controller1(self.effective_controller_state(ctx));
        // Consume each short press once; held keys are sampled again from
        // egui when subsequent emulation frames execute.
        self.queued_keyboard_presses.clear();
        self.queued_gamepad_presses.clear();
        let started = Instant::now();
        match self.session.run_frame_with_timeout(GUI_FRAME_TIMEOUT) {
            Ok(()) => {
                self.frame_performance.record(started.elapsed());
                if self.exit_after_frames.is_some() {
                    self.native_smoke_audio.record(
                        self.session.snapshot().frame,
                        &self.session.emulator().audio_samples().samples,
                    );
                }
                let uploaded = Instant::now();
                self.refresh_framebuffer(ctx);
                self.frame_performance
                    .record_texture_upload(uploaded.elapsed());
                let pcm = &self.session.emulator().audio_samples().samples;
                self.last_audio_pairs = pcm.len() / 2;
                self.last_audio_nonzero_pairs = pcm
                    .chunks_exact(2)
                    .filter(|pair| pair[0] != 0 || pair[1] != 0)
                    .count();
                self.audio_nonzero_pairs_total = self
                    .audio_nonzero_pairs_total
                    .saturating_add(self.last_audio_nonzero_pairs as u64);
                self.enqueue_current_audio();
                let timing = self.frame_performance;
                let now = Instant::now();
                if self.is_playing {
                    self.frame_rate.record(now);
                }
                let emulated_fps = self.frame_rate.fps(now);
                if (frame_spike_reportable(timing)
                    || self.frame_rate.last_gap_ms >= FRAME_PACING_LOG_THRESHOLD_MS
                    || emulated_fps.is_some_and(|fps| fps < 55.0))
                    && now.saturating_duration_since(self.last_frame_spike_log)
                        >= FRAME_SPIKE_LOG_INTERVAL
                {
                    warn!(
                        emulated_fps = emulated_fps.unwrap_or(0.0),
                        frame_gap_ms = self.frame_rate.last_gap_ms,
                        emulation_ms = timing.last_ms,
                        ui_wake_late_ms = timing.last_wake_late_ms,
                        texture_submit_ms = timing.last_texture_ms,
                        active_audio_pairs = self.last_audio_nonzero_pairs,
                        audio_producer_drops = self
                            .audio_output
                            .as_ref()
                            .map_or(0, AudioOutput::producer_dropped_frames),
                        audio_worker_drops = self
                            .audio_output
                            .as_ref()
                            .map_or(0, AudioOutput::worker_dropped_frames),
                        audio_sink_empty_on_pcm = self
                            .audio_output
                            .as_ref()
                            .map_or(0, AudioOutput::sink_empty_on_pcm),
                        "Starbyte frame pacing spike (CPU, UI scheduling, texture, audio queue)"
                    );
                    self.last_frame_spike_log = now;
                }
                self.status_line = self.session.snapshot().status_line();
            }
            Err(error) => self.record_playback_error(&error.to_string()),
        }
    }

    fn remember_recent_rom(&mut self, path: &Path) {
        record_recent_rom(&mut self.config.library.recent_roms, path);
        self.persist_config();
    }

    fn open_local_rom(&mut self, path: &Path, ctx: &egui::Context) {
        let resolved = if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        {
            LibraryService::new(self.config.clone(), self.assets.clone())
                .and_then(|service| service.materialize_single_rom_zip(path))
        } else {
            Ok(path.to_path_buf())
        };
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(error) => {
                self.status_line = format!("Could not open ROM archive: {error}");
                return;
            }
        };
        self.clear_audio();
        self.audio_output = None;
        match load_rom_with_default_sound(&mut self.session, &resolved) {
            Ok(()) => {
                let matching = self
                    .library_snapshot
                    .entries
                    .iter()
                    .find(|entry| {
                        entry.local.as_ref().is_some_and(|local| {
                            local.rom_path.as_path() == path
                                || local.extracted_cache_path.as_deref() == Some(resolved.as_path())
                        })
                    })
                    .cloned();
                self.loaded_game_id = matching.as_ref().map(|entry| entry.game_id.clone());
                if let Some(entry) = matching {
                    let _ = self.session.set_active_cheats(&entry.cheats);
                }
                self.remember_recent_rom(path);
                if self.loaded_game_id.is_none() {
                    // Directly opened ZIPs and ROMs must become library games
                    // before local Cheatarium codes can be matched to them.
                    if let Some(parent) = path.parent()
                        && !self.config.library.rom_dirs.contains(&parent.to_path_buf())
                    {
                        self.config.library.rom_dirs.push(parent.to_path_buf());
                        self.persist_config();
                    }
                    self.queue_job(WorkerCommandKind::RefreshSnapshot);
                    if self
                        .config
                        .cheats
                        .cheatarium_snes_index()
                        .is_some_and(|index| index.is_file())
                    {
                        self.queue_job(WorkerCommandKind::RefreshCheats {
                            target: LibraryTarget {
                                installed_only: true,
                                ..LibraryTarget::default()
                            },
                        });
                    }
                }
                if let Err(error) = self.session.run_frame_with_timeout(GUI_FRAME_TIMEOUT) {
                    self.record_playback_error(&error.to_string());
                    self.play_view = true;
                    self.refresh_framebuffer(ctx);
                    return;
                }
                self.playback_error = None;
                self.refresh_framebuffer(ctx);
                self.is_playing = true;
                self.play_view = true;
                self.pending_step_frames = 0;
                self.frame_performance = FramePerformance::default();
                self.frame_rate.reset();
                self.frame_clock.reset(Instant::now());
                self.last_audio_pairs = 0;
                self.last_audio_nonzero_pairs = 0;
                self.audio_nonzero_pairs_total = 0;
                self.last_sram_flush = Instant::now();
                self.status_line = format!("Loaded {}", path.display());
            }
            Err(error) => {
                self.status_line = format!("Could not load ROM: {error}");
            }
        }
    }

    fn queue_load_entry(&mut self, entry: &LibraryEntry) {
        if entry.installed_status == InstalledStatus::Missing {
            self.status_line = format!("{} is not installed locally.", entry.display_title);
            return;
        }
        info!(game = %entry.display_title, "queueing local game launch");
        self.queue_job(WorkerCommandKind::MaterializeRom {
            entry: entry.clone(),
        });
    }

    fn ensure_cover_texture(
        &mut self,
        ctx: &egui::Context,
        entry: &LibraryEntry,
    ) -> Option<TextureHandle> {
        if let Some(texture) = self.cover_textures.get(&entry.game_id) {
            return Some(texture.clone());
        }
        if self.failed_cover_ids.contains(&entry.game_id) {
            return None;
        }
        let cover = entry.cover.as_ref()?;
        let Ok(reader) = ImageReader::open(&cover.cache_path) else {
            warn!("failed to open cached cover {}", cover.cache_path.display());
            self.failed_cover_ids.insert(entry.game_id.clone());
            return None;
        };
        let Ok(image) = reader.decode() else {
            warn!(
                "failed to decode cached cover {}",
                cover.cache_path.display()
            );
            self.failed_cover_ids.insert(entry.game_id.clone());
            return None;
        };
        let rgba = image.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let texture = ctx.load_texture(
            format!("cover-{}", entry.game_id),
            color_image,
            TextureOptions::LINEAR,
        );
        self.cover_textures
            .insert(entry.game_id.clone(), texture.clone());
        self.failed_cover_ids.remove(&entry.game_id);
        Some(texture)
    }

    fn toggle_cheat(&mut self, game_id: &str, cheat_id: &str, enabled: bool) {
        let enabled_list = self
            .config
            .cheats
            .enabled_by_game
            .entry(game_id.to_owned())
            .or_default();
        if enabled {
            if !enabled_list.iter().any(|value| value == cheat_id) {
                enabled_list.push(cheat_id.to_owned());
            }
        } else {
            enabled_list.retain(|value| value != cheat_id);
        }
        self.update_snapshot_cheat_flags();
        self.persist_config();
        if self.loaded_game_id.as_deref() == Some(game_id) {
            self.sync_loaded_game_cheats();
        }
    }

    fn empty_cheat_explanation(&self) -> &'static str {
        let Some(index) = self.config.cheats.cheatarium_snes_index() else {
            return "Cheatarium is not configured. Settings → Cheats shows the local checkout path.";
        };
        if !index.is_file() {
            return "Cheatarium index missing. Open Settings → Cheats and check the local checkout.";
        }
        if self
            .jobs
            .iter()
            .any(|job| job.label == "Refresh Cheats" && job.state == "failed")
        {
            return "Local cheat refresh failed. Check the Jobs panel for the error.";
        }
        if self
            .jobs
            .iter()
            .any(|job| job.label == "Refresh Cheats" && matches!(job.state, "queued" | "running"))
        {
            return "Local cheat refresh is still in progress. No results displayed yet.";
        }
        "No matching cheats cached for this game. Choose Reload Cheats to retry title matching."
    }

    fn draw_play_cheats_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(420.0);
        ui.heading("Cheats");
        let game = self
            .loaded_game_id
            .as_ref()
            .and_then(|id| {
                self.library_snapshot
                    .entries
                    .iter()
                    .find(|entry| &entry.game_id == id)
            })
            .cloned();
        ui.horizontal(|ui| {
            ui.label("Cheatarium");
            if ui.button("Reload").clicked() {
                self.queue_job(WorkerCommandKind::RefreshCheats {
                    target: LibraryTarget {
                        game_id: self.loaded_game_id.clone(),
                        installed_only: true,
                        ..LibraryTarget::default()
                    },
                });
            }
        });
        let Some(entry) = game else {
            ui.label("Loaded game not yet indexed. Check Library or wait for local scan.");
            return;
        };
        if entry.cheats.is_empty() {
            ui.label(self.empty_cheat_explanation());
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.cheat_search_query)
                .hint_text("Search name, code, type, or source...")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("play_cheat_sort")
                .selected_text(format!("Sort: {}", self.cheat_sort.label()))
                .show_ui(ui, |ui| {
                    for field in [
                        CheatSort::Name,
                        CheatSort::Kind,
                        CheatSort::Source,
                        CheatSort::Code,
                        CheatSort::Status,
                    ] {
                        ui.selectable_value(&mut self.cheat_sort, field, field.label());
                    }
                });
            if ui
                .button(if self.cheat_sort_descending {
                    "Descending"
                } else {
                    "Ascending"
                })
                .clicked()
            {
                self.cheat_sort_descending = !self.cheat_sort_descending;
            }
            egui::ComboBox::from_id_salt("play_cheat_visibility")
                .selected_text(format!("Show: {}", self.cheat_visibility.label()))
                .show_ui(ui, |ui| {
                    for filter in [
                        CheatVisibility::All,
                        CheatVisibility::Enabled,
                        CheatVisibility::Available,
                        CheatVisibility::Unavailable,
                    ] {
                        ui.selectable_value(&mut self.cheat_visibility, filter, filter.label());
                    }
                });
            if ui.small_button("Reset").clicked() {
                self.cheat_search_query.clear();
                self.cheat_sort = CheatSort::default();
                self.cheat_sort_descending = false;
                self.cheat_visibility = CheatVisibility::default();
            }
        });
        let matches = filtered_sorted_cheats(
            &entry.cheats,
            &self.cheat_search_query,
            self.cheat_visibility,
            self.cheat_sort,
            self.cheat_sort_descending,
        );
        let enabled_count = entry.cheats.iter().filter(|cheat| cheat.enabled).count();
        ui.label(format!(
            "{} of {} cheats shown | {} enabled",
            matches.len(),
            entry.cheats.len(),
            enabled_count
        ));
        ui.separator();
        if matches.is_empty() {
            ui.label("No cheats match these filters.");
        } else {
            // Render visible rows only: large Cheatarium lists should not
            // create hundreds of offscreen egui widgets each repaint.
            egui::ScrollArea::vertical()
                .id_salt("play_cheat_results")
                .max_height(350.0)
                .show_rows(ui, 45.0, matches.len(), |ui, range| {
                    for cheat in &matches[range] {
                        let executable = cheat.can_execute();
                        let mut enabled = cheat.enabled;
                        if ui
                            .add_enabled(
                                executable,
                                egui::Checkbox::new(&mut enabled, cheat.name.as_str()),
                            )
                            .changed()
                        {
                            self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                        }
                        ui.label(
                            RichText::new(format!(
                                "{} [{}]{} ",
                                cheat.code,
                                cheat.kind,
                                if executable { "" } else { " | unsupported" }
                            ))
                            .small(),
                        );
                    }
                });
        }
    }

    fn draw_top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, compact: bool) {
        ui.horizontal_wrapped(|ui| {
            if self.play_view {
                ui.heading("Starbyte");
                if ui.button("Library (Esc)").clicked() {
                    self.play_view = false;
                }
                if ui
                    .add_enabled(
                        self.session.snapshot().has_rom,
                        egui::Button::new(if self.is_playing { "Pause" } else { "Play" }),
                    )
                    .clicked()
                {
                    self.is_playing = !self.is_playing;
                    if !self.is_playing {
                        self.clear_audio();
                    }
                    self.frame_rate.reset();
                    self.frame_clock.reset(Instant::now());
                }
                if ui.button("Quick Save (F5)").clicked() {
                    self.quick_save();
                }
                if ui
                    .add_enabled(
                        self.session.has_quick_save(),
                        egui::Button::new("Quick Load (F8)"),
                    )
                    .clicked()
                {
                    self.quick_load(ctx);
                }
                ui.menu_button("Disk Slots", |ui| self.draw_persistent_slots(ui, ctx));
                if ui.button("Screenshot (F12)").clicked() {
                    self.save_screenshot();
                }
                let sound_label = match self.session.sound_session_mode() {
                    SoundSessionMode::Compatibility => "Sound (silent)",
                    SoundSessionMode::Isolated => "Sound (isolated)",
                    SoundSessionMode::Persistent => "Sound (on)",
                };
                ui.menu_button(sound_label, |ui| self.draw_audio_controls(ui));
                if ui.button("Cheats").clicked() {
                    self.show_cheats_window = !self.show_cheats_window;
                }
                if ui
                    .checkbox(&mut self.show_performance_overlay, "Timing")
                    .changed()
                {
                    ctx.request_repaint();
                }
                if ui
                    .checkbox(&mut self.show_input_overlay, "Input Diagnostics")
                    .changed()
                {
                    ctx.request_repaint();
                }
                if ui.button("Fullscreen").clicked() {
                    self.config.video.fullscreen = !self.config.video.fullscreen;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                        self.config.video.fullscreen,
                    ));
                    self.persist_config();
                }
                return;
            }

            ui.heading("Starbyte");
            ui.label("Cozy SNES · experimental");
            ui.separator();
            let has_rom = self.session.snapshot().has_rom;
            if ui
                .add_enabled(has_rom, egui::Button::new("Play View (F9)"))
                .clicked()
            {
                self.play_view = true;
            }
            if ui.button("Open ROM...").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "Super Nintendo ROM or ZIP",
                        &["sfc", "smc", "swc", "fig", "zip"],
                    )
                    .pick_file()
            {
                self.open_local_rom(&path, ctx);
            }
            ui.menu_button("Recent", |ui| {
                let paths = self.config.library.recent_roms.clone();
                if paths.is_empty() {
                    ui.label("No games opened yet");
                }
                for path in paths {
                    let label = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    if ui
                        .add_enabled(path.is_file(), egui::Button::new(label))
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.open_local_rom(&path, ctx);
                        ui.close();
                    }
                }
            });
            let selected_installed = self
                .selected_entry()
                .filter(|entry| entry.installed_status == InstalledStatus::Installed);
            if ui
                .add_enabled(
                    selected_installed.is_some(),
                    egui::Button::new("Play Selected"),
                )
                .on_hover_text("Select an installed game from the library to launch it")
                .clicked()
            {
                if let Some(entry) = selected_installed {
                    self.queue_load_entry(&entry);
                }
            }
            if ui
                .add_enabled(
                    has_rom,
                    egui::Button::new(if self.is_playing { "Pause" } else { "Resume" }),
                )
                .clicked()
            {
                self.is_playing = !self.is_playing;
                if !self.is_playing {
                    self.clear_audio();
                }
                self.frame_rate.reset();
                self.frame_clock.reset(Instant::now());
            }
            if ui
                .add(egui::TextEdit::singleline(&mut self.search_query).hint_text("Search library"))
                .changed()
            {
                self.status_line = format!("Filtered to {} entries.", self.visible_entries().len());
            }

            if ui
                .checkbox(
                    &mut self.config.library.show_installed_only,
                    "Installed only",
                )
                .changed()
            {
                self.persist_config();
                self.status_line = format!("Filtered to {} entries.", self.visible_entries().len());
            }

            let previous_view = self.config.library.active_view;
            egui::ComboBox::from_label("View")
                .selected_text(match self.config.library.active_view {
                    LibraryViewMode::List => "List",
                    LibraryViewMode::Grid => "Grid",
                    LibraryViewMode::Detailed => "Detailed",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.library.active_view,
                        LibraryViewMode::List,
                        "List",
                    );
                    ui.selectable_value(
                        &mut self.config.library.active_view,
                        LibraryViewMode::Grid,
                        "Grid",
                    );
                    ui.selectable_value(
                        &mut self.config.library.active_view,
                        LibraryViewMode::Detailed,
                        "Detailed",
                    );
                });
            if self.config.library.active_view != previous_view {
                self.persist_config();
            }

            if ui.button("Get Covers").clicked() {
                self.queue_job(WorkerCommandKind::RefreshArtwork);
            }
            ui.menu_button("Refresh", |ui| {
                if ui.button("Metadata").clicked() {
                    self.queue_job(WorkerCommandKind::RefreshMetadata);
                    ui.close();
                }
                if ui.button("Covers").clicked() {
                    self.queue_job(WorkerCommandKind::RefreshCovers {
                        target: LibraryTarget::default(),
                    });
                    ui.close();
                }
                if ui.button("Cheats").clicked() {
                    self.queue_job(WorkerCommandKind::RefreshCheats {
                        target: LibraryTarget::default(),
                    });
                    ui.close();
                }
                if ui.button("Everything").clicked() {
                    self.queue_job(WorkerCommandKind::RefreshAll);
                    ui.close();
                }
            });
            if ui
                .checkbox(&mut self.config.prefer_dark_mode, "Night Mode")
                .changed()
            {
                apply_theme(ctx, self.config.prefer_dark_mode);
                self.persist_config();
            }

            ui.separator();
            if compact {
                if ui.button("Settings").clicked() {
                    self.show_compact_settings = !self.show_compact_settings;
                }
                if ui.button("Session").clicked() {
                    self.show_compact_session = !self.show_compact_session;
                }
                if ui.button("Logs").clicked() {
                    self.show_compact_logs = !self.show_compact_logs;
                }
            } else {
                ui.checkbox(&mut self.config.ui.show_left_panel, "Left");
                ui.add_enabled_ui(
                    self.config.library.active_view == LibraryViewMode::List,
                    |ui| {
                        ui.checkbox(&mut self.config.ui.show_details_panel, "Details");
                    },
                );
                ui.checkbox(&mut self.config.ui.show_right_panel, "Session");
                ui.checkbox(&mut self.config.ui.show_log_panel, "Logs");
            }
            if ui.button("Save Layout").clicked() {
                self.persist_config();
            }
        });
    }

    fn draw_settings_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Library");
            ui.label(format!(
                "Showing {} of {} entries",
                self.visible_entries().len(),
                self.library_snapshot.total_count
            ));
            ui.label(self.status_line.as_str());
            if ui.button("Open Scan Report").clicked() {
                let report = self
                    .cache_root
                    .join("manifests")
                    .join("rom-scan-report.json");
                if let Err(error) = open_path(&report) {
                    self.status_line =
                        format!("Could not open report {}: {error}", report.display());
                }
            }
            ui.separator();

            ui.label("ROM Directories");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.rom_dir_input);
                if ui.button("Add").clicked() {
                    let value = self.rom_dir_input.trim();
                    let path = if let Some(suffix) = value.strip_prefix("~/") {
                        std::env::var_os("HOME")
                            .map(|home| PathBuf::from(home).join(suffix))
                            .unwrap_or_else(|| PathBuf::from(value))
                    } else {
                        PathBuf::from(value)
                    };
                    if !value.is_empty()
                        && path.is_dir()
                        && !self.config.library.rom_dirs.contains(&path)
                    {
                        self.config.library.rom_dirs.push(path);
                        self.rom_dir_input.clear();
                        self.persist_config();
                        self.queue_job(WorkerCommandKind::RefreshSnapshot);
                    } else if !value.is_empty() && !path.is_dir() {
                        self.status_line =
                            format!("ROM directory does not exist: {}", path.display());
                    }
                }
                if ui.button("Browse").clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                    && !self.config.library.rom_dirs.contains(&path)
                {
                    self.config.library.rom_dirs.push(path);
                    self.persist_config();
                    self.queue_job(WorkerCommandKind::RefreshSnapshot);
                }
            });

            let mut remove_index = None;
            for (index, rom_dir) in self.config.library.rom_dirs.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(rom_dir.display().to_string());
                    if ui.button("Remove").clicked() {
                        remove_index = Some(index);
                    }
                });
            }
            if let Some(index) = remove_index {
                self.config.library.rom_dirs.remove(index);
                self.persist_config();
                self.queue_job(WorkerCommandKind::RefreshSnapshot);
            }

            ui.separator();
            egui::CollapsingHeader::new("Audio")
                .default_open(true)
                .show(ui, |ui| {
                    self.draw_audio_controls(ui);
                    if ui
                        .checkbox(&mut self.config.audio.mute_on_startup, "Mute on startup")
                        .changed()
                    {
                        self.persist_config();
                    }
                    ui.label(
                        "Emulated DSP: 32 kHz stereo; the OS chooses the speaker output rate.",
                    );
                });

            egui::CollapsingHeader::new("Video")
                .default_open(true)
                .show(ui, |ui| {
                    let video = &mut self.config.video;
                    let mut changed = false;
                    let fullscreen_changed =
                        ui.checkbox(&mut video.fullscreen, "Fullscreen").changed();
                    if fullscreen_changed {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(video.fullscreen));
                    }
                    changed |= fullscreen_changed;
                    changed |= ui
                        .checkbox(&mut video.integer_scale, "Integer scale")
                        .changed();
                    changed |= ui.checkbox(&mut video.vsync, "VSync").changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut video.scale, 1..=6).text("Scale"))
                        .changed();
                    if changed {
                        self.persist_config();
                    }
                });

            egui::CollapsingHeader::new("Input")
                .default_open(false)
                .show(ui, |ui| {
                    self.draw_input_settings(ui);
                });

            egui::CollapsingHeader::new("Cheats")
                .default_open(false)
                .show(ui, |ui| {
                    if ui
                        .checkbox(
                            &mut self.config.cheats.show_cheat_badges,
                            "Show cheat badges",
                        )
                        .changed()
                    {
                        self.persist_config();
                    }
                    ui.separator();
                    ui.label("Local Cheatarium checkout");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.cheatarium_root_input);
                        if ui.button("Save Path").clicked() {
                            let root = self.cheatarium_root_input.trim();
                            self.config.cheats.cheatarium_root =
                                (!root.is_empty()).then(|| PathBuf::from(root));
                            self.persist_config();
                            self.status_line = "Cheatarium source path saved.".to_owned();
                            if self
                                .config
                                .cheats
                                .cheatarium_snes_index()
                                .is_some_and(|index| index.is_file())
                            {
                                self.queue_job(WorkerCommandKind::RefreshCheats {
                                    target: LibraryTarget::default(),
                                });
                            }
                        }
                        if ui.button("Browse").clicked()
                            && let Some(root) = rfd::FileDialog::new().pick_folder()
                        {
                            self.cheatarium_root_input = root.display().to_string();
                            self.config.cheats.cheatarium_root = Some(root);
                            self.persist_config();
                            self.status_line = "Cheatarium source path saved.".to_owned();
                            if self
                                .config
                                .cheats
                                .cheatarium_snes_index()
                                .is_some_and(|index| index.is_file())
                            {
                                self.queue_job(WorkerCommandKind::RefreshCheats {
                                    target: LibraryTarget::default(),
                                });
                            }
                        }
                    });
                    if let Some(path) = self.config.cheats.cheatarium_snes_index() {
                        ui.label(format!("SNES index: {}", path.display()));
                        if path.is_file() {
                            ui.label(
                                "Local SNES index found. Cheat candidates refresh on startup.",
                            );
                            if ui.button("Reload local cheats").clicked() {
                                self.queue_job(WorkerCommandKind::RefreshCheats {
                                    target: LibraryTarget {
                                        installed_only: true,
                                        ..LibraryTarget::default()
                                    },
                                });
                            }
                        } else {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "Cheatarium index not found. Check the checkout path above.",
                            );
                        }
                    } else {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Cheatarium source not configured. Select its local checkout folder.",
                        );
                    }
                });

            egui::CollapsingHeader::new("Advanced / Cache")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(format!("Cache Root: {}", self.cache_root.display()));
                    let advanced = &mut self.config.advanced;
                    let mut changed = false;
                    changed |= ui
                        .checkbox(&mut advanced.show_missing_games, "Show metadata-only games")
                        .changed();
                    changed |= ui
                        .checkbox(&mut advanced.refresh_on_startup, "Refresh on startup")
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut advanced.providers.enable_network,
                            "Enable network providers",
                        )
                        .changed();
                    changed |= ui
                        .text_edit_singleline(&mut self.config.log_filter)
                        .changed();
                    if changed {
                        self.persist_config();
                        self.queue_job(WorkerCommandKind::RefreshSnapshot);
                    }
                });

            ui.separator();
            ui.heading("Jobs");
            for job in self.jobs.iter().rev().take(8) {
                ui.label(format!("[{}] {}: {}", job.state, job.label, job.detail));
            }
        });
    }

    fn draw_details_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Details");
        let Some(entry) = self.selected_entry() else {
            ui.label("Select a game to see details.");
            return;
        };

        ui.horizontal_wrapped(|ui| {
            if ui.button("Play").clicked() {
                self.queue_load_entry(&entry);
            }
            if ui.button("Properties").clicked() {
                self.show_properties = true;
            }
            if ui.button("Refresh Covers").clicked() {
                self.queue_job(WorkerCommandKind::RefreshCovers {
                    target: LibraryTarget {
                        game_id: Some(entry.game_id.clone()),
                        ..LibraryTarget::default()
                    },
                });
            }
            if ui.button("Refresh Cheats").clicked() {
                self.queue_job(WorkerCommandKind::RefreshCheats {
                    target: LibraryTarget {
                        game_id: Some(entry.game_id.clone()),
                        ..LibraryTarget::default()
                    },
                });
            }
        });

        ui.separator();
        if let Some(texture) = self.ensure_cover_texture(ctx, &entry) {
            let size = fit_size(texture.size_vec2(), Vec2::new(ui.available_width(), 260.0));
            ui.add(egui::Image::new((texture.id(), size)));
        } else {
            ui.label("No cached cover.");
        }
        ui.separator();
        ui.heading(entry.display_title.as_str());
        ui.label(match entry.installed_status {
            InstalledStatus::Installed => "Installed locally",
            InstalledStatus::Missing => "Metadata only",
        });
        if let Some(local) = &entry.local {
            ui.label(format!("Source: {:?}", local.source_kind));
            ui.label(format!("Path: {}", local.rom_path.display()));
            if let Some(member) = &local.archive_member_path {
                ui.label(format!("Archive Member: {member}"));
            }
            if let Some(path) = &local.extracted_cache_path {
                ui.label(format!("Extraction Cache: {}", path.display()));
            }
            ui.label(format!("Mapper: {}", local.mapper));
            ui.label(format!(
                "Coprocessor: {}",
                local.coprocessor.as_deref().unwrap_or("None")
            ));
        }
        if let Some(metadata) = &entry.metadata {
            ui.label(format!("Metadata Source: {}", metadata.source));
            ui.label(format!(
                "Has cheat files: {}",
                if metadata.has_cheat_files {
                    "yes"
                } else {
                    "no"
                }
            ));
        }

        ui.separator();
        ui.heading("Cheats");
        if self.loaded_game_id.as_deref() == Some(entry.game_id.as_str()) {
            ui.label(format!(
                "Game Genie ROM-read matches: {} (bus observations, not proof of gameplay effect)",
                self.session.rom_cheat_read_hits()
            ));
        }
        if entry.cheats.is_empty() {
            ui.label(self.empty_cheat_explanation());
        } else {
            ui.label("Cheatarium matches are title candidates, not verified for this ROM.");
            for cheat in entry.cheats {
                let mut enabled = cheat.enabled;
                if ui
                    .add_enabled(
                        cheat.can_execute(),
                        egui::Checkbox::new(&mut enabled, cheat.name.as_str()),
                    )
                    .changed()
                {
                    self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                }
                ui.label(
                    RichText::new(format!("{} [{}] {}", cheat.code, cheat.kind, cheat.source))
                        .small(),
                );
                if !cheat.can_execute() {
                    ui.label(RichText::new("Read-only: unsupported code format.").small());
                }
            }
        }
    }

    fn draw_input_settings(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("Active Device")
            .selected_text(match self.config.input.active_device {
                InputDeviceMode::Auto => "Automatic (Keyboard + Gamepad)",
                InputDeviceMode::Keyboard => "Keyboard",
                InputDeviceMode::Gamepad => "Gamepad",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.input.active_device,
                    InputDeviceMode::Auto,
                    "Automatic (Keyboard + Gamepad)",
                );
                ui.selectable_value(
                    &mut self.config.input.active_device,
                    InputDeviceMode::Keyboard,
                    "Keyboard",
                );
                ui.selectable_value(
                    &mut self.config.input.active_device,
                    InputDeviceMode::Gamepad,
                    "Gamepad",
                );
            });

        ui.horizontal(|ui| {
            if ui.button("Save Input").clicked() {
                self.persist_config();
            }
            if ui.button("Reset Pad Defaults").clicked() {
                self.config.input.reset_gamepad_bindings();
                self.pending_gamepad_bind = None;
                self.queued_gamepad_presses.clear();
                self.persist_config();
                self.status_line = "Restored default gamepad bindings.".to_owned();
            }
        });
        ui.separator();

        ui.label("Keyboard Bindings");
        for &action in input_actions() {
            ui.horizontal(|ui| {
                ui.label(action_label(action));
                let current = self
                    .config
                    .input
                    .keyboard_bindings
                    .get(action)
                    .cloned()
                    .unwrap_or_else(|| "Unbound".to_owned());
                ui.label(current);
                if ui.button("Rebind").clicked() {
                    self.pending_keyboard_bind = Some(action.to_string());
                    self.pending_gamepad_bind = None;
                }
            });
        }
        if let Some(action) = &self.pending_keyboard_bind {
            ui.label(format!("Press a key to bind {}...", action_label(action)));
        }

        ui.separator();
        ui.label("Connected Gamepads");
        if let Some(gilrs) = self.gilrs.as_ref() {
            let devices: Vec<_> = gilrs
                .gamepads()
                .filter(|(_, pad)| pad.is_connected())
                .map(|(id, pad)| (id, pad.name().to_owned()))
                .collect();
            if devices.is_empty() {
                ui.label("No connected controller detected by Starbyte.");
            } else {
                for (id, name) in devices {
                    ui.label(name);
                    let left = self
                        .gamepad_sticks_by_id
                        .get(&id)
                        .copied()
                        .unwrap_or_default();
                    let right = self
                        .gamepad_right_sticks_by_id
                        .get(&id)
                        .copied()
                        .unwrap_or_default();
                    let hat = self
                        .gamepad_hat_axes_by_id
                        .get(&id)
                        .copied()
                        .unwrap_or_default();
                    ui.label(format!(
                        "Left X {:+.2} Y {:+.2} | Right X {:+.2} Y {:+.2} | D-pad X {:+.2} Y {:+.2}",
                        left.x, left.y, right.x, right.y, hat.x, hat.y
                    ));
                }
            }
        } else {
            ui.label("Gamepad input backend could not initialize.");
        }
        if self.gamepad_buttons_down.is_empty() {
            ui.label("Live buttons: none held");
        } else {
            ui.label(format!(
                "Live buttons: {}",
                self.gamepad_buttons_down
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        ui.label(format!(
            "Mapped SNES buttons: {}",
            pressed_button_labels(self.effective_controller_state(ui.ctx()))
        ));
        ui.label("Choose Rebind Pad, then press a button or push a stick direction fully.");
        ui.label("Move an already-held stick back to center before binding its direction.");
        ui.separator();
        ui.label("Gamepad Bindings");
        for &action in input_actions() {
            ui.horizontal(|ui| {
                ui.label(action_label(action));
                let current = self
                    .config
                    .input
                    .gamepad_bindings
                    .get(action)
                    .cloned()
                    .unwrap_or_else(|| "Unbound".to_owned());
                ui.label(current);
                if ui.button("Rebind Pad").clicked() {
                    self.pending_gamepad_bind = Some(action.to_string());
                    self.pending_keyboard_bind = None;
                    self.queued_gamepad_presses.clear();
                }
                if ui.button("Unbind").clicked() {
                    self.config.input.gamepad_bindings.remove(action);
                    self.pending_gamepad_bind = None;
                    self.queued_gamepad_presses.clear();
                    self.persist_config();
                    self.status_line =
                        format!("Unbound {} from the gamepad.", action_label(action));
                }
            });
        }
        if let Some(action) = self.pending_gamepad_bind.clone() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Press a button or move a stick to bind {}...",
                    action_label(&action)
                ));
                if ui.button("Cancel Rebind").clicked() {
                    self.pending_gamepad_bind = None;
                    self.status_line = "Canceled gamepad rebinding.".to_owned();
                }
            });
        }
        for (physical, actions) in shared_gamepad_bindings(&self.config.input.gamepad_bindings) {
            ui.label(format!(
                "Shared input {physical}: {} will activate together.",
                actions.join(", ")
            ));
        }
    }

    fn quick_save(&mut self) {
        match self.session.quick_save() {
            Ok(()) => self.status_line = "Quick save stored for this session (F5).".to_owned(),
            Err(error) => self.status_line = error.to_string(),
        }
    }

    fn quick_load(&mut self, ctx: &egui::Context) {
        match self.session.quick_load() {
            Ok(()) => {
                self.clear_audio();
                self.refresh_framebuffer(ctx);
                self.frame_clock.reset(Instant::now());
                self.frame_rate.reset();
                self.status_line = "Quick save restored (F8).".to_owned();
            }
            Err(error) => self.status_line = error.to_string(),
        }
    }

    fn save_persistent_slot(&mut self, slot: u8) {
        match self.session.save_state_slot(slot) {
            Ok(_) => self.status_line = format!("Saved game to disk slot {slot}."),
            Err(error) => self.status_line = error.to_string(),
        }
    }

    fn load_persistent_slot(&mut self, slot: u8, ctx: &egui::Context) {
        match self.session.load_state_slot(slot) {
            Ok(()) => {
                self.clear_audio();
                self.refresh_framebuffer(ctx);
                self.frame_clock.reset(Instant::now());
                self.frame_rate.reset();
                self.status_line = format!("Restored disk slot {slot}.");
            }
            Err(error) => self.status_line = error.to_string(),
        }
    }

    fn save_screenshot(&mut self) {
        let snapshot = self.session.snapshot();
        if !snapshot.has_rom {
            self.status_line = "Load a game before taking a screenshot.".to_owned();
            return;
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self
            .assets
            .screenshot_root()
            .join(format!("starbyte-{timestamp}-frame-{}.png", snapshot.frame));
        match write_png_screenshot(
            &path,
            self.session.framebuffer_rgba(),
            snapshot.framebuffer_width,
            snapshot.framebuffer_height,
        ) {
            Ok(()) => self.status_line = format!("Screenshot saved to {}", path.display()),
            Err(error) => {
                warn!("could not save screenshot: {error}");
                self.status_line = format!("Screenshot failed: {error}");
            }
        }
    }

    fn draw_persistent_slots(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.session.sound_session_mode() {
            SoundSessionMode::Compatibility => {
                ui.label("Compatibility slots (your existing game states)");
            }
            SoundSessionMode::Persistent => {
                ui.label("Sound-mode slots (separate from existing compatibility slots)");
            }
            SoundSessionMode::Isolated => {
                ui.label("Isolated audio: disk save/load is disabled. Your slots are untouched.");
                return;
            }
        }
        for slot in 1..=3 {
            ui.horizontal(|ui| {
                ui.label(format!("Slot {slot}"));
                if ui.button("Save").clicked() {
                    self.save_persistent_slot(slot);
                }
                if ui
                    .add_enabled(self.session.has_state_slot(slot), egui::Button::new("Load"))
                    .clicked()
                {
                    self.load_persistent_slot(slot, ctx);
                }
            });
        }
    }

    fn draw_play_view(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
        if let Some(texture) = &self.framebuffer_texture {
            let image_size = fit_game_size(
                texture.size_vec2(),
                rect.size(),
                self.config.video.integer_scale,
            );
            let image_rect = egui::Rect::from_center_size(rect.center(), image_size);
            ui.put(image_rect, egui::Image::new((texture.id(), image_size)));
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("The game framebuffer will appear after a frame has been rendered.");
            });
        }
        if let Some(error) = &self.playback_error {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{error}\nPress Escape to return to your library.\nSee Logs for details."),
                egui::TextStyle::Heading.resolve(ui.style()),
                egui::Color32::LIGHT_RED,
            );
        }
        if self.show_input_overlay {
            let focused = ui.ctx().input(|input| input.focused);
            let (host, latched, busy) = self.session.emulator().joypad_status();
            let physical = if self.gamepad_buttons_down.is_empty() {
                "none".to_owned()
            } else {
                self.gamepad_buttons_down
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let label = format!(
                "SNES mapped: {}\nPhysical: {}\nHost bits: {host:04X} | game latched: {latched:04X}{}{}",
                pressed_button_labels(self.effective_controller_state(ui.ctx())),
                physical,
                if busy { " (reading)" } else { "" },
                if focused {
                    ""
                } else {
                    "\nWindow unfocused: click Starbyte"
                }
            );
            ui.painter().text(
                rect.left_top() + egui::vec2(12.0, 12.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::TextStyle::Monospace.resolve(ui.style()),
                egui::Color32::LIGHT_YELLOW,
            );
        }
        if self.show_performance_overlay && self.frame_performance.samples > 0 {
            let stats = self.frame_performance;
            let fps = self
                .frame_rate
                .fps(Instant::now())
                .map_or_else(|| "--".to_owned(), |fps| format!("{fps:.1}"));
            let label = format!(
                "Emu FPS {fps} / 60.1 (2s) | 40ms+ frame gaps: {} | last gap {:.1} / peak {:.1} ms\nEmulation {:.1} ms avg / {:.1} ms peak | {} slow frames\nUI late {:.1} ms / peak {:.1} ms ({} missed frames)\nTexture CPU {:.1} ms / peak {:.1} ms",
                self.frame_rate.hitches,
                self.frame_rate.last_gap_ms,
                self.frame_rate.peak_gap_ms,
                stats.avg_ms,
                stats.peak_ms,
                stats.slow_frames,
                stats.last_wake_late_ms,
                stats.peak_wake_late_ms,
                stats.wakes_over_budget,
                stats.last_texture_ms,
                stats.peak_texture_ms,
            );
            let gap_count = self
                .audio_output
                .as_ref()
                .map_or(0, AudioOutput::sink_empty_on_pcm);
            let label = format!("{label}\nMixer empty before PCM: {gap_count} possible gaps");
            ui.painter().text(
                rect.left_bottom() + egui::vec2(12.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::TextStyle::Monospace.resolve(ui.style()),
                egui::Color32::LIGHT_GREEN,
            );
        }
    }

    fn draw_session_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Session");
        let snapshot = self.session.snapshot();
        if let Some(path) = &snapshot.rom_path {
            ui.label(path.display().to_string());
        } else {
            ui.label("No ROM selected");
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(snapshot.has_rom, egui::Button::new("Quick Save (F5)"))
                .clicked()
            {
                self.quick_save();
            }
            if ui
                .add_enabled(
                    self.session.has_quick_save(),
                    egui::Button::new("Quick Load (F8)"),
                )
                .clicked()
            {
                self.quick_load(ctx);
            }
        });
        ui.collapsing("Disk save slots (1–3)", |ui| {
            self.draw_persistent_slots(ui, ctx);
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.is_playing && snapshot.has_rom && self.pending_step_frames == 0,
                    egui::Button::new("Step Frame"),
                )
                .clicked()
            {
                self.pending_step_frames = 1;
                ctx.request_repaint();
            }
            if ui
                .add_enabled(
                    !self.is_playing && snapshot.has_rom && self.pending_step_frames == 0,
                    egui::Button::new("Step 60 Frames"),
                )
                .clicked()
            {
                // A sixty-frame synchronous call blocks all Wayland events.
                // One frame is instead processed per UI tick, with Cancel.
                self.pending_step_frames = 60;
                ctx.request_repaint();
            }
            if self.pending_step_frames > 0 {
                ui.label(format!("{} frames remaining", self.pending_step_frames));
                if ui.button("Cancel").clicked() {
                    self.pending_step_frames = 0;
                }
            }
        });
        if self.frame_performance.samples > 0 {
            let fps = self
                .frame_rate
                .fps(Instant::now())
                .map_or_else(|| "--".to_owned(), |fps| format!("{fps:.1}"));
            ui.label(format!(
                "Emulated FPS: {fps} / 60.1; 40ms+ frame gaps: {}; peak {:.1} ms",
                self.frame_rate.hitches, self.frame_rate.peak_gap_ms
            ));
            ui.label(format!(
                "Emulation: {:.1} ms/frame avg; {:.1} ms peak; {} of {} frames over 16.7 ms",
                self.frame_performance.avg_ms,
                self.frame_performance.peak_ms,
                self.frame_performance.slow_frames,
                self.frame_performance.samples,
            ));
            ui.label(format!(
                "Late UI wake: {:.1} ms last, {:.1} ms peak; {} full-frame misses",
                self.frame_performance.last_wake_late_ms,
                self.frame_performance.peak_wake_late_ms,
                self.frame_performance.wakes_over_budget
            ));
            ui.label(format!(
                "CPU framebuffer/texture submission: {:.1} ms last, {:.1} ms peak",
                self.frame_performance.last_texture_ms, self.frame_performance.peak_texture_ms
            ));
        }
        ui.separator();
        if let Some(texture) = &self.framebuffer_texture {
            let available = ui.available_size();
            let size = fit_size(texture.size_vec2(), available);
            ui.add(egui::Image::new((texture.id(), size)));
        } else {
            ui.label("Load and run a game to populate the framebuffer preview.");
        }
    }

    fn draw_library_browser(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let entries = self.visible_entries();
        match self.config.library.active_view {
            LibraryViewMode::List => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for entry in entries {
                        self.draw_list_entry(ui, ctx, &entry);
                        ui.separator();
                    }
                });
            }
            LibraryViewMode::Grid => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for entry in entries {
                            self.draw_grid_entry(ui, ctx, &entry);
                        }
                    });
                });
            }
            LibraryViewMode::Detailed => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for entry in entries {
                        self.draw_detailed_entry(ui, ctx, &entry);
                        ui.separator();
                    }
                });
            }
        }
    }

    fn draw_list_entry(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, entry: &LibraryEntry) {
        let status = match entry.installed_status {
            InstalledStatus::Installed => "Present",
            InstalledStatus::Missing => "Missing",
        };
        let response = ui.selectable_label(
            self.selected_game_id.as_deref() == Some(entry.game_id.as_str()),
            format!(
                "{}  |  {}  |  Cheats {}",
                entry.display_title,
                status,
                entry.cheats.len()
            ),
        );
        if response.clicked() {
            self.selected_game_id = Some(entry.game_id.clone());
        }
        if response.double_clicked() {
            self.queue_load_entry(entry);
        }
        response.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
    }

    fn draw_grid_entry(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, entry: &LibraryEntry) {
        const CARD_WIDTH: f32 = 196.0;
        const CARD_HEIGHT: f32 = 320.0;
        const COVER_BOX: Vec2 = Vec2::new(164.0, 190.0);

        ui.allocate_ui_with_layout(
            Vec2::new(CARD_WIDTH, CARD_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let selected = self.selected_game_id.as_deref() == Some(entry.game_id.as_str());
                let mut frame = egui::Frame::group(ui.style());
                if selected {
                    let color = ui.visuals().selection.stroke.color;
                    frame = frame.stroke(egui::Stroke::new(2.0, color));
                }
                let inner = frame.show(ui, |ui| {
                    ui.set_min_size(Vec2::new(CARD_WIDTH - 12.0, CARD_HEIGHT - 12.0));
                    ui.vertical_centered(|ui| {
                        ui.allocate_ui_with_layout(
                            COVER_BOX,
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                if let Some(texture) = self.ensure_cover_texture(ctx, entry) {
                                    let size = fit_size(texture.size_vec2(), COVER_BOX);
                                    ui.add(egui::Image::new((texture.id(), size)));
                                } else {
                                    ui.centered_and_justified(|ui| {
                                        ui.label(RichText::new("No cover").small());
                                    });
                                }
                            },
                        );
                    });
                    ui.add_space(6.0);
                    let title = abbreviate_title(&entry.display_title, 28);
                    let title_response = ui.add_sized(
                        [CARD_WIDTH - 24.0, 36.0],
                        egui::Label::new(RichText::new(title).size(13.0).strong()).wrap(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(match entry.installed_status {
                            InstalledStatus::Installed => "Installed",
                            InstalledStatus::Missing => "Missing",
                        })
                        .size(12.0),
                    );
                    ui.label(RichText::new(format!("Cheats {}", entry.cheats.len())).size(11.0));
                    if entry.installed_status == InstalledStatus::Installed
                        && ui.small_button("Play game").clicked()
                    {
                        self.selected_game_id = Some(entry.game_id.clone());
                        self.queue_load_entry(entry);
                    }

                    if title_response.clicked() {
                        self.selected_game_id = Some(entry.game_id.clone());
                    }
                    if title_response.double_clicked() {
                        self.queue_load_entry(entry);
                    }
                });

                let response = inner.response.interact(egui::Sense::click());
                if response.clicked() {
                    self.selected_game_id = Some(entry.game_id.clone());
                }
                if response.double_clicked() {
                    self.queue_load_entry(entry);
                }
                response.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
            },
        );
    }

    fn draw_detailed_entry(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        entry: &LibraryEntry,
    ) {
        let response = ui.group(|ui| {
            ui.horizontal(|ui| {
                if let Some(texture) = self.ensure_cover_texture(ctx, entry) {
                    let size = fit_size(texture.size_vec2(), Vec2::new(96.0, 96.0));
                    ui.add(egui::Image::new((texture.id(), size)));
                }
                ui.vertical(|ui| {
                    ui.heading(entry.display_title.as_str());
                    ui.label(match entry.installed_status {
                        InstalledStatus::Installed => "Installed locally",
                        InstalledStatus::Missing => "Metadata only",
                    });
                    if let Some(local) = &entry.local {
                        ui.label(format!("Mapper: {}", local.mapper));
                        ui.label(format!("Region: {}", local.region));
                        if let Some(member) = &local.archive_member_path {
                            ui.label(format!("Archive Member: {member}"));
                        }
                    }
                    ui.label(format!("Cheats: {}", entry.cheats.len()));
                });
            });
        });
        if response.response.clicked() {
            self.selected_game_id = Some(entry.game_id.clone());
        }
        response
            .response
            .context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
    }

    fn draw_entry_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
        entry: &LibraryEntry,
    ) {
        if ui.button("Play").clicked() {
            self.queue_load_entry(entry);
            ui.close();
        }
        if ui.button("Properties").clicked() {
            self.selected_game_id = Some(entry.game_id.clone());
            self.show_properties = true;
            ui.close();
        }
        if ui.button("Refresh Metadata").clicked() {
            self.queue_job(WorkerCommandKind::RefreshMetadata);
            ui.close();
        }
        if ui.button("Refresh Covers").clicked() {
            self.queue_job(WorkerCommandKind::RefreshCovers {
                target: LibraryTarget {
                    game_id: Some(entry.game_id.clone()),
                    ..LibraryTarget::default()
                },
            });
            ui.close();
        }
        if ui.button("Refresh Cheats").clicked() {
            self.queue_job(WorkerCommandKind::RefreshCheats {
                target: LibraryTarget {
                    game_id: Some(entry.game_id.clone()),
                    ..LibraryTarget::default()
                },
            });
            ui.close();
        }
        if let Some(local) = &entry.local
            && ui.button("Open ROM Folder").clicked()
        {
            let target = match local.source_kind {
                starbyte_frontend::LocalRomSourceKind::File => {
                    local.rom_path.parent().map(Path::to_path_buf)
                }
                starbyte_frontend::LocalRomSourceKind::ZipArchiveMember => {
                    local.rom_path.parent().map(Path::to_path_buf)
                }
            };
            if let Some(path) = target {
                let _ = open_path(&path);
            }
            ui.close();
        }
    }

    fn draw_properties_window(&mut self, ctx: &egui::Context) {
        let Some(entry) = self.selected_entry() else {
            self.show_properties = false;
            return;
        };
        let mut open = self.show_properties;
        egui::Window::new("Game Properties")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(entry.display_title.as_str());
                ui.label(format!("Game ID: {}", entry.game_id));
                ui.label(match entry.installed_status {
                    InstalledStatus::Installed => "Installed locally",
                    InstalledStatus::Missing => "Metadata only",
                });
                if let Some(local) = &entry.local {
                    ui.label(format!("Source: {:?}", local.source_kind));
                    ui.label(format!("Path: {}", local.rom_path.display()));
                    if let Some(member) = &local.archive_member_path {
                        ui.label(format!("Archive Member: {member}"));
                    }
                }
                if let Some(metadata) = &entry.metadata {
                    ui.label(format!("Metadata Source: {}", metadata.source));
                    ui.label(format!("Fetched At: {}", metadata.fetched_at_unix));
                }
                ui.separator();
                ui.heading("Cheats");
                if self.loaded_game_id.as_deref() == Some(entry.game_id.as_str()) {
                    ui.label(format!(
                        "Game Genie ROM-read matches: {} (bus observations, not proof of gameplay effect)",
                        self.session.rom_cheat_read_hits()
                    ));
                }
                if entry.cheats.is_empty() {
                    ui.label(self.empty_cheat_explanation());
                } else {
                    ui.label("Cheatarium matches are title candidates, not verified for this ROM.");
                    for cheat in entry.cheats {
                        let mut enabled = cheat.enabled;
                        if ui
                            .add_enabled(
                                cheat.can_execute(),
                                egui::Checkbox::new(&mut enabled, cheat.name.as_str()),
                            )
                            .changed()
                        {
                            self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                        }
                        ui.label(
                            RichText::new(format!(
                                "{} [{}] {}",
                                cheat.code, cheat.kind, cheat.source
                            ))
                            .small(),
                        );
                        if !cheat.can_execute() {
                            ui.label(RichText::new("Read-only: unsupported code format.").small());
                        }
                    }
                }
            });
        self.show_properties = open;
    }

    fn draw_logs_contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Logs");
            if ui.button("Open Log Folder").clicked() {
                let _ = open_path(&self.cache_root.join("logs"));
            }
            if ui.button("Copy All").clicked() {
                ctx.copy_text(snapshot_logs(&self.logs).join("\n"));
            }
            if ui.button("Clear View").clicked()
                && let Ok(mut lines) = self.logs.lock()
            {
                lines.clear();
            }
            ui.checkbox(&mut self.config.ui.log_auto_scroll, "Auto-scroll");
        });
        ui.separator();
        let lines = snapshot_logs(&self.logs);
        egui::ScrollArea::vertical()
            .stick_to_bottom(self.config.ui.log_auto_scroll)
            .show(ui, |ui| {
                for line in lines {
                    let color = if line.contains(" ERROR ") || line.contains(" error ") {
                        egui::Color32::LIGHT_RED
                    } else if line.contains(" WARN ") || line.contains(" warn ") {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::LIGHT_GRAY
                    };
                    ui.label(RichText::new(line).monospace().color(color));
                }
            });
    }

    fn draw_log_panel(&mut self, ctx: &egui::Context) {
        if !self.config.ui.show_log_panel {
            return;
        }
        let response = egui::TopBottomPanel::bottom("logs")
            .resizable(true)
            .default_height(self.config.ui.log_panel_height)
            .min_height(120.0)
            .show(ctx, |ui| self.draw_logs_contents(ui, ctx));
        self.config.ui.log_panel_height = response.response.rect.height();
    }
}

impl eframe::App for StarbyteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Theme is installed at construction and when the user changes it.
        // Replacing egui visuals on every repaint invalidates unnecessary UI
        // state during time-critical gameplay.
        let was_recording_binding = self.pending_keyboard_bind.is_some();
        self.capture_keyboard_binding(ctx);
        self.poll_gamepad_events(ctx.input(|input| input.focused));
        self.poll_worker_events(ctx);
        if self.speaker_probe.poll() {
            if let SpeakerProbeState::Finished(Err(error)) = &self.speaker_probe {
                warn!("speaker test failed: {error}");
            }
        } else if matches!(&self.speaker_probe, SpeakerProbeState::Running(_)) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Drop a local ROM from the file manager to open it immediately.
        // Untrusted formats are validated by the normal cartridge loader;
        // ignored non-file drops cannot smuggle raw ROM data into cache.
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        }) {
            self.open_local_rom(&path, ctx);
        }

        // Hotkeys should never fire while editing controls or recording a new binding.
        if !ctx.wants_keyboard_input() && self.pending_keyboard_bind.is_none() {
            if ctx.input(|input| input.key_pressed(egui::Key::F5))
                && self.session.snapshot().has_rom
            {
                self.quick_save();
            }
            if ctx.input(|input| input.key_pressed(egui::Key::F8)) && self.session.has_quick_save()
            {
                self.quick_load(ctx);
            }
            if ctx.input(|input| input.key_pressed(egui::Key::F12))
                && self.session.snapshot().has_rom
            {
                self.save_screenshot();
            }
            if ctx.input(|input| input.key_pressed(egui::Key::F9))
                && self.session.snapshot().has_rom
            {
                self.play_view = !self.play_view;
            }
            if self.play_view && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.play_view = false;
            }
        }

        let available = ctx.available_rect();
        let compact = is_compact_layout(available.width(), available.height());
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| self.draw_top_bar(ui, ctx, compact));
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.play_view && self.session.snapshot().has_rom {
                    let fps = if !self.is_playing {
                        "FPS: paused".to_owned()
                    } else {
                        self.frame_rate
                            .fps(Instant::now())
                            .map_or_else(|| "FPS: --".to_owned(), |value| format!("FPS: {value:.1} / 60.1"))
                    };
                    ui.label(RichText::new(fps).monospace()).on_hover_text(
                        "Completed emulated frames per real second (rolling 2 seconds). This is not GPU or display FPS. Enable Timing for frame-time and hitch details.",
                    );
                    ui.separator();
                    let audio_status = audio_playback_status(
                        self.session.emulator().isolated_apu_driver_probe_enabled(),
                        self.config.audio.enabled,
                        self.audio_muted,
                        self.config.audio.volume,
                        self.audio_nonzero_pairs_total,
                        self.audio_output.as_ref().map(AudioOutput::device_status),
                        self.audio_output
                            .as_ref()
                            .map_or(0, AudioOutput::submitted_nonzero_frames),
                    );
                    ui.label(RichText::new(audio_status).monospace()).on_hover_text(
                        "Sound and saves are enabled for ordinary game loads. Silent compatibility remains an explicit fallback in the Sound menu. Device-open and PCM counters do not prove physical audibility.",
                    );
                    ui.separator();
                }
                ui.label(self.status_line.as_str());
                let active = self
                    .jobs
                    .iter()
                    .filter(|job| job.state == "running" || job.state == "queued")
                    .count();
                if active > 0 {
                    ui.separator();
                    ui.label(format!("{active} background job(s)"));
                }
            });
        });
        if self.play_view {
            egui::CentralPanel::default().show(ctx, |ui| self.draw_play_view(ui));
            if self.show_cheats_window {
                // egui menu popups are transient: clicking a TextEdit inside a
                // menu dismisses the popup before the user can type. Use a
                // persistent floating window with real text-input focus.
                let mut open = self.show_cheats_window;
                egui::Window::new("Game Cheats")
                    .id(egui::Id::new("starbyte_game_cheats_window"))
                    .open(&mut open)
                    .resizable(true)
                    .default_width(500.0)
                    .default_height(490.0)
                    .show(ctx, |ui| self.draw_play_cheats_menu(ui));
                self.show_cheats_window = open;
            }
        } else {
            if !compact {
                self.draw_log_panel(ctx);
            }

            if !compact && self.config.ui.show_left_panel {
                let response = egui::SidePanel::left("settings")
                    .resizable(true)
                    .default_width(self.config.ui.left_panel_width)
                    .min_width(240.0)
                    .show(ctx, |ui| self.draw_settings_panel(ui));
                self.config.ui.left_panel_width = response.response.rect.width();
            }

            if !compact && self.config.ui.show_right_panel {
                let response = egui::SidePanel::right("session")
                    .resizable(true)
                    .default_width(self.config.ui.right_panel_width)
                    .min_width(280.0)
                    .show(ctx, |ui| self.draw_session_panel(ui, ctx));
                self.config.ui.right_panel_width = response.response.rect.width();
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                if !compact
                    && self.config.library.active_view == LibraryViewMode::List
                    && self.config.ui.show_details_panel
                    && ui.available_width() >= 620.0
                {
                    let response = egui::SidePanel::right("details")
                        .resizable(true)
                        .default_width(self.config.ui.details_panel_width)
                        .min_width(260.0)
                        .show_inside(ui, |ui| self.draw_details_panel(ui, ctx));
                    self.config.ui.details_panel_width = response.response.rect.width();
                }

                self.draw_library_browser(ui, ctx);
            });

            if compact && self.show_compact_settings {
                let mut open = self.show_compact_settings;
                egui::Window::new("Settings")
                    .open(&mut open)
                    .default_width(340.0)
                    .resizable(true)
                    .show(ctx, |ui| self.draw_settings_panel(ui));
                self.show_compact_settings = open;
            }
            if compact && self.show_compact_session {
                let mut open = self.show_compact_session;
                egui::Window::new("Session")
                    .open(&mut open)
                    .default_width(340.0)
                    .resizable(true)
                    .show(ctx, |ui| self.draw_session_panel(ui, ctx));
                self.show_compact_session = open;
            }

            if compact && self.show_compact_logs {
                let mut open = self.show_compact_logs;
                egui::Window::new("Logs")
                    .open(&mut open)
                    .default_width(420.0)
                    .default_height(230.0)
                    .resizable(true)
                    .show(ctx, |ui| self.draw_logs_contents(ui, ctx));
                self.show_compact_logs = open;
            }

            if self.show_properties {
                self.draw_properties_window(ctx);
            }
        }

        // Capture edges from every UI update, including repaint callbacks
        // that are too early for another NTSC emulation frame.
        self.capture_gameplay_key_presses(ctx, was_recording_binding);
        // Do not replay short presses after a long pause or a new game.
        if !self.is_playing && self.pending_step_frames == 0 {
            self.queued_gamepad_presses.clear();
        }

        if self.is_playing && self.session.snapshot().has_rom {
            self.pending_step_frames = 0;
            let now = Instant::now();
            let late_by = now.saturating_duration_since(self.frame_clock.next_frame_at);
            if self.frame_clock.take_due_frame(now) {
                self.frame_performance.record_wake_lateness(late_by);
                self.run_frame(ctx);
            }
            if self.is_playing {
                ctx.request_repaint_after(self.frame_clock.time_until_next_frame(Instant::now()));
            }
        } else if self.pending_step_frames > 0 && self.session.snapshot().has_rom {
            let previous_frame = self.session.snapshot().frame;
            self.run_frame(ctx);
            if self.session.snapshot().frame > previous_frame {
                self.pending_step_frames -= 1;
            } else {
                self.pending_step_frames = 0;
            }
            if self.pending_step_frames > 0 {
                ctx.request_repaint();
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if let Some(limit) = self.exit_after_frames
            && self.session.snapshot().frame >= limit
        {
            self.exit_after_frames = None;
            let frame = self.session.snapshot().frame;
            let metrics = self.native_smoke_audio;
            let spc_instructions = self
                .session
                .emulator()
                .isolated_apu_spc700_status()
                .map_or(0, |status| status.instructions);
            let uploaded_spc_bytes = self
                .session
                .emulator()
                .isolated_apu_upload()
                .map_or(0, |upload| upload.bytes_written());
            let audio_worker_spawned = self.audio_output.is_some();
            let audio_device_status = self
                .audio_output
                .as_ref()
                .map_or("worker_not_started", |output| {
                    output.device_status().label()
                });
            let first_audio_frame = metrics
                .first_nonzero_frame
                .map_or_else(|| "none".to_owned(), |frame| frame.to_string());
            info!(
                frame,
                stereo_pairs = metrics.stereo_pairs,
                nonzero_pairs = metrics.nonzero_pairs,
                spc_instructions,
                uploaded_spc_bytes,
                audio_worker_spawned,
                audio_device_status,
                "native GUI smoke completed"
            );
            println!(
                "STARBYTE_NATIVE_SMOKE_OK frame={frame} audio_pairs={} audio_nonzero_pairs={} audio_peak={} audio_first_nonzero_frame={first_audio_frame} spc_instructions={spc_instructions} uploaded_spc_bytes={uploaded_spc_bytes} audio_worker_spawned={audio_worker_spawned} audio_device_status={audio_device_status}",
                metrics.stereo_pairs, metrics.nonzero_pairs, metrics.peak_absolute,
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        if self.session.snapshot().has_rom
            && self.last_sram_flush.elapsed() >= Duration::from_secs(30)
        {
            self.last_sram_flush = Instant::now();
            if let Err(error) = self.session.flush_save_ram() {
                warn!("could not autosave cartridge SRAM: {error}");
            }
        }
    }
}

impl Drop for StarbyteApp {
    fn drop(&mut self) {
        if let Err(error) = self.session.flush_save_ram() {
            warn!("could not persist cartridge SRAM on exit: {error}");
        }
    }
}

#[cfg(test)]
mod playback_tests {
    use std::time::{Duration, Instant};

    use super::{
        CheatSort, CheatVisibility, FRAME_INTERVAL, FrameClock, FramePerformance, FrameRateMeter,
        NativeSmokeAudioMetrics, SpeakerProbeState, Vec2, audible_output_volume, egui,
        filtered_sorted_cheats, fit_game_size, frame_spike_reportable, is_compact_layout,
        load_rom_with_default_sound, merged_gamepad_buttons, pressed_button_labels,
        record_recent_rom, write_png_screenshot,
    };

    #[test]
    fn ordinary_desktop_game_loads_enable_sound_and_keep_save_persistence() {
        use starbyte_core::manifest::AssetConfig;
        use starbyte_frontend::{FrontendSession, SoundSessionMode};

        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("desktop-default.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            ..AssetConfig::default()
        };
        let mut session = FrontendSession::new(assets).unwrap();
        load_rom_with_default_sound(&mut session, &rom).unwrap();
        assert_eq!(session.sound_session_mode(), SoundSessionMode::Persistent);
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert!(
            session
                .state_slot_path(1)
                .unwrap()
                .to_string_lossy()
                .ends_with(".sound.slot1.state.json")
        );

        // The old silent mode is an explicit fallback, not the desktop default.
        session.load_rom(&rom).unwrap();
        assert_eq!(
            session.sound_session_mode(),
            SoundSessionMode::Compatibility
        );
        assert!(!session.emulator().isolated_apu_driver_probe_enabled());
    }

    #[test]
    fn floating_cheat_search_survives_pointer_focus_and_typing() {
        // A transient menu popup is not a durable text editing surface.
        // Exercise pointer focus and character input in the new window.
        fn draw(
            ctx: &egui::Context,
            open: &mut bool,
            search: &mut String,
            input_rect: &mut egui::Rect,
            input: egui::RawInput,
        ) {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Game running in the background");
                });
                egui::Window::new("Game Cheats")
                    .id(egui::Id::new("starbyte_game_cheats_window"))
                    .open(open)
                    .default_pos(egui::pos2(100.0, 100.0))
                    .show(ctx, |ui| {
                        let response = ui.add(egui::TextEdit::singleline(search));
                        *input_rect = response.rect;
                    });
            });
        }

        let ctx = egui::Context::default();
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let mut window_open = true;
        let mut search = String::new();
        let mut input_rect = egui::Rect::NOTHING;
        let input = |events| egui::RawInput {
            screen_rect: Some(screen_rect),
            events,
            ..egui::RawInput::default()
        };
        draw(&ctx, &mut window_open, &mut search, &mut input_rect, input(vec![]));
        assert!(window_open);
        assert!(input_rect.width() > 0.0 && input_rect.height() > 0.0);
        let pos = input_rect.center();
        draw(&ctx, &mut window_open, &mut search, &mut input_rect, input(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]));
        draw(&ctx, &mut window_open, &mut search, &mut input_rect, input(vec![
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]));
        draw(&ctx, &mut window_open, &mut search, &mut input_rect, input(vec![
            egui::Event::Text("coins".to_owned()),
        ]));
        assert!(window_open, "clicking cheat search must not close the window");
        assert_eq!(search, "coins", "search must accept typed characters");
    }

    #[test]
    fn cheat_menu_filters_searches_and_sorts_without_changing_enabled_flags() {
        use starbyte_frontend::CheatEntry;
        let make = |id: &str, name: &str, kind: &str, enabled: bool, source: &str| CheatEntry {
            id: id.to_owned(),
            game_id: "test".to_owned(),
            name: name.to_owned(),
            code: format!("CODE-{id}"),
            source: source.to_owned(),
            kind: kind.to_owned(),
            enabled,
        };
        let cheats = vec![
            make("1", "Extra Lives", "game-genie", false, "Local"),
            make("2", "Infinite Coins", "raw", true, "Local"),
            make("3", "Unlock Level", "raw", false, "Local"),
        ];
        let sorted =
            filtered_sorted_cheats(&cheats, "", CheatVisibility::All, CheatSort::Name, false);
        assert_eq!(
            sorted
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Extra Lives", "Infinite Coins", "Unlock Level"]
        );
        let reversed =
            filtered_sorted_cheats(&cheats, "", CheatVisibility::All, CheatSort::Name, true);
        assert_eq!(reversed[0].name, "Unlock Level");
        let enabled = filtered_sorted_cheats(
            &cheats,
            "",
            CheatVisibility::Enabled,
            CheatSort::Name,
            false,
        );
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "Infinite Coins");
        let search = filtered_sorted_cheats(
            &cheats,
            "LOCAL COINS",
            CheatVisibility::All,
            CheatSort::Kind,
            false,
        );
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].id, "2");
        let code_search = filtered_sorted_cheats(
            &cheats,
            "code-1",
            CheatVisibility::All,
            CheatSort::Code,
            false,
        );
        assert_eq!(code_search[0].name, "Extra Lives");
        assert!(cheats[1].enabled);
    }

    #[test]
    fn speaker_probe_result_is_nonblocking_and_survives_until_retested() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut probe = SpeakerProbeState::Running(receiver);
        assert!(!probe.poll());
        sender.send(Ok(())).unwrap();
        assert!(probe.poll());
        assert!(matches!(probe, SpeakerProbeState::Finished(Ok(()))));
        assert!(!probe.poll());

        let (sender, receiver) = std::sync::mpsc::channel();
        let mut failed = SpeakerProbeState::Running(receiver);
        sender
            .send(Err("no default audio device".to_owned()))
            .unwrap();
        assert!(failed.poll());
        assert!(matches!(failed, SpeakerProbeState::Finished(Err(_))));

        let (sender, receiver) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        drop(sender);
        let mut disconnected = SpeakerProbeState::Running(receiver);
        assert!(disconnected.poll());
        assert!(matches!(disconnected, SpeakerProbeState::Finished(Err(_))));
    }

    #[test]
    fn playback_indicator_distinguishes_silent_shim_from_unverified_pcm() {
        use super::audio_playback_status;
        use crate::audio::AudioDeviceStatus;

        assert_eq!(
            audio_playback_status(
                false,
                true,
                false,
                1.0,
                500,
                Some(AudioDeviceStatus::Opened),
                4
            ),
            "Sound: SILENT (compatibility)"
        );
        assert_eq!(
            audio_playback_status(true, true, false, 1.0, 0, None, 0),
            "Sound: waiting for PCM (experimental)"
        );
        assert_eq!(
            audio_playback_status(
                true,
                true,
                false,
                1.0,
                530,
                Some(AudioDeviceStatus::OpenFailed),
                0
            ),
            "Sound: PCM present; OS device FAILED"
        );
        assert_eq!(
            audio_playback_status(
                true,
                true,
                false,
                1.0,
                530,
                Some(AudioDeviceStatus::Opened),
                1
            ),
            "Sound: nonzero PCM sent (audibility unknown)"
        );
        assert_eq!(
            audio_playback_status(
                true,
                true,
                false,
                1.0,
                530,
                Some(AudioDeviceStatus::Opened),
                0
            ),
            "Sound: OS open; nonzero PCM not submitted"
        );
        assert_eq!(
            audio_playback_status(
                true,
                false,
                false,
                1.0,
                530,
                Some(AudioDeviceStatus::Opened),
                1
            ),
            "Sound: MUTED (experimental)"
        );
        assert_eq!(
            audio_playback_status(
                true,
                true,
                true,
                1.0,
                530,
                Some(AudioDeviceStatus::Opened),
                1
            ),
            "Sound: MUTED (experimental)"
        );
    }

    #[test]
    fn audio_settings_gate_actual_speaker_output_without_muting_dsp_emulation() {
        assert_eq!(audible_output_volume(true, false, 0.5), 0.5);
        assert_eq!(audible_output_volume(false, false, 1.0), 0.0);
        assert_eq!(audible_output_volume(true, true, 1.0), 0.0);
        assert_eq!(audible_output_volume(true, false, 0.0), 0.0);
        assert_eq!(audible_output_volume(true, false, 1.5), 1.0);
        assert_eq!(audible_output_volume(true, false, f32::NAN), 0.0);
    }

    #[test]
    fn native_smoke_audio_metrics_track_silence_then_real_stereo_activity() {
        let mut metrics = NativeSmokeAudioMetrics::default();
        metrics.record(1, &[0, 0, 0, 0]);
        assert_eq!(metrics.stereo_pairs, 2);
        assert_eq!(metrics.nonzero_pairs, 0);
        assert_eq!(metrics.first_nonzero_frame, None);
        metrics.record(2, &[0, -1, 0, 0, 4, 0]);
        metrics.record(3, &[-32_768, 32_767]);
        assert_eq!(metrics.stereo_pairs, 6);
        assert_eq!(metrics.nonzero_pairs, 3);
        assert_eq!(metrics.first_nonzero_frame, Some(2));
        assert_eq!(metrics.peak_absolute, 32_768);
        metrics.record(4, &[0, 0]);
        assert_eq!(metrics.first_nonzero_frame, Some(2));
        assert_eq!(metrics.nonzero_pairs, 3);
    }

    #[test]
    fn frame_rate_meter_reports_wall_clock_fps_and_measured_hitches() {
        let start = Instant::now();
        let mut meter = FrameRateMeter::default();
        assert_eq!(meter.fps(start), None);
        for frame in 0..=60 {
            meter.record(start + Duration::from_millis(frame * 16));
        }
        let measured = meter.fps(start + Duration::from_millis(960)).unwrap();
        assert!((measured - 62.5).abs() < 0.01);
        assert_eq!(meter.hitches, 0);
        assert_eq!(meter.last_gap_ms, 16.0);

        // Wall-clock delay lowers effective FPS even if the emulator's CPU
        // frame time is short. The gap is recorded as a hitch, not a CPU cost.
        meter.record(start + Duration::from_millis(1_160));
        let slower = meter.fps(start + Duration::from_millis(1_160)).unwrap();
        assert!(slower < measured);
        assert_eq!(meter.hitches, 1);
        assert_eq!(meter.peak_gap_ms, 200.0);

        meter.reset();
        assert_eq!(meter.hitches, 0);
        assert_eq!(meter.fps(start + Duration::from_secs(3)), None);
        assert_eq!(meter.completed_at.len(), 0);
    }

    #[test]
    fn frame_rate_meter_expires_old_frames_and_does_not_report_stale_fps() {
        let start = Instant::now();
        let mut meter = FrameRateMeter::default();
        for frame in 0..60 {
            meter.record(start + Duration::from_millis(frame * 16));
        }
        assert!(meter.fps(start + Duration::from_millis(960)).is_some());
        assert_eq!(meter.fps(start + Duration::from_secs(3)), None);
        meter.record(start + Duration::from_secs(3));
        assert_eq!(meter.completed_at.len(), 1);
        assert_eq!(meter.fps(start + Duration::from_secs(3)), None);
    }

    #[test]
    fn frame_pacing_matches_ntsc_master_clock_ratio() {
        let expected_ns = 341_u128 * 262 * 4 * 1_000_000_000 / 21_477_272;
        assert_eq!(FRAME_INTERVAL.as_nanos(), expected_ns);
        assert!(FRAME_INTERVAL < Duration::from_nanos(16_666_667));
    }

    #[test]
    fn clock_limits_work_to_one_frame_per_tick_and_does_not_accumulate_lag() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        assert!(clock.take_due_frame(start));
        assert!(!clock.take_due_frame(start + FRAME_INTERVAL / 2));

        let late = start + Duration::from_secs(2);
        assert!(clock.take_due_frame(late));
        assert!(!clock.take_due_frame(late));
        assert_eq!(clock.time_until_next_frame(late), FRAME_INTERVAL);

        clock.reset(late);
        assert!(clock.take_due_frame(late));
    }

    #[test]
    fn frame_clock_preserves_ntsc_phase_across_small_ui_wake_delays() {
        let start = Instant::now();
        let mut clock = FrameClock::new(start);
        assert!(clock.take_due_frame(start));
        let slightly_late = start + FRAME_INTERVAL + Duration::from_millis(3);
        assert!(clock.take_due_frame(slightly_late));
        assert_eq!(
            clock.time_until_next_frame(slightly_late),
            FRAME_INTERVAL - Duration::from_millis(3)
        );
        assert!(!clock.take_due_frame(slightly_late));
        let original_next_deadline = start + FRAME_INTERVAL + FRAME_INTERVAL;
        assert!(clock.take_due_frame(original_next_deadline));
    }

    #[test]
    fn input_overlay_reports_effective_snes_button_names() {
        assert_eq!(pressed_button_labels(Default::default()), "None");
        let state = starbyte_core::input::ControllerState {
            start: true,
            up: true,
            a: true,
            ..Default::default()
        };
        assert_eq!(pressed_button_labels(state), "Start Up A");
    }

    #[test]
    fn rapid_keyboard_tap_is_not_lost_between_emulation_frames() {
        let ctx = egui::Context::default();
        let key_event = |pressed| egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        let input = egui::RawInput {
            events: vec![key_event(true), key_event(false)],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            let (held, observed) = ctx.input(|input| {
                (
                    input.key_down(egui::Key::B),
                    super::gameplay_key_active(input, egui::Key::B),
                )
            });
            assert!(!held, "the key is released within this UI frame");
            assert!(observed, "the press edge must still reach the game");
        });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            assert!(!ctx.input(|input| super::gameplay_key_active(input, egui::Key::B)));
        });
    }

    #[test]
    fn keyboard_press_survives_a_ui_update_without_emulation_and_is_consumed_once() {
        use super::QueuedKeyboardPresses;

        let mut pending = QueuedKeyboardPresses::default();
        let bindings = std::collections::BTreeMap::from([("b".to_owned(), "B".to_owned())]);
        let ctx = egui::Context::default();
        let key_event = |pressed| egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key_event(true), key_event(false)],
                ..Default::default()
            },
            |ctx| {
                pending.capture(ctx, &bindings, true);
                assert!(pending.contains("b"));
            },
        );
        // A repaint happened, but no game frame ran. The key is already up.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            pending.capture(ctx, &bindings, true);
            assert!(pending.contains("b"));
            assert!(!ctx.input(|input| super::gameplay_key_active(input, egui::Key::B)));
            pending.clear();
            assert!(!pending.contains("b"));
        });
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            pending.capture(ctx, &bindings, true);
            assert!(!pending.contains("b"));
        });
    }

    #[test]
    fn keyboard_focus_or_pause_discards_queued_gameplay_taps() {
        use super::QueuedKeyboardPresses;

        let mut pending = QueuedKeyboardPresses::default();
        let bindings = std::collections::BTreeMap::from([("start".to_owned(), "Enter".to_owned())]);
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::default(),
                }],
                ..Default::default()
            },
            |ctx| {
                pending.capture(ctx, &bindings, true);
                assert!(pending.contains("start"));
                // Text focus, binding capture, pause, or no ROM must clear.
                pending.capture(ctx, &bindings, false);
                assert!(!pending.contains("start"));
            },
        );
    }

    #[test]
    fn automatic_input_preserves_gamepad_during_keyboard_text_focus() {
        use super::controller_input_sources;
        use starbyte_core::manifest::InputDeviceMode;

        assert_eq!(
            controller_input_sources(InputDeviceMode::Auto, false, false),
            (true, true)
        );
        assert_eq!(
            controller_input_sources(InputDeviceMode::Auto, true, false),
            (false, true)
        );
        assert_eq!(
            controller_input_sources(InputDeviceMode::Auto, false, true),
            (false, true)
        );
        assert_eq!(
            controller_input_sources(InputDeviceMode::Keyboard, true, false),
            (false, false)
        );
        assert_eq!(
            controller_input_sources(InputDeviceMode::Keyboard, false, false),
            (true, false)
        );
        assert_eq!(
            controller_input_sources(InputDeviceMode::Gamepad, true, true),
            (false, true)
        );
    }

    #[test]
    fn stick_axis_binding_requires_deliberate_threshold_crossing() {
        use super::{Axis, gamepad_axis_binding};
        assert_eq!(gamepad_axis_binding(Axis::LeftStickX, 0.2, 0.5), None);
        assert_eq!(
            gamepad_axis_binding(Axis::LeftStickX, 0.65, 0.85),
            Some("LeftStickX+")
        );
        assert_eq!(gamepad_axis_binding(Axis::LeftStickX, 0.85, 0.9), None);
        assert_eq!(
            gamepad_axis_binding(Axis::LeftStickY, -0.4, -0.88),
            Some("LeftStickY-")
        );
        assert_eq!(
            gamepad_axis_binding(Axis::RightStickY, 0.1, 0.9),
            Some("RightStickY+")
        );
        assert_eq!(gamepad_axis_binding(Axis::LeftZ, 0.0, 1.0), None);
        assert_eq!(
            gamepad_axis_binding(Axis::DPadX, 0.0, -1.0),
            Some("DPadLeft")
        );
        assert_eq!(gamepad_axis_binding(Axis::DPadY, 0.0, 1.0), Some("DPadUp"));
    }

    #[test]
    fn short_stick_and_hat_taps_cross_gameplay_threshold() {
        use super::{Axis, GAMEPAD_STICK_DEADZONE, gamepad_axis_crossing};
        assert_eq!(
            gamepad_axis_crossing(Axis::DPadX, 0.0, -1.0, GAMEPAD_STICK_DEADZONE),
            Some("DPadLeft")
        );
        assert_eq!(
            gamepad_axis_crossing(Axis::RightStickY, 0.2, 0.6, GAMEPAD_STICK_DEADZONE),
            Some("RightStickY+")
        );
        assert_eq!(
            gamepad_axis_crossing(Axis::LeftStickY, 0.6, 0.7, GAMEPAD_STICK_DEADZONE),
            None
        );
        assert_eq!(
            gamepad_axis_crossing(Axis::LeftStickX, 0.0, 0.2, GAMEPAD_STICK_DEADZONE),
            None
        );
    }

    #[test]
    fn stick_axes_are_bindable_without_sacrificing_virtual_dpad() {
        use super::StickAxes;
        let stick = StickAxes { x: -0.9, y: 0.9 };
        let axes = stick.axis_bindings("LeftStick");
        assert!(axes.contains("LeftStickX-"));
        assert!(axes.contains("LeftStickY+"));
        assert!(!axes.contains("LeftStickX+"));
        let pad = stick.dpad_buttons();
        assert!(pad.contains("DPadLeft"));
        assert!(pad.contains("DPadUp"));
        assert!(StickAxes::default().axis_bindings("RightStick").is_empty());
    }

    #[test]
    fn stale_library_worker_cannot_overwrite_new_controller_or_cheat_path() {
        use starbyte_core::manifest::{InputDeviceMode, RuntimeConfig};

        let queued = RuntimeConfig::default();
        let mut live = queued.clone();
        live.input.active_device = InputDeviceMode::Gamepad;
        live.input
            .gamepad_bindings
            .insert("start".to_owned(), "North".to_owned());
        live.cheats.cheatarium_root = Some(std::path::PathBuf::from("test/cheatarium"));
        live.advanced.providers.last_cheat_refresh_unix = Some(123);

        let mut stale_worker = queued;
        stale_worker.advanced.providers.last_metadata_refresh_unix = Some(200);
        stale_worker.advanced.providers.last_cheat_refresh_unix = Some(100);
        super::merge_worker_refresh_bookkeeping(&mut live, &stale_worker);

        assert_eq!(live.input.active_device, InputDeviceMode::Gamepad);
        assert_eq!(
            live.input.gamepad_bindings.get("start").map(String::as_str),
            Some("North")
        );
        assert_eq!(
            live.cheats.cheatarium_root,
            Some(std::path::PathBuf::from("test/cheatarium"))
        );
        assert_eq!(live.advanced.providers.last_cheat_refresh_unix, Some(123));
        assert_eq!(
            live.advanced.providers.last_metadata_refresh_unix,
            Some(200)
        );
    }

    #[test]
    fn worker_refresh_timestamp_merge_never_reverts_to_older_values() {
        use super::newest_timestamp;
        assert_eq!(newest_timestamp(None, None), None);
        assert_eq!(newest_timestamp(None, Some(10)), Some(10));
        assert_eq!(newest_timestamp(Some(50), None), Some(50));
        assert_eq!(newest_timestamp(Some(50), Some(10)), Some(50));
        assert_eq!(newest_timestamp(Some(10), Some(50)), Some(50));
    }

    #[test]
    fn conflicting_gamepad_bindings_are_reported_without_discarding_user_mapping() {
        use std::collections::BTreeMap;

        use super::shared_gamepad_bindings;
        let mut bindings = BTreeMap::from([
            ("a".to_owned(), "South".to_owned()),
            ("b".to_owned(), "South".to_owned()),
            ("start".to_owned(), "Start".to_owned()),
        ]);
        assert_eq!(
            shared_gamepad_bindings(&bindings),
            vec![("South".to_owned(), vec!["A".to_owned(), "B".to_owned()])]
        );
        bindings.insert("b".to_owned(), "East".to_owned());
        assert!(shared_gamepad_bindings(&bindings).is_empty());
    }

    #[test]
    fn primary_gamepad_stick_drives_cardinal_and_diagonal_snes_directions() {
        use super::StickAxes;
        let neutral = StickAxes { x: 0.15, y: -0.3 }.dpad_buttons();
        assert!(neutral.is_empty());
        let up_right = StickAxes { x: 0.8, y: 0.9 }.dpad_buttons();
        assert!(up_right.contains("DPadUp"));
        assert!(up_right.contains("DPadRight"));
        assert!(!up_right.contains("DPadDown"));
        assert!(!up_right.contains("DPadLeft"));
        let down_left = StickAxes { x: -0.75, y: -1.0 }.dpad_buttons();
        assert!(down_left.contains("DPadDown"));
        assert!(down_left.contains("DPadLeft"));
        assert!(StickAxes::default().dpad_buttons().is_empty());
    }

    #[test]
    fn stick_directions_combine_with_physical_pad_buttons() {
        use super::StickAxes;
        let pads = std::collections::HashMap::from([(
            1_u8,
            std::collections::BTreeSet::from(["South".to_owned(), "DPadUp".to_owned()]),
        )]);
        let mut pressed = merged_gamepad_buttons(&pads);
        pressed.extend(StickAxes { x: -0.8, y: 0.0 }.dpad_buttons());
        assert!(pressed.contains("South"));
        assert!(pressed.contains("DPadUp"));
        assert!(pressed.contains("DPadLeft"));
        assert_eq!(pressed.len(), 3);
    }

    #[test]
    fn brief_gamepad_press_survives_release_and_is_consumed_once() {
        let mut held = std::collections::BTreeSet::new();
        let mut queued = std::collections::HashMap::new();
        held.insert("South".to_owned());
        queued.insert(1_u8, std::collections::BTreeSet::from(["South".to_owned()]));
        held.remove("South");
        assert!(super::gamepad_button_active(&held, &queued, "South"));
        // An emulated frame consumes the transient edge.
        queued.clear();
        assert!(!super::gamepad_button_active(&held, &queued, "South"));
    }

    #[test]
    fn gamepad_disconnect_cancels_only_its_own_queued_press() {
        let held = std::collections::BTreeSet::new();
        let mut queued = std::collections::HashMap::from([
            (1_u8, std::collections::BTreeSet::from(["South".to_owned()])),
            (2_u8, std::collections::BTreeSet::from(["North".to_owned()])),
        ]);
        queued.remove(&1);
        assert!(!super::gamepad_button_active(&held, &queued, "South"));
        assert!(super::gamepad_button_active(&held, &queued, "North"));
        queued.remove(&2);
        assert!(!super::gamepad_button_active(&held, &queued, "North"));
    }

    #[test]
    fn captured_gamepad_button_never_leaks_into_play_until_released() {
        use std::collections::{BTreeSet, HashMap};

        use super::merged_gamepad_buttons_excluding;
        let pads = HashMap::from([
            (1_u8, BTreeSet::from(["South".to_owned()])),
            (2_u8, BTreeSet::from(["North".to_owned()])),
        ]);
        let mut suppressed = HashMap::from([(1_u8, BTreeSet::from(["South".to_owned()]))]);
        let active = merged_gamepad_buttons_excluding(&pads, &suppressed);
        assert!(!active.contains("South"));
        assert!(active.contains("North"));
        // A release from the same pad clears its suppression.
        suppressed.get_mut(&1).unwrap().remove("South");
        assert!(merged_gamepad_buttons_excluding(&pads, &suppressed).contains("South"));
    }

    #[test]
    fn captured_stick_direction_and_virtual_dpad_wait_for_neutral() {
        use std::collections::BTreeSet;

        use super::{Axis, StickAxes, release_captured_axis, virtual_dpad_for_axis};
        let name = "LeftStickX+";
        let dpad = virtual_dpad_for_axis(name).unwrap();
        assert_eq!(dpad, "DPadRight");
        let mut suppressed = BTreeSet::from([name.to_owned(), dpad.to_owned()]);
        let mut raw = StickAxes { x: 0.9, y: 0.0 }.axis_bindings("LeftStick");
        raw.extend(StickAxes { x: 0.9, y: 0.0 }.dpad_buttons());
        assert!(raw.contains(name));
        assert!(raw.contains(dpad));
        assert!(raw.is_superset(&suppressed));
        release_captured_axis(&mut suppressed, Axis::LeftStickX, 0.6);
        assert_eq!(suppressed.len(), 2);
        release_captured_axis(&mut suppressed, Axis::LeftStickX, 0.0);
        assert!(suppressed.is_empty());
        assert!(virtual_dpad_for_axis("RightStickX+").is_none());
        let mut hat_capture = BTreeSet::from(["DPadLeft".to_owned()]);
        release_captured_axis(&mut hat_capture, Axis::DPadX, -1.0);
        assert!(hat_capture.contains("DPadLeft"));
        release_captured_axis(&mut hat_capture, Axis::DPadX, 0.0);
        assert!(hat_capture.is_empty());
    }

    #[test]
    fn unplugged_gamepad_releases_only_its_own_buttons() {
        let mut pads = std::collections::HashMap::new();
        pads.insert(1_u8, std::collections::BTreeSet::from(["South".to_owned()]));
        pads.insert(
            2_u8,
            std::collections::BTreeSet::from(["South".to_owned(), "North".to_owned()]),
        );
        assert_eq!(merged_gamepad_buttons(&pads).len(), 2);
        pads.remove(&1);
        assert_eq!(merged_gamepad_buttons(&pads).len(), 2);
        pads.remove(&2);
        assert!(merged_gamepad_buttons(&pads).is_empty());
    }

    #[test]
    fn recent_games_are_deduplicated_newest_first_and_bounded() {
        let mut recent = Vec::new();
        for index in 0..12 {
            record_recent_rom(
                &mut recent,
                std::path::Path::new(&format!("starbyte-test-game-{index}.sfc")),
            );
        }
        assert_eq!(recent.len(), 8);
        assert_eq!(
            recent[0],
            std::path::PathBuf::from("starbyte-test-game-11.sfc")
        );
        record_recent_rom(
            &mut recent,
            std::path::Path::new("starbyte-test-game-7.sfc"),
        );
        assert_eq!(recent.len(), 8);
        assert_eq!(
            recent[0],
            std::path::PathBuf::from("starbyte-test-game-7.sfc")
        );
        assert_eq!(recent.iter().filter(|path| *path == &recent[0]).count(), 1);
    }

    #[test]
    fn actionable_frame_spike_filter_is_thresholded() {
        let mut stats = FramePerformance::default();
        stats.record(Duration::from_millis(16));
        stats.record_wake_lateness(Duration::from_millis(4));
        stats.record_texture_upload(Duration::from_millis(2));
        assert!(!frame_spike_reportable(stats));
        stats.record_wake_lateness(Duration::from_millis(50));
        assert!(frame_spike_reportable(stats));
        stats.record_wake_lateness(Duration::ZERO);
        stats.record_texture_upload(Duration::from_millis(45));
        assert!(frame_spike_reportable(stats));
        stats.record_texture_upload(Duration::ZERO);
        stats.record(Duration::from_millis(55));
        assert!(frame_spike_reportable(stats));
    }

    #[test]
    fn frame_performance_distinguishes_ui_wake_and_texture_spikes() {
        let mut stats = FramePerformance::default();
        stats.record(Duration::from_millis(8));
        stats.record_wake_lateness(Duration::from_millis(29));
        stats.record_texture_upload(Duration::from_millis(5));
        assert_eq!(stats.slow_frames, 0);
        assert_eq!(stats.wakes_over_budget, 1);
        assert_eq!(stats.peak_wake_late_ms, 29.0);
        assert_eq!(stats.peak_texture_ms, 5.0);
        stats.record_wake_lateness(Duration::from_millis(1));
        stats.record_texture_upload(Duration::from_millis(2));
        assert_eq!(stats.wakes_over_budget, 1);
        assert_eq!(stats.peak_wake_late_ms, 29.0);
        assert_eq!(stats.peak_texture_ms, 5.0);
        assert_eq!(stats.last_wake_late_ms, 1.0);
    }

    #[test]
    fn frame_performance_counts_over_budget_without_accumulating_lag() {
        let mut stats = FramePerformance::default();
        stats.record(Duration::from_millis(10));
        assert_eq!(stats.samples, 1);
        assert_eq!(stats.slow_frames, 0);
        assert_eq!(stats.avg_ms, 10.0);
        stats.record(Duration::from_millis(20));
        assert_eq!(stats.samples, 2);
        assert_eq!(stats.slow_frames, 1);
        assert_eq!(stats.peak_ms, 20.0);
        assert!((stats.avg_ms - 11.0).abs() < f32::EPSILON);
    }

    #[test]
    fn game_view_scales_to_integer_pixels_without_clipping_small_tiles() {
        let source = Vec2::new(256.0, 224.0);
        assert_eq!(
            fit_game_size(source, Vec2::new(800.0, 600.0), true),
            Vec2::new(512.0, 448.0)
        );
        assert_eq!(
            fit_game_size(source, Vec2::new(2560.0, 1440.0), true),
            Vec2::new(1536.0, 1344.0)
        );
        let small = fit_game_size(source, Vec2::new(450.0, 180.0), true);
        assert!(small.x <= 450.0 && small.y <= 180.0);
        let fractional = fit_game_size(source, Vec2::new(800.0, 600.0), false);
        assert!(fractional.x > 512.0);
        assert_eq!(fractional.y, 600.0);
    }

    #[test]
    fn png_screenshot_exports_exact_framebuffer_colors() {
        let dir = std::env::temp_dir().join(format!(
            "starbyte-screenshot-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("frame.png");
        write_png_screenshot(&path, &[248, 0, 0, 255, 0, 0, 248, 255], 2, 1).unwrap();
        let pixels = image::open(&path).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (2, 1));
        assert_eq!(pixels.as_raw(), &[248, 0, 0, 255, 0, 0, 248, 255]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn small_tiled_windows_use_popup_panels() {
        assert!(is_compact_layout(520.0, 900.0));
        assert!(is_compact_layout(959.0, 900.0));
        assert!(is_compact_layout(1200.0, 500.0));
        assert!(!is_compact_layout(960.0, 650.0));
    }
}

/// Persisted volume affects OS playback, not synthesis or WAV diagnostics.
/// Report the *actual* audio pipeline state; frame sample counts alone
/// do not establish working music or that the speaker rendered anything.
fn audio_playback_status(
    experimental: bool,
    enabled: bool,
    muted: bool,
    volume: f32,
    nonzero_pairs_total: u64,
    device: Option<AudioDeviceStatus>,
    submitted_nonzero_frames: u64,
) -> &'static str {
    if !experimental {
        return "Sound: SILENT (compatibility)";
    }
    if !enabled || muted || !volume.is_finite() || volume <= 0.0 {
        return "Sound: MUTED (experimental)";
    }
    if nonzero_pairs_total == 0 {
        return "Sound: waiting for PCM (experimental)";
    }
    match device {
        None => "Sound: PCM present; no audio worker",
        Some(AudioDeviceStatus::NotAttempted) => "Sound: PCM present; opening device",
        Some(AudioDeviceStatus::OpenFailed) => "Sound: PCM present; OS device FAILED",
        Some(AudioDeviceStatus::Opened) if submitted_nonzero_frames == 0 => {
            "Sound: OS open; nonzero PCM not submitted"
        }
        Some(AudioDeviceStatus::Opened) => "Sound: nonzero PCM sent (audibility unknown)",
    }
}

fn audible_output_volume(enabled: bool, muted: bool, volume: f32) -> f32 {
    if !enabled || muted || !volume.is_finite() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

fn pressed_button_labels(state: ControllerState) -> String {
    let buttons = [
        ("B", state.b),
        ("Y", state.y),
        ("Select", state.select),
        ("Start", state.start),
        ("Up", state.up),
        ("Down", state.down),
        ("Left", state.left),
        ("Right", state.right),
        ("A", state.a),
        ("X", state.x),
        ("L", state.l),
        ("R", state.r),
    ];
    let held = buttons
        .into_iter()
        .filter_map(|(name, pressed)| pressed.then_some(name))
        .collect::<Vec<_>>();
    if held.is_empty() {
        "None".to_owned()
    } else {
        held.join(" ")
    }
}

fn gamepad_button_active<K: Eq + std::hash::Hash>(
    held: &BTreeSet<String>,
    queued_by_controller: &HashMap<K, BTreeSet<String>>,
    binding: &str,
) -> bool {
    held.contains(binding)
        || queued_by_controller
            .values()
            .any(|pressed| pressed.contains(binding))
}

fn merged_gamepad_buttons<K: Eq + std::hash::Hash>(
    by_controller: &HashMap<K, BTreeSet<String>>,
) -> BTreeSet<String> {
    by_controller
        .values()
        .flat_map(|buttons| buttons.iter().cloned())
        .collect()
}

/// Rebinding consumes a real button/axis press until *that same gamepad*
/// releases it. Other connected gamepads retain their independent inputs.
fn merged_gamepad_buttons_excluding<K: Eq + std::hash::Hash>(
    by_controller: &HashMap<K, BTreeSet<String>>,
    suppressed: &HashMap<K, BTreeSet<String>>,
) -> BTreeSet<String> {
    by_controller
        .iter()
        .flat_map(|(id, buttons)| {
            buttons
                .iter()
                .filter(move |button| !suppressed.get(id).is_some_and(|set| set.contains(*button)))
                .cloned()
        })
        .collect()
}

fn record_recent_rom(recent: &mut Vec<PathBuf>, path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    recent.retain(|old| old != &path);
    recent.insert(0, path);
    recent.truncate(8);
}

fn empty_snapshot() -> LibrarySnapshot {
    LibrarySnapshot {
        entries: Vec::new(),
        filter: LibraryFilter::default(),
        total_count: 0,
        installed_count: 0,
        missing_count: 0,
    }
}

fn normalize_query(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A press and release between two UI ticks must still reach one SNES
/// controller snapshot; ordinary held keys continue to work as before.
fn gameplay_key_active(input: &egui::InputState, key: egui::Key) -> bool {
    input.key_down(key) || input.key_pressed(key)
}

/// Decide which physical inputs may control the emulated pad.
fn controller_input_sources(
    mode: InputDeviceMode,
    keyboard_focused: bool,
    binding_keyboard: bool,
) -> (bool, bool) {
    let keyboard = matches!(mode, InputDeviceMode::Auto | InputDeviceMode::Keyboard)
        && !keyboard_focused
        && !binding_keyboard;
    let gamepad = matches!(mode, InputDeviceMode::Auto | InputDeviceMode::Gamepad);
    (keyboard, gamepad)
}

/// Shared bindings are valid but can produce surprising simultaneous SNES
/// presses. Surface them as warnings rather than silently discarding a choice.
fn merge_worker_refresh_bookkeeping(live: &mut RuntimeConfig, worker: &RuntimeConfig) {
    let current = &mut live.advanced.providers;
    let incoming = &worker.advanced.providers;
    current.last_metadata_refresh_unix = newest_timestamp(
        current.last_metadata_refresh_unix,
        incoming.last_metadata_refresh_unix,
    );
    current.last_cover_refresh_unix = newest_timestamp(
        current.last_cover_refresh_unix,
        incoming.last_cover_refresh_unix,
    );
    current.last_cheat_refresh_unix = newest_timestamp(
        current.last_cheat_refresh_unix,
        incoming.last_cheat_refresh_unix,
    );
}

fn newest_timestamp(current: Option<u64>, incoming: Option<u64>) -> Option<u64> {
    match (current, incoming) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn shared_gamepad_bindings(bindings: &BTreeMap<String, String>) -> Vec<(String, Vec<String>)> {
    let mut by_physical = BTreeMap::<String, Vec<String>>::new();
    for (action, physical) in bindings {
        if !physical.is_empty() {
            by_physical
                .entry(physical.clone())
                .or_default()
                .push(action_label(action).to_owned());
        }
    }
    by_physical
        .into_iter()
        .filter(|(_, actions)| actions.len() > 1)
        .collect()
}

fn input_actions() -> &'static [&'static str] {
    &[
        "up", "down", "left", "right", "start", "select", "a", "b", "x", "y", "l", "r",
    ]
}

fn action_label(action: &str) -> &'static str {
    match action {
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "start" => "Start",
        "select" => "Select",
        "a" => "A",
        "b" => "B",
        "x" => "X",
        "y" => "Y",
        "l" => "L",
        "r" => "R",
        _ => "Unknown",
    }
}

fn set_controller_flag(state: &mut ControllerState, action: &str, pressed: bool) {
    match action {
        "up" => state.up |= pressed,
        "down" => state.down |= pressed,
        "left" => state.left |= pressed,
        "right" => state.right |= pressed,
        "start" => state.start |= pressed,
        "select" => state.select |= pressed,
        "a" => state.a |= pressed,
        "b" => state.b |= pressed,
        "x" => state.x |= pressed,
        "y" => state.y |= pressed,
        "l" => state.l |= pressed,
        "r" => state.r |= pressed,
        _ => {}
    }
}

fn parse_egui_key(binding: &str) -> Option<egui::Key> {
    match binding {
        "ArrowUp" => Some(egui::Key::ArrowUp),
        "ArrowDown" => Some(egui::Key::ArrowDown),
        "ArrowLeft" => Some(egui::Key::ArrowLeft),
        "ArrowRight" => Some(egui::Key::ArrowRight),
        "Enter" => Some(egui::Key::Enter),
        "Space" => Some(egui::Key::Space),
        "A" => Some(egui::Key::A),
        "B" => Some(egui::Key::B),
        "C" => Some(egui::Key::C),
        "D" => Some(egui::Key::D),
        "E" => Some(egui::Key::E),
        "F" => Some(egui::Key::F),
        "G" => Some(egui::Key::G),
        "H" => Some(egui::Key::H),
        "I" => Some(egui::Key::I),
        "J" => Some(egui::Key::J),
        "K" => Some(egui::Key::K),
        "L" => Some(egui::Key::L),
        "M" => Some(egui::Key::M),
        "N" => Some(egui::Key::N),
        "O" => Some(egui::Key::O),
        "P" => Some(egui::Key::P),
        "Q" => Some(egui::Key::Q),
        "R" => Some(egui::Key::R),
        "S" => Some(egui::Key::S),
        "T" => Some(egui::Key::T),
        "U" => Some(egui::Key::U),
        "V" => Some(egui::Key::V),
        "W" => Some(egui::Key::W),
        "X" => Some(egui::Key::X),
        "Y" => Some(egui::Key::Y),
        "Z" => Some(egui::Key::Z),
        _ => None,
    }
}

fn abbreviate_title(title: &str, max_chars: usize) -> String {
    let mut value = String::new();
    for (index, ch) in title.chars().enumerate() {
        if index >= max_chars {
            value.push_str("...");
            return value;
        }
        value.push(ch);
    }
    value
}

fn resolve_cache_root(config: &RuntimeConfig, assets: &AssetConfig) -> PathBuf {
    config
        .library
        .cache_dir
        .clone()
        .or_else(|| assets.cache_dir.clone())
        .unwrap_or_else(|| assets.cache_root())
}

fn job_label(kind: &WorkerCommandKind) -> &'static str {
    match kind {
        WorkerCommandKind::RefreshSnapshot => "Scan Library",
        WorkerCommandKind::RefreshMetadata => "Refresh Metadata",
        WorkerCommandKind::RefreshArtwork => "Get Artwork",
        WorkerCommandKind::RefreshCovers { .. } => "Refresh Covers",
        WorkerCommandKind::RefreshCheats { .. } => "Refresh Cheats",
        WorkerCommandKind::RefreshAll => "Refresh All",
        WorkerCommandKind::MaterializeRom { .. } => "Load Game",
    }
}

fn snapshot_logs(logs: &SharedLogBuffer) -> Vec<String> {
    logs.lock()
        .map(|lines| lines.iter().cloned().collect())
        .unwrap_or_default()
}

fn apply_theme(ctx: &egui::Context, prefer_dark_mode: bool) {
    if prefer_dark_mode {
        ctx.set_visuals(egui::Visuals::dark());
    } else {
        ctx.set_visuals(egui::Visuals::light());
    }
}

/// Use exact integer pixel multiples when requested; permit shrinking for
/// very small Wayland tiles and fractional scaling when explicitly selected.
fn fit_game_size(source: Vec2, available: Vec2, integer_scale: bool) -> Vec2 {
    if source.x <= 0.0 || source.y <= 0.0 {
        return source;
    }
    let maximum_scale = (available.x / source.x)
        .min(available.y / source.y)
        .max(0.01);
    let scale = if integer_scale && maximum_scale >= 1.0 {
        maximum_scale.floor()
    } else {
        maximum_scale
    };
    source * scale
}

fn write_png_screenshot(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::save_buffer_with_format(
        path,
        rgba,
        width,
        height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    Ok(())
}

fn fit_size(source: Vec2, available: Vec2) -> Vec2 {
    if source.x <= 0.0 || source.y <= 0.0 {
        return source;
    }
    let scale = (available.x / source.x)
        .min(available.y / source.y)
        .clamp(0.1, 4.0);
    Vec2::new(source.x * scale, source.y * scale)
}

fn open_path(path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }
    Ok(())
}
