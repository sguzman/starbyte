//! Opt-in native acceptance shell for the independent emulation engine.
//!
//! Deliberately separate from the user's established library UI until all
//! existing game controls, cheats, saves and layout panels are migrated.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use eframe::egui::{self, Color32};
use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, AudioPitchMode, InputDeviceMode, RuntimeConfig, UiSettings},
};
use starbyte_frontend::{
    InstalledStatus, LibraryEntry, LibraryFilter, LibrarySnapshot, SessionControlsSnapshot,
    SessionLaunchRequest, SessionSaveAction, SessionSaveOutcome, SessionScreenshot,
    SoundSessionMode,
};

use crate::{
    app::{
        controller_input_sources, input_actions, parse_egui_key, set_controller_flag,
        write_png_screenshot,
    },
    preview_gamepads::{PreviewGamepads, merge_sources},
    scheduler::EmulatorScheduler,
    scheduler_video::WorkerDisplay,
    session_engine::{
        InputPort, OwnedSession, PublishedGameFrame, RenderPort, WorkerLaunchResponse,
        WorkerSaveResponse,
    },
    worker::{AppWorker, WorkerCommand, WorkerCommandKind, WorkerEvent},
};

/// Keep library presentation bounded even with thousands of ROMs.
fn indexed_installed_matches(snapshot: &LibrarySnapshot, query: &str) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    snapshot
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry.installed_status == InstalledStatus::Installed
                && (needle.is_empty() || entry.display_title.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect()
}

fn pitch_for(mode: AudioPitchMode, custom_percent: u16, speed: u16) -> f32 {
    match mode {
        AudioPitchMode::Preserve => 1.0,
        AudioPitchMode::FollowSpeed => f32::from(speed) / 100.0,
        AudioPitchMode::Custom => f32::from(custom_percent.clamp(50, 200)) / 100.0,
    }
}

enum PendingOperation {
    Save {
        epoch: u64,
        reply: Receiver<Result<WorkerSaveResponse, String>>,
    },
    SpeakerCheck {
        epoch: u64,
        reply: Receiver<Result<(), String>>,
    },
    ClearCheats {
        epoch: u64,
        reply: Receiver<Result<(), String>>,
    },
    Screenshot {
        epoch: u64,
        reply: Receiver<Result<SessionScreenshot, String>>,
    },
    PngWrite {
        epoch: u64,
        reply: Receiver<Result<PathBuf, String>>,
    },
    LoadRom {
        epoch: u64,
        path: PathBuf,
        reply: Receiver<Result<WorkerLaunchResponse, String>>,
    },
}

pub struct ThreadedPreviewApp {
    engine: EmulatorScheduler<OwnedSession, PublishedGameFrame>,
    input: InputPort,
    render: RenderPort,
    display: WorkerDisplay,
    controls: SessionControlsSnapshot,
    focused: bool,
    playing: bool,
    pause_when_unfocused: bool,
    integer_scale: bool,
    speed_percent: u16,
    active_speed_percent: u16,
    fast_forward_percent: u16,
    fast_forward_active: bool,
    pitch_mode: AudioPitchMode,
    pitch_percent: u16,
    audio_enabled: bool,
    audio_muted: bool,
    audio_volume: f32,
    status_settings: UiSettings,
    show_input_diagnostics: bool,
    keyboard_bindings: BTreeMap<String, String>,
    gamepads: PreviewGamepads,
    input_mode: InputDeviceMode,
    exit_after_frames: Option<u64>,
    exit_after_library_scan: bool,
    exit_after_library_play: bool,
    library_play_selected: bool,
    repaint_gap_smoke_pending: bool,
    repaint_gap_challenge: Option<(Instant, u64)>,
    last_error: Option<String>,
    screenshot_root: PathBuf,
    loaded_rom_path: Option<PathBuf>,
    status_note: Option<String>,
    pending: Vec<PendingOperation>,
    /// Invalidate any old game's asynchronous save/capture receipts on launch.
    session_epoch: u64,
    /// Newest requested launch while the owner is installing a cartridge.
    pending_next_launch: Option<SessionLaunchRequest>,
    library_assets: AssetConfig,
    library_config: RuntimeConfig,
    library_worker: Option<AppWorker>,
    library_snapshot: Option<LibrarySnapshot>,
    library_status: String,
    library_query: String,
    library_matches: Vec<usize>,
    show_library: bool,
    next_library_job: u64,
    pending_library_rom: Option<u64>,
}

impl ThreadedPreviewApp {
    pub fn new(
        assets: AssetConfig,
        config: &RuntimeConfig,
        rom_path: Option<PathBuf>,
        exit_after_frames: Option<u64>,
        preview_background: bool,
        exit_after_library_scan: bool,
        exit_after_library_play: bool,
    ) -> Result<Self> {
        let screenshot_root = assets.screenshot_root();
        // The worker owns cartridge installation even at startup. Creating
        // an empty session here makes the native window responsive while a
        // large ROM, ZIP archive or SRAM image is being prepared.
        let mut session = OwnedSession::create(assets.clone())?;
        session.set_audio(
            config.audio.enabled,
            config.audio.mute_on_startup,
            config.audio.volume,
        );
        let speed_percent = match config.video.playback_speed_percent {
            50 | 75 | 100 | 125 | 150 | 200 | 300 => config.video.playback_speed_percent,
            _ => 100,
        };
        let pitch_factor = pitch_for(
            config.audio.pitch_mode,
            config.audio.pitch_percent,
            speed_percent,
        );
        session.set_speed(speed_percent, pitch_factor);
        let controls = session.controls_snapshot();
        let (engine, input, render) = session.spawn_with_ports()?;
        engine.set_speed_percent(speed_percent);
        render.set_visible(true);
        // Native smoke may run under a compositor without keyboard focus.
        // This opt-in does not mutate RuntimeConfig or the default policy.
        let pause_when_unfocused = config.ui.pause_when_unfocused && !preview_background;
        engine.set_pause_when_unfocused(pause_when_unfocused);
        engine.set_focused(true);
        engine.set_playing(false);
        let mut app = Self {
            engine,
            input,
            render,
            display: WorkerDisplay::default(),
            controls,
            focused: true,
            playing: false,
            pause_when_unfocused,
            integer_scale: config.video.integer_scale,
            speed_percent,
            active_speed_percent: speed_percent,
            fast_forward_percent: config.video.fast_forward_speed_percent.clamp(100, 400),
            fast_forward_active: false,
            pitch_mode: config.audio.pitch_mode,
            pitch_percent: config.audio.pitch_percent,
            audio_enabled: config.audio.enabled,
            audio_muted: config.audio.mute_on_startup,
            audio_volume: config.audio.volume,
            status_settings: config.ui.clone(),
            show_input_diagnostics: false,
            keyboard_bindings: config.input.keyboard_bindings.clone(),
            gamepads: PreviewGamepads::new(config.input.gamepad_bindings.clone()),
            input_mode: config.input.active_device,
            exit_after_frames,
            exit_after_library_scan,
            exit_after_library_play,
            library_play_selected: false,
            repaint_gap_smoke_pending: false,
            repaint_gap_challenge: None,
            last_error: None,
            screenshot_root,
            loaded_rom_path: None,
            status_note: None,
            pending: Vec::new(),
            session_epoch: 0,
            pending_next_launch: None,
            library_assets: assets,
            library_config: config.clone(),
            library_worker: None,
            library_snapshot: None,
            library_status: "Not scanned".to_owned(),
            library_query: String::new(),
            library_matches: Vec::new(),
            show_library: rom_path.is_none(),
            next_library_job: 1,
            pending_library_rom: None,
        };
        if let Some(path) = rom_path {
            app.queue_rom_load(path);
        } else {
            app.refresh_library();
        }
        Ok(app)
    }

    /// Native CI-only experiment. No actual compositor callbacks are
    /// processed during the interval even if Weston asks for a repaint.
    pub fn arm_repaint_gap_smoke(&mut self) {
        self.repaint_gap_smoke_pending = true;
    }

    /// Tab fast-forward is a temporary worker-clock rate, independent of
    /// the ordinary speed menu. Only transitions enqueue speed commands:
    /// holding Tab must not continually flush the audio queue at 60 Hz.
    fn apply_playback(&mut self, speed: u16, force_audio_update: bool) {
        if self.active_speed_percent == speed && !force_audio_update {
            return;
        }
        self.active_speed_percent = speed;
        let pitch = pitch_for(self.pitch_mode, self.pitch_percent, speed);
        if let Err(error) = self.engine.configure_playback_speed(speed, pitch) {
            self.last_error = Some(error);
        }
    }

    fn can_control_playback(&self) -> bool {
        // Keep Pause/Resume available while a new ROM is only being
        // prepared on the I/O lane: the previous cartridge still owns
        // the clock. Never toggle during an actual owner installation.
        self.controls.session.has_rom
            && self.pending_next_launch.is_none()
            && !self
                .pending
                .iter()
                .any(|op| matches!(op, PendingOperation::LoadRom { .. }))
    }

    fn can_modify_running_game(&self) -> bool {
        self.can_control_playback() && self.pending_library_rom.is_none()
    }

    fn queue_save(&mut self, action: SessionSaveAction) {
        if !self.can_modify_running_game() {
            self.last_error = Some("Cartridge is still loading; save action skipped".to_owned());
            return;
        }
        match self.engine.queue_save_action_with_frame(action) {
            Ok(reply) => self.pending.push(PendingOperation::Save {
                epoch: self.session_epoch,
                reply,
            }),
            Err(error) => self.last_error = Some(error),
        }
    }

    fn queue_rom_load(&mut self, path: PathBuf) {
        // Direct files and ZIPs share the library's dedicated preparation
        // lane. Reuse indexed cheats for an already-scanned cartridge.
        let matching = self.library_snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .entries
                .iter()
                .find(|entry| {
                    entry.local.as_ref().is_some_and(|local| {
                        local.rom_path == path
                            || local.extracted_cache_path.as_deref() == Some(path.as_path())
                    })
                })
                .cloned()
        });
        self.queue_rom_preparation(
            WorkerCommandKind::MaterializePath {
                path: path.clone(),
                matching,
            },
            format!("Preparing {}...", path.display()),
        );
    }

    fn queue_rom_launch(&mut self, request: SessionLaunchRequest) {
        if self
            .pending
            .iter()
            .any(|operation| matches!(operation, PendingOperation::LoadRom { .. }))
        {
            // A queued replacement supersedes the current launch's UI
            // acknowledgements immediately, even before worker handoff.
            self.session_epoch = self.session_epoch.wrapping_add(1);
            self.status_note = Some(format!(
                "Queued newer cartridge: {}",
                request.rom_path.display()
            ));
            self.last_error = None;
            self.pending_next_launch = Some(request);
            return;
        }
        self.pending_next_launch = None;
        let path = request.rom_path.clone();
        match self.engine.queue_launch_with_state(request) {
            Ok(reply) => {
                self.session_epoch = self.session_epoch.wrapping_add(1);
                self.input.clear();
                self.status_note = Some(format!("Loading {}...", path.display()));
                self.last_error = None;
                self.pending.push(PendingOperation::LoadRom {
                    epoch: self.session_epoch,
                    path,
                    reply,
                });
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    fn queue_rom_preparation(&mut self, kind: WorkerCommandKind, status: String) {
        // Preparation runs on a separate ROM I/O lane and may proceed while
        // a prior cartridge is being installed. Its latest request is
        // handed to queue_rom_launch after materialization completes.
        if self.library_worker.is_none() {
            self.library_worker = Some(AppWorker::spawn(self.library_assets.clone()));
        }
        let job_id = self.next_library_job;
        self.next_library_job += 1;
        // A newly selected ROM supersedes even a previously prepared
        // replacement waiting behind the active owner's launch. If this
        // preparation later fails, never install that older queued choice.
        self.pending_next_launch = None;
        // AppWorker coalesces pending archive jobs; only the latest prepared
        // selection may reach the emulation worker.
        self.pending_library_rom = Some(job_id);
        // Invalidate old-game replies as soon as a new selection enters the
        // ROM preparation lane, not only when its worker launch begins.
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.status_note = Some(status.clone());
        self.library_status = status;
        self.last_error = None;
        self.library_worker.as_ref().unwrap().submit(WorkerCommand {
            job_id,
            config: self.library_config.clone(),
            filter: LibraryFilter {
                installed_only: true,
                ..LibraryFilter::default()
            },
            kind,
        });
    }

    fn refresh_library(&mut self) {
        if self.library_worker.is_none() {
            self.library_worker = Some(AppWorker::spawn(self.library_assets.clone()));
        }
        let job_id = self.next_library_job;
        self.next_library_job += 1;
        self.library_status = "Scanning local library...".to_owned();
        self.library_worker.as_ref().unwrap().submit(WorkerCommand {
            job_id,
            config: self.library_config.clone(),
            filter: LibraryFilter {
                installed_only: true,
                ..LibraryFilter::default()
            },
            kind: WorkerCommandKind::RefreshSnapshot,
        });
    }

    fn queue_library_entry(&mut self, entry: LibraryEntry) {
        if entry.installed_status != InstalledStatus::Installed {
            self.last_error = Some("Game is not installed locally".to_owned());
            return;
        }
        let title = entry.display_title.clone();
        self.queue_rom_preparation(
            WorkerCommandKind::MaterializeRom { entry },
            format!("Preparing {title}..."),
        );
    }

    fn poll_library_events(&mut self) {
        let mut launch = None;
        let mut auto_select = None;
        if let Some(worker) = &self.library_worker {
            while let Some(event) = worker.try_recv() {
                match event {
                    WorkerEvent::SnapshotReady {
                        snapshot, status, ..
                    } => {
                        if self.exit_after_library_play && !self.library_play_selected {
                            auto_select = snapshot
                                .entries
                                .iter()
                                .find(|entry| entry.installed_status == InstalledStatus::Installed)
                                .cloned();
                        }
                        // Refresh the index only when the library snapshot
                        // changes, not on every egui repaint.
                        self.library_matches =
                            indexed_installed_matches(&snapshot, &self.library_query);
                        self.library_snapshot = Some(snapshot);
                        self.library_status = status;
                    }
                    WorkerEvent::RomReady {
                        job_id, request, ..
                    }
                    | WorkerEvent::DirectRomReady {
                        job_id, request, ..
                    } if self.pending_library_rom == Some(job_id) => {
                        self.pending_library_rom = None;
                        self.library_status = "Cartridge prepared".to_owned();
                        launch = Some(request);
                    }
                    WorkerEvent::JobFailed { job_id, error, .. }
                        if self.pending_library_rom == Some(job_id) =>
                    {
                        self.pending_library_rom = None;
                        self.library_status = "ROM preparation failed".to_owned();
                        self.status_note = None;
                        self.last_error = Some(format!("Library ROM preparation failed: {error}"));
                    }
                    WorkerEvent::JobFailed { label, error, .. } => {
                        if label == "Scan Library" {
                            self.library_status = format!("Library scan failed: {error}");
                        }
                    }
                    WorkerEvent::JobSuperseded { job_id, .. }
                        if self.pending_library_rom == Some(job_id) =>
                    {
                        self.pending_library_rom = None;
                    }
                    WorkerEvent::JobStarted { .. }
                    | WorkerEvent::RomReady { .. }
                    | WorkerEvent::DirectRomReady { .. }
                    | WorkerEvent::JobSuperseded { .. } => {}
                }
            }
        }
        if let Some(entry) = auto_select {
            self.library_play_selected = true;
            self.queue_library_entry(entry);
        }
        if let Some(request) = launch {
            self.queue_rom_launch(request);
        }
    }

    fn queue_screenshot(&mut self) {
        if !self.can_modify_running_game() {
            return;
        }
        // Match the full-library worker's bound: two captures or PNG writes
        // can be pending at once. Repeated F12 must not spawn unbounded
        // screenshot writers or exhaust memory during slow filesystem I/O.
        let outstanding = self
            .pending
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    PendingOperation::Screenshot { .. } | PendingOperation::PngWrite { .. }
                )
            })
            .count();
        if outstanding >= 2 {
            self.last_error = Some("Finish pending screenshots before taking another".to_owned());
            return;
        }
        match self.engine.queue_screenshot_pixels() {
            Ok(reply) => self.pending.push(PendingOperation::Screenshot {
                epoch: self.session_epoch,
                reply,
            }),
            Err(error) => self.last_error = Some(error),
        }
    }

    fn write_screenshot_async(
        &mut self,
        capture: SessionScreenshot,
    ) -> Result<Receiver<Result<PathBuf, String>>, String> {
        let rgba = capture.rgba;
        let width = capture.width;
        let height = capture.height;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self
            .screenshot_root
            .join(format!("starbyte-{timestamp}-frame-{}.png", capture.frame));
        let (reply, result) = mpsc::channel();
        thread::Builder::new()
            .name("starbyte-screenshot".to_owned())
            .spawn(move || {
                let response = write_png_screenshot(&path, &rgba, width, height)
                    .map(|()| path)
                    .map_err(|error| error.to_string());
                let _ = reply.send(response);
            })
            .map_err(|error| error.to_string())?;
        Ok(result)
    }

    fn poll_operations(&mut self, ctx: &egui::Context) {
        let mut still_pending = Vec::new();
        let mut launch_finished = false;
        for operation in std::mem::take(&mut self.pending) {
            match operation {
                PendingOperation::LoadRom { epoch, path, reply } => match reply.try_recv() {
                    Ok(Ok(response)) => {
                        launch_finished = true;
                        let _ = self.engine.take_latest_frame();
                        // Successful installs change the actual owner even if
                        // a newer selection is being prepared. Keep controls
                        // and pixels synchronized with that owner; only the
                        // superseded status message must be discarded.
                        self.controls = response.controls;
                        self.display.import_frame(ctx, response.frame);
                        self.loaded_rom_path = Some(path.clone());
                        self.engine.set_playing(true);
                        self.playing = true;
                        if epoch == self.session_epoch && self.pending_next_launch.is_none() {
                            self.status_note = Some(format!("Loaded {}", path.display()));
                            self.last_error = None;
                        }
                    }
                    Ok(Err(error)) => {
                        launch_finished = true;
                        if epoch == self.session_epoch && self.pending_next_launch.is_none() {
                            self.last_error = Some(format!("ROM load failed: {error}"));
                            self.status_note = None;
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::LoadRom { epoch, path, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        launch_finished = true;
                        if epoch == self.session_epoch && self.pending_next_launch.is_none() {
                            self.last_error = Some("ROM loader worker stopped".to_owned());
                            self.status_note = None;
                        }
                    }
                },
                PendingOperation::ClearCheats { epoch, .. } if epoch != self.session_epoch => {}
                PendingOperation::ClearCheats { epoch, reply } => match reply.try_recv() {
                    Ok(Ok(())) => {
                        self.status_note = Some("Runtime cheats disabled".to_owned());
                        self.last_error = None;
                    }
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::ClearCheats { epoch, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Cheat command worker stopped".to_owned());
                    }
                },
                PendingOperation::SpeakerCheck { epoch, .. } if epoch != self.session_epoch => {}
                PendingOperation::SpeakerCheck { epoch, reply } => match reply.try_recv() {
                    Ok(Ok(())) => {
                        self.status_note =
                            Some("Speaker check submitted to the game audio sink".to_owned());
                        self.last_error = None;
                    }
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::SpeakerCheck { epoch, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error =
                            Some("Audio worker stopped before speaker check".to_owned());
                    }
                },
                // A previous game's async acknowledgement must never publish
                // controls, restored pixels, errors or status over a newer ROM.
                PendingOperation::Save { epoch, .. } if epoch != self.session_epoch => {}
                PendingOperation::Save { epoch, reply } => match reply.try_recv() {
                    Ok(Ok(response)) => {
                        self.controls = response.controls;
                        if let SessionSaveOutcome::SavedTo(path) = response.outcome {
                            self.status_note = Some(format!("Saved state to {}", path.display()));
                        }
                        if let Some(frame) = response.restored_frame {
                            // Discard any framebuffer from before this
                            // restore, including while manually paused.
                            let _ = self.engine.take_latest_frame();
                            self.display.import_frame(ctx, frame);
                        }
                        self.last_error = None;
                    }
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::Save { epoch, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Save worker stopped unexpectedly".to_owned());
                    }
                },
                PendingOperation::Screenshot { epoch, reply } => match reply.try_recv() {
                    Ok(Ok(frame)) => match self.write_screenshot_async(frame) {
                        Ok(io_reply) => {
                            still_pending.push(PendingOperation::PngWrite {
                                epoch,
                                reply: io_reply,
                            });
                        }
                        Err(error) => {
                            if epoch == self.session_epoch {
                                self.last_error = Some(error);
                            }
                        }
                    },
                    Ok(Err(error)) => {
                        if epoch == self.session_epoch {
                            self.last_error = Some(error);
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::Screenshot { epoch, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        if epoch == self.session_epoch {
                            self.last_error = Some("Screenshot worker stopped".to_owned());
                        }
                    }
                },
                PendingOperation::PngWrite { epoch, reply } => match reply.try_recv() {
                    Ok(Ok(path)) => {
                        if epoch == self.session_epoch {
                            self.status_note =
                                Some(format!("Screenshot saved to {}", path.display()));
                            self.last_error = None;
                        }
                    }
                    Ok(Err(error)) => {
                        if epoch == self.session_epoch {
                            self.last_error = Some(error);
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::PngWrite { epoch, reply });
                    }
                    Err(TryRecvError::Disconnected) => {
                        if epoch == self.session_epoch {
                            self.last_error = Some("Screenshot writer stopped".to_owned());
                        }
                    }
                },
            }
        }
        self.pending = still_pending;
        if launch_finished {
            if let Some(request) = self.pending_next_launch.take() {
                self.queue_rom_launch(request);
            }
        }
    }

    fn keyboard(&self, ctx: &egui::Context) -> (ControllerState, ControllerState) {
        // Use the same persisted keyboard bindings as the established game
        // view, including X/Y and shoulder buttons. Never forward keystrokes
        // intended for a text field or keybinding editor to a game.
        if ctx.wants_keyboard_input() {
            return (ControllerState::default(), ControllerState::default());
        }
        let (held, pressed) = ctx.input(|input| {
            let mut held = ControllerState::default();
            let mut pressed = ControllerState::default();
            for &action in input_actions() {
                if let Some(key) = self
                    .keyboard_bindings
                    .get(action)
                    .and_then(|binding| parse_egui_key(binding))
                {
                    set_controller_flag(&mut held, action, input.key_down(key));
                    set_controller_flag(&mut pressed, action, input.key_pressed(key));
                }
            }
            (held, pressed)
        });
        (held, pressed)
    }
}

#[cfg(test)]
mod pitch_mode_tests {
    use super::*;

    #[test]
    fn library_preview_starts_with_no_cartridge_and_no_emulation() {
        let temp = tempfile::tempdir().unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let app = ThreadedPreviewApp::new(
            assets,
            &RuntimeConfig::default(),
            None,
            None,
            false,
            false,
            false,
        )
        .unwrap();
        assert!(!app.playing);
        assert!(!app.controls.session.has_rom);
        assert!(app.show_library);
        assert!(app.loaded_rom_path.is_none());
    }

    #[test]
    fn selecting_an_indexed_rom_uses_the_independent_worker_until_video_is_ready() {
        use starbyte_frontend::{LocalRomInfo, LocalRomSourceKind};
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("preview-library-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        let entry = LibraryEntry {
            game_id: "preview-demo".to_owned(),
            display_title: "Preview Demo".to_owned(),
            installed_status: InstalledStatus::Installed,
            local: Some(LocalRomInfo {
                game_id: "preview-demo".to_owned(),
                title: "Preview Demo".to_owned(),
                normalized_title: "preview demo".to_owned(),
                rom_path: rom.clone(),
                source_kind: LocalRomSourceKind::File,
                archive_member_path: None,
                extracted_cache_path: None,
                mapper: "LoROM".to_owned(),
                coprocessor: None,
                region: "NTSC".to_owned(),
                file_size_bytes: std::fs::metadata(&rom).unwrap().len(),
            }),
            metadata: None,
            cover: None,
            cheats: Vec::new(),
        };
        app.queue_library_entry(entry);
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.controls.session.has_rom && app.loaded_rom_path.as_deref() == Some(rom.as_path())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.controls.session.has_rom,
            "indexed selection was not launched"
        );
        assert_eq!(app.loaded_rom_path.as_deref(), Some(rom.as_path()));
        assert!(app.playing);
        assert!(
            app.display.texture().is_some(),
            "worker still frame was not uploaded"
        );
        // No egui repaint calls: the independent worker remains authoritative.
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.engine.completed_frames() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.engine.completed_frames() >= 3);
    }

    #[test]
    fn startup_cartridge_is_prepared_without_an_egui_frame_callback() {
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("startup-preview-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, Some(rom.clone()), None, true, false, false)
                .unwrap();
        // Constructor never borrowed an emulator to load the cartridge and
        // never stepped a frame on the window thread.
        assert!(!app.controls.session.has_rom);
        assert!(!app.playing);
        assert!(app.pending_library_rom.is_some());
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.controls.session.has_rom {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.controls.session.has_rom);
        assert_eq!(app.loaded_rom_path.as_deref(), Some(rom.as_path()));
        assert!(app.playing);
        assert!(app.display.texture().is_some());
    }

    #[test]
    fn direct_rom_preparation_uses_same_background_worker_as_library() {
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("direct-preview-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        app.queue_rom_load(rom.clone());
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.loaded_rom_path.as_deref() == Some(rom.as_path()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.loaded_rom_path.as_deref(), Some(rom.as_path()));
        assert!(app.controls.session.has_rom);
        assert!(app.display.texture().is_some());

        // This is an actual installed owner. Synthetic held receipts model
        // slow capture/PNG disk I/O without writing into the filesystem.
        let (capture_tx, capture_rx) = mpsc::channel();
        let (png_tx, png_rx) = mpsc::channel();
        app.pending.push(PendingOperation::Screenshot {
            epoch: app.session_epoch,
            reply: capture_rx,
        });
        app.pending.push(PendingOperation::PngWrite {
            epoch: app.session_epoch,
            reply: png_rx,
        });
        assert!(app.can_modify_running_game());
        let pending_count = app.pending.len();
        app.queue_screenshot();
        assert_eq!(app.pending.len(), pending_count);
        assert!(app.last_error.as_deref().is_some_and(|error| {
            error.contains("Finish pending screenshots")
        }));
        drop(capture_tx);
        drop(png_tx);
    }

    #[test]
    fn newer_direct_selection_wins_over_an_older_preparation_request() {
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("older.sfc");
        let second = temp.path().join("newer.sfc");
        let demo = starbyte_core::testing::demo::demo_rom_bytes();
        std::fs::write(&first, &demo).unwrap();
        std::fs::write(&second, &demo).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        app.queue_rom_load(first);
        let older = app.pending_library_rom.unwrap();
        app.queue_rom_load(second.clone());
        assert!(app.pending_library_rom.unwrap() > older);
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.loaded_rom_path.as_deref() == Some(second.as_path()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.loaded_rom_path.as_deref(), Some(second.as_path()));
        assert!(app.controls.session.has_rom);
        assert!(app.last_error.is_none());
    }

    #[test]
    fn failed_direct_open_preserves_the_running_cartridge() {
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let good = temp.path().join("working.sfc");
        std::fs::write(&good, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let missing = temp.path().join("not-present.sfc");
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        let ctx = egui::Context::default();
        app.queue_rom_load(good.clone());
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.loaded_rom_path.as_deref() == Some(good.as_path()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.loaded_rom_path.as_deref(), Some(good.as_path()));
        app.queue_rom_load(missing);
        // The installed game's clock can still be paused, but do not
        // save into a cartridge namespace during a pending replacement.
        assert!(app.can_control_playback());
        assert!(!app.can_modify_running_game());
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("ROM load failed"))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            app.last_error.is_some(),
            "missing ROM error was not delivered"
        );
        assert_eq!(app.loaded_rom_path.as_deref(), Some(good.as_path()));
        assert!(app.playing);
        assert!(app.controls.session.has_rom);
    }

    #[test]
    fn direct_preparation_can_supersede_an_install_in_flight() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.sfc");
        let next = temp.path().join("next.sfc");
        let demo = starbyte_core::testing::demo::demo_rom_bytes();
        std::fs::write(&first, &demo).unwrap();
        std::fs::write(&next, &demo).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        app.queue_rom_launch(SessionLaunchRequest {
            rom_path: first,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        });
        assert!(
            app.pending
                .iter()
                .any(|op| matches!(op, PendingOperation::LoadRom { .. }))
        );
        app.queue_rom_load(next);
        assert!(app.pending_library_rom.is_some());
        assert!(app.last_error.is_none());
        // A previous launch can install successfully while the new ROM is
        // still being prepared. The UI must follow the actual owner without
        // overwriting the newer selection's progress indicator.
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_operations(&egui::Context::default());
            if app
                .pending
                .iter()
                .all(|op| !matches!(op, PendingOperation::LoadRom { .. }))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.controls.session.has_rom);
        assert!(app.playing);
        assert!(
            app.status_note
                .as_deref()
                .is_some_and(|status| { status.starts_with("Preparing ") })
        );
    }

    #[test]
    fn selecting_next_cartridge_rejects_old_acknowledgements_during_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let next = temp.path().join("next.sfc");
        std::fs::write(&next, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        let prior_epoch = app.session_epoch;
        let (save_sender, save_reply) = mpsc::channel();
        let (load_sender, load_reply) = mpsc::channel();
        app.pending.push(PendingOperation::Save {
            epoch: prior_epoch,
            reply: save_reply,
        });
        app.pending.push(PendingOperation::LoadRom {
            epoch: prior_epoch,
            path: temp.path().join("old.sfc"),
            reply: load_reply,
        });
        app.queue_rom_load(next);
        assert!(app.pending_library_rom.is_some());
        assert_ne!(app.session_epoch, prior_epoch);
        save_sender
            .send(Err("old save failure".to_owned()))
            .unwrap();
        load_sender
            .send(Err("old launch failure".to_owned()))
            .unwrap();
        app.poll_operations(&egui::Context::default());
        assert!(app.last_error.is_none());
        assert!(
            app.status_note
                .as_deref()
                .is_some_and(|status| { status.starts_with("Preparing ") })
        );
    }

    #[test]
    fn old_cartridge_async_receipts_cannot_override_new_launch() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("replacement.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();

        // Deliberately delay receipts belonging to the previous session.
        let old_epoch = app.session_epoch;
        let (save_sender, save_reply) = mpsc::channel();
        let (capture_sender, capture_reply) = mpsc::channel();
        let (png_sender, png_reply) = mpsc::channel();
        let (cheat_sender, cheat_reply) = mpsc::channel();
        let (speaker_sender, speaker_reply) = mpsc::channel();
        app.pending.push(PendingOperation::ClearCheats {
            epoch: old_epoch,
            reply: cheat_reply,
        });
        app.pending.push(PendingOperation::SpeakerCheck {
            epoch: old_epoch,
            reply: speaker_reply,
        });
        app.pending.push(PendingOperation::Save {
            epoch: old_epoch,
            reply: save_reply,
        });
        app.pending.push(PendingOperation::Screenshot {
            epoch: old_epoch,
            reply: capture_reply,
        });
        app.pending.push(PendingOperation::PngWrite {
            epoch: old_epoch,
            reply: png_reply,
        });
        app.queue_rom_launch(SessionLaunchRequest {
            rom_path: rom,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        });
        assert_ne!(app.session_epoch, old_epoch);
        assert!(app.last_error.is_none());
        save_sender
            .send(Ok(WorkerSaveResponse {
                outcome: SessionSaveOutcome::SavedTo(temp.path().join("old-save")),
                controls: app.controls.clone(),
                restored_frame: None,
            }))
            .unwrap();
        capture_sender
            .send(Err("old capture failure".to_owned()))
            .unwrap();
        png_sender.send(Err("old PNG failure".to_owned())).unwrap();
        cheat_sender.send(Ok(())).unwrap();
        speaker_sender.send(Ok(())).unwrap();
        app.poll_operations(&egui::Context::default());
        assert!(app.last_error.is_none());
        // The replacement may have finished loading while we polled: either
        // launch status is valid, but an old save's feedback is never valid.
        assert!(app.status_note.as_deref().is_some_and(|status| {
            status.starts_with("Loading ") || status.starts_with("Loaded ")
        }));
        assert!(!app.pending.iter().any(|op| matches!(
            op,
            PendingOperation::Save { .. }
                | PendingOperation::Screenshot { .. }
                | PendingOperation::PngWrite { .. }
                | PendingOperation::ClearCheats { .. }
                | PendingOperation::SpeakerCheck { .. }
        )));
    }

    #[test]
    fn screenshot_from_previous_cartridge_finishes_writing_without_old_feedback() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("next.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        app.screenshot_root = temp.path().join("screenshots");
        let previous_epoch = app.session_epoch;
        let (sender, reply) = mpsc::channel();
        app.pending.push(PendingOperation::Screenshot {
            epoch: previous_epoch,
            reply,
        });
        app.queue_rom_launch(SessionLaunchRequest {
            rom_path: rom,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        });
        sender
            .send(Ok(SessionScreenshot {
                frame: 12,
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            }))
            .unwrap();
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut written = false;
        while Instant::now() < deadline {
            app.poll_operations(&ctx);
            written = std::fs::read_dir(&app.screenshot_root)
                .ok()
                .is_some_and(|mut files| files.next().is_some());
            if written
                && !app
                    .pending
                    .iter()
                    .any(|op| matches!(op, PendingOperation::PngWrite { .. }))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(written, "previous session's requested screenshot was lost");
        assert!(app.last_error.is_none());
        assert!(app.status_note.as_deref().is_some_and(|status| {
            status.starts_with("Loading ") || status.starts_with("Loaded ")
        }));
    }

    #[test]
    fn save_controls_are_suppressed_during_cartridge_installation() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("save-guard-demo.sfc");
        std::fs::write(&rom, starbyte_core::testing::demo::demo_rom_bytes()).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        assert!(!app.can_modify_running_game());
        app.queue_rom_launch(SessionLaunchRequest {
            rom_path: rom,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        });
        assert!(!app.can_modify_running_game());
        app.queue_save(SessionSaveAction::QuickSave);
        assert!(app.last_error.is_some());
        assert!(
            !app.pending
                .iter()
                .any(|op| matches!(op, PendingOperation::Save { .. }))
        );
    }

    #[test]
    fn rapid_cartridge_install_replaces_intermediate_result() {
        use std::time::Instant;

        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("intermediate.sfc");
        let final_rom = temp.path().join("final.sfc");
        let demo = starbyte_core::testing::demo::demo_rom_bytes();
        std::fs::write(&first, &demo).unwrap();
        std::fs::write(&final_rom, &demo).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        let request = |rom_path| SessionLaunchRequest {
            rom_path,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        };
        app.queue_rom_launch(request(first));
        app.queue_rom_launch(request(final_rom.clone()));
        assert_eq!(
            app.pending_next_launch
                .as_ref()
                .map(|next| next.rom_path.as_path()),
            Some(final_rom.as_path())
        );
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_operations(&ctx);
            if app.loaded_rom_path.as_deref() == Some(final_rom.as_path()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.loaded_rom_path.as_deref(), Some(final_rom.as_path()));
        assert!(app.controls.session.has_rom);
        assert!(app.pending_next_launch.is_none());
        assert!(app.display.texture().is_some());
    }

    #[test]
    fn newer_preparation_cancels_queued_replacement_even_if_preparation_fails() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.sfc");
        let superseded = temp.path().join("superseded.sfc");
        let missing = temp.path().join("missing.zip");
        let demo = starbyte_core::testing::demo::demo_rom_bytes();
        std::fs::write(&first, &demo).unwrap();
        std::fs::write(&superseded, &demo).unwrap();
        let assets = AssetConfig {
            spc700_ipl: None,
            save_dir: Some(temp.path().join("saves")),
            state_dir: Some(temp.path().join("states")),
            cache_dir: Some(temp.path().join("cache")),
            config_path: None,
        };
        let mut config = RuntimeConfig::default();
        config.audio.enabled = false;
        let mut app =
            ThreadedPreviewApp::new(assets, &config, None, None, true, false, false).unwrap();
        let request = |rom_path| SessionLaunchRequest {
            rom_path,
            sound_mode: SoundSessionMode::Persistent,
            cheats: Vec::new(),
        };
        app.queue_rom_launch(request(first.clone()));
        app.queue_rom_launch(request(superseded.clone()));
        assert_eq!(
            app.pending_next_launch
                .as_ref()
                .map(|next| next.rom_path.as_path()),
            Some(superseded.as_path())
        );
        app.queue_rom_load(missing);
        assert!(app.pending_next_launch.is_none());
        assert!(app.pending_library_rom.is_some());
        assert!(!app.can_modify_running_game());

        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            app.poll_library_events();
            app.poll_operations(&ctx);
            if app.pending_library_rom.is_none()
                && app
                    .pending
                    .iter()
                    .all(|op| !matches!(op, PendingOperation::LoadRom { .. }))
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.pending_library_rom.is_none());
        assert!(app.pending_next_launch.is_none());
        assert_eq!(app.loaded_rom_path.as_deref(), Some(first.as_path()));
        assert!(app.controls.session.has_rom);
        assert!(app.can_modify_running_game());
        assert!(
            app.last_error
                .as_deref()
                .is_some_and(|error| { error.contains("Library ROM preparation failed") })
        );
        assert!(app.status_note.is_none());
    }

    #[test]
    fn large_library_search_only_draws_installed_matching_rows() {
        let sample = |name: &str, installed_status| LibraryEntry {
            game_id: name.to_owned(),
            display_title: name.to_owned(),
            installed_status,
            local: None,
            metadata: None,
            cover: None,
            cheats: Vec::new(),
        };
        let snapshot = LibrarySnapshot {
            entries: vec![
                sample("Super Mario World", InstalledStatus::Installed),
                sample("F-Zero", InstalledStatus::Installed),
                sample("Mario Paint", InstalledStatus::Missing),
            ],
            filter: LibraryFilter::default(),
            total_count: 3,
            installed_count: 2,
            missing_count: 1,
        };
        assert_eq!(indexed_installed_matches(&snapshot, ""), vec![0, 1]);
        assert_eq!(indexed_installed_matches(&snapshot, "mArIo"), vec![0]);
        assert!(indexed_installed_matches(&snapshot, "mario paint").is_empty());
    }

    #[test]
    fn custom_pitch_is_independent_of_fast_forward_speed() {
        assert_eq!(pitch_for(AudioPitchMode::Custom, 80, 100), 0.8);
        assert_eq!(pitch_for(AudioPitchMode::Custom, 80, 400), 0.8);
        assert_eq!(pitch_for(AudioPitchMode::FollowSpeed, 80, 400), 4.0);
    }

    #[test]
    fn follow_speed_changes_pitch_without_changing_custom_mode() {
        assert_eq!(pitch_for(AudioPitchMode::Preserve, 150, 300), 1.0);
        assert_eq!(pitch_for(AudioPitchMode::FollowSpeed, 150, 300), 3.0);
        assert_eq!(pitch_for(AudioPitchMode::Custom, 150, 300), 1.5);
    }
}

impl eframe::App for ThreadedPreviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some((deadline, earlier_frames)) = self.repaint_gap_challenge {
            let now = Instant::now();
            if now < deadline {
                ctx.request_repaint_after(deadline - now);
                return;
            }
            let advanced = self
                .engine
                .completed_frames()
                .saturating_sub(earlier_frames);
            if advanced >= 45 {
                println!("STARBYTE_NO_REPAINT_SMOKE_OK advanced_frames={advanced}");
            } else {
                println!("STARBYTE_NO_REPAINT_SMOKE_FAILED advanced_frames={advanced}");
            }
            self.repaint_gap_challenge = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // Deliberately independent of frame production: an absent heartbeat
        // tells the worker that Wayland stopped delivering redraws. With
        // background pause disabled, emulation continues regardless.
        self.engine.gui_heartbeat();
        self.poll_library_events();
        if self.exit_after_library_scan {
            if let Some(snapshot) = &self.library_snapshot {
                if snapshot.installed_count > 0 {
                    println!(
                        "STARBYTE_THREADED_LIBRARY_SMOKE_OK installed={}",
                        snapshot.installed_count
                    );
                    self.exit_after_library_scan = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        let focused = ctx.input(|input| input.focused);
        let fast_forwarding = focused
            && !ctx.wants_keyboard_input()
            && ctx.input(|input| input.key_down(egui::Key::Tab));
        self.fast_forward_active = fast_forwarding;
        let target_speed = if fast_forwarding {
            self.fast_forward_percent
        } else {
            self.speed_percent
        };
        self.apply_playback(target_speed, false);
        if focused != self.focused {
            self.focused = focused;
            self.engine.set_focused(focused);
        }
        // Focus is not the same as visibility: a window can be on-screen
        // and unfocused while the game continues in the background. Keep
        // framebuffer publication enabled in this preview; the single-slot
        // mailbox safely discards frames when Wayland withholds redraws.
        self.render.set_visible(true);
        // Poll device releases even when the window lacks focus, but only
        // transmit gameplay input while focused. This matches the existing
        // Keyboard / Gamepad / Auto input selection.
        let (keyboard_enabled, gamepad_enabled) =
            controller_input_sources(self.input_mode, ctx.wants_keyboard_input(), false);
        let gamepad = self.gamepads.poll(focused && gamepad_enabled);
        if focused {
            let keyboard = if keyboard_enabled {
                self.keyboard(ctx)
            } else {
                (ControllerState::default(), ControllerState::default())
            };
            let (held, pressed) = merge_sources(keyboard, gamepad);
            self.input.submit(held, pressed);
        } else {
            self.input.clear();
        }

        if let Some(error) = self.engine.take_error() {
            self.last_error = Some(error);
            self.playing = false;
        } else if self.engine.is_worker_finished() && self.playing {
            self.last_error = Some(
                "Emulation worker stopped unexpectedly. Game execution has halted.".to_owned(),
            );
            self.playing = false;
        }
        egui::TopBottomPanel::top("worker_preview_menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("SNES ROM or ZIP", &["sfc", "smc", "swc", "fig", "zip"])
                            .pick_file()
                        {
                            self.queue_rom_load(path);
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Game", |ui| {
                    if ui
                        .add_enabled(
                            self.can_control_playback(),
                            egui::Button::new(if self.playing { "Pause" } else { "Resume" }),
                        )
                        .clicked()
                    {
                        self.playing = !self.playing;
                        self.engine.set_playing(self.playing);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.can_modify_running_game(),
                            egui::Button::new("Quick Save (F5)"),
                        )
                        .clicked()
                    {
                        self.queue_save(SessionSaveAction::QuickSave);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.can_modify_running_game() && self.controls.has_quick_save,
                            egui::Button::new("Quick Load (F8)"),
                        )
                        .clicked()
                    {
                        self.queue_save(SessionSaveAction::QuickLoad);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.can_modify_running_game(),
                            egui::Button::new("Screenshot (F12)"),
                        )
                        .clicked()
                    {
                        self.queue_screenshot();
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Cheats", |ui| {
                        if ui
                            .add_enabled(
                                self.can_modify_running_game(),
                                egui::Button::new("Disable all runtime cheats"),
                            )
                            .clicked()
                        {
                            match self.engine.queue_clear_cheats() {
                                Ok(reply) => {
                                    self.pending.push(PendingOperation::ClearCheats {
                                        epoch: self.session_epoch,
                                        reply,
                                    });
                                }
                                Err(error) => self.last_error = Some(error),
                            }
                            ui.close();
                        }
                    });
                    ui.menu_button("Disk save slots", |ui| {
                        for slot in 1..=3_u8 {
                            if ui
                                .add_enabled(
                                    self.can_modify_running_game(),
                                    egui::Button::new(format!("Save slot {slot}")),
                                )
                                .clicked()
                            {
                                self.queue_save(SessionSaveAction::SaveSlot(slot));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.can_modify_running_game()
                                        && self.controls.state_slots[usize::from(slot - 1)],
                                    egui::Button::new(format!("Load slot {slot}")),
                                )
                                .clicked()
                            {
                                self.queue_save(SessionSaveAction::LoadSlot(slot));
                                ui.close();
                            }
                        }
                    });
                });
                ui.menu_button("Library", |ui| {
                    if ui.button("Browse installed games").clicked() {
                        self.show_library = true;
                        if self.library_worker.is_none() {
                            self.refresh_library();
                        }
                        ui.close();
                    }
                    if ui.button("Refresh installed games").clicked() {
                        self.show_library = true;
                        self.refresh_library();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut self.pause_when_unfocused, "Pause in background")
                        .changed()
                    {
                        self.engine
                            .set_pause_when_unfocused(self.pause_when_unfocused);
                    }
                    ui.checkbox(&mut self.integer_scale, "Integer scale");
                    ui.checkbox(&mut self.show_input_diagnostics, "Input diagnostics");
                    ui.menu_button("Status bar", |ui| {
                        let prefs = &mut self.status_settings;
                        ui.checkbox(&mut prefs.show_status_bar, "Show status bar");
                        ui.add_enabled_ui(prefs.show_status_bar, |ui| {
                            ui.checkbox(&mut prefs.status_fps, "Worker FPS");
                            ui.checkbox(&mut prefs.status_sound, "Sound");
                            ui.checkbox(&mut prefs.status_frame, "Frame counter");
                            ui.checkbox(&mut prefs.status_apu_steps, "APU steps");
                            ui.checkbox(&mut prefs.status_audio_samples, "Audio samples");
                            ui.checkbox(&mut prefs.status_activity, "Frame diagnostics");
                        });
                    });
                });
                ui.menu_button("Audio", |ui| {
                    let mut changed = false;
                    changed |= ui
                        .checkbox(&mut self.audio_enabled, "Sound enabled")
                        .changed();
                    changed |= ui.checkbox(&mut self.audio_muted, "Mute").changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut self.audio_volume, 0.0..=1.0).text("Volume"))
                        .changed();
                    if changed {
                        if let Err(error) = self.engine.configure_sound(
                            self.audio_enabled,
                            self.audio_muted,
                            self.audio_volume,
                        ) {
                            self.last_error = Some(error);
                        }
                    }
                    if ui
                        .add_enabled(
                            self.audio_enabled && !self.audio_muted && self.audio_volume > 0.0,
                            egui::Button::new("Test speakers (440 Hz)"),
                        )
                        .clicked()
                    {
                        match self.engine.queue_speaker_check() {
                            Ok(reply) => {
                                self.pending.push(PendingOperation::SpeakerCheck {
                                    epoch: self.session_epoch,
                                    reply,
                                });
                            }
                            Err(error) => self.last_error = Some(error),
                        }
                    }
                    ui.separator();
                    ui.label("Pitch");
                    let mut pitch_changed = false;
                    pitch_changed |= ui
                        .selectable_value(
                            &mut self.pitch_mode,
                            AudioPitchMode::Preserve,
                            "Preserve original pitch",
                        )
                        .changed();
                    pitch_changed |= ui
                        .selectable_value(
                            &mut self.pitch_mode,
                            AudioPitchMode::FollowSpeed,
                            "Follow game speed",
                        )
                        .changed();
                    pitch_changed |= ui
                        .selectable_value(
                            &mut self.pitch_mode,
                            AudioPitchMode::Custom,
                            "Custom pitch",
                        )
                        .changed();
                    if self.pitch_mode == AudioPitchMode::Custom {
                        pitch_changed |= ui
                            .add(
                                egui::Slider::new(&mut self.pitch_percent, 50..=200)
                                    .text("Pitch")
                                    .suffix("%"),
                            )
                            .changed();
                    }
                    if pitch_changed {
                        self.apply_playback(self.active_speed_percent, true);
                    }
                });
                ui.menu_button("Speed", |ui| {
                    for speed in [50_u16, 75, 100, 125, 150, 200, 300] {
                        if ui
                            .selectable_value(
                                &mut self.speed_percent,
                                speed,
                                format!("{:.2}x", f32::from(speed) / 100.0),
                            )
                            .changed()
                        {
                            // Preserve held Tab fast-forward if the user
                            // adjusts the normal speed from the menu.
                            if !self.fast_forward_active {
                                self.apply_playback(speed, false);
                            }
                            ui.close();
                        }
                    }
                });
                ui.label("Threaded preview");
                ui.small(self.loaded_rom_path.as_ref().map_or_else(
                    || "No game loaded".to_owned(),
                    |path| path.display().to_string(),
                ));
            });
        });
        if focused
            && self.can_modify_running_game()
            && ctx.input(|input| input.key_pressed(egui::Key::F5))
        {
            self.queue_save(SessionSaveAction::QuickSave);
        }
        if focused
            && self.can_modify_running_game()
            && self.controls.has_quick_save
            && ctx.input(|input| input.key_pressed(egui::Key::F8))
        {
            self.queue_save(SessionSaveAction::QuickLoad);
        }

        if focused
            && self.can_modify_running_game()
            && ctx.input(|input| input.key_pressed(egui::Key::F12))
        {
            self.queue_screenshot();
        }
        // The worker may finish an intermediate cartridge while a newer
        // request is already queued. Do not paint transient frames before
        // its launch acknowledgement has been examined.
        if !self
            .pending
            .iter()
            .any(|op| matches!(op, PendingOperation::LoadRom { .. }))
        {
            self.display.poll(ctx, &self.engine);
        }
        self.poll_operations(ctx);
        if self.repaint_gap_smoke_pending
            && self.engine.completed_frames() >= 10
            && self.display.texture().is_some()
        {
            self.repaint_gap_smoke_pending = false;
            self.repaint_gap_challenge = Some((
                Instant::now() + Duration::from_secs(2),
                self.engine.completed_frames(),
            ));
            ctx.request_repaint_after(Duration::from_secs(2));
            return;
        }
        // Acceptance requires both worker progress and a real texture upload
        // through the window renderer, not just counting hidden ticks.
        if let Some(limit) = self.exit_after_frames {
            let rendered = self.display.status().map_or(0, |status| status.frame);
            if rendered >= limit && self.display.texture().is_some() {
                if self.exit_after_library_play {
                    println!(
                        "STARBYTE_THREADED_LIBRARY_PLAY_SMOKE_OK worker_frames={} rendered_frame={rendered}",
                        self.engine.completed_frames()
                    );
                } else {
                    println!(
                        "STARBYTE_THREADED_PREVIEW_SMOKE_OK worker_frames={} rendered_frame={rendered}",
                        self.engine.completed_frames()
                    );
                }
                self.exit_after_frames = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let mut selected = None;
        if self.show_library {
            egui::SidePanel::left("worker_library")
                .resizable(true)
                .default_width(280.0)
                .min_width(200.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("SNES Library");
                        if ui.small_button("Refresh").clicked() {
                            self.refresh_library();
                        }
                        if ui.small_button("Close").clicked() {
                            self.show_library = false;
                        }
                    });
                    ui.small(&self.library_status);
                    let query_changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.library_query)
                                .hint_text("Search installed games..."),
                        )
                        .changed();
                    if query_changed {
                        if let Some(snapshot) = &self.library_snapshot {
                            self.library_matches =
                                indexed_installed_matches(snapshot, &self.library_query);
                        }
                    }
                    if let Some(snapshot) = &self.library_snapshot {
                        ui.label(format!("{} installed games", snapshot.installed_count));
                        ui.separator();
                        let matches = &self.library_matches;
                        egui::ScrollArea::vertical().show_rows(
                            ui,
                            28.0,
                            matches.len(),
                            |ui, visible_rows| {
                                for row in visible_rows {
                                    let entry = &snapshot.entries[matches[row]];
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 24.0],
                                            egui::Button::new(&entry.display_title),
                                        )
                                        .clicked()
                                    {
                                        selected = Some(entry.clone());
                                    }
                                }
                            },
                        );
                    } else {
                        ui.label("Scanning local library...");
                    }
                });
        }
        if let Some(entry) = selected {
            self.queue_library_entry(entry);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let bounds = ui.max_rect();
            ui.painter().rect_filled(bounds, 0.0, Color32::BLACK);
            if let Some(error) = &self.last_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            } else if let Some(message) = &self.status_note {
                ui.label(message);
            }
            let available = ui.available_size();
            if self.controls.session.has_rom {
                self.display.draw(ui, available, self.integer_scale);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Open a ROM or select an installed library game");
                });
            }
            if self.show_input_diagnostics {
                let diag = self.display.input_diagnostics();
                ui.painter().text(
                    bounds.left_top() + egui::vec2(12.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "SNES host {:04X} | latched {:04X} | reading {}\nROM cheat hits {}",
                        diag.host_bits, diag.latched_bits, diag.reading, diag.rom_cheat_read_hits
                    ),
                    egui::TextStyle::Monospace.resolve(ui.style()),
                    Color32::LIGHT_YELLOW,
                );
            }
        });
        if self.status_settings.show_status_bar {
            egui::TopBottomPanel::bottom("worker_preview_stats").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let prefs = &self.status_settings;
                    let rate = self.engine.frame_stats();
                    if prefs.status_fps {
                        if let Some(fps) = rate.fps {
                            ui.label(format!(
                                "Worker FPS {fps:.1} / {:.1}",
                                60.0985 * f32::from(self.active_speed_percent) / 100.0
                            ));
                        } else {
                            ui.label("Worker FPS: measuring");
                        }
                    }
                    if prefs.status_sound {
                        ui.separator();
                        let sound = if !self.audio_enabled {
                            "Sound: disabled"
                        } else if self.audio_muted || self.audio_volume <= 0.0 {
                            "Sound: muted"
                        } else {
                            "Sound: enabled"
                        };
                        let audio = self.display.audio_diagnostics();
                        let status = audio
                            .device_status
                            .map(|value| value.label())
                            .unwrap_or("not_attempted");
                        ui.label(format!(
                            "{sound} | device {status} | submitted {} (nonzero {})",
                            audio.submitted_frames, audio.submitted_nonzero_frames
                        ))
                        .on_hover_text(format!(
                            "Producer drops {} | worker drops {} | empty sink {}. \
Mixer delivery does not establish speaker audibility.",
                            audio.producer_dropped_frames,
                            audio.worker_dropped_frames,
                            audio.sink_empty_on_pcm
                        ));
                    }
                    if let Some(status) = self.display.status() {
                        if prefs.status_frame {
                            ui.separator();
                            ui.label(format!("Frame {}", status.frame));
                        }
                        if prefs.status_apu_steps {
                            ui.separator();
                            ui.label(format!("APU steps {}", status.apu_steps));
                        }
                        if prefs.status_audio_samples {
                            ui.separator();
                            ui.label(format!("Audio samples {}", status.audio_sample_count));
                        }
                    }
                    if prefs.status_activity {
                        ui.separator();
                        ui.label(format!(
                            "CPU {:.2} ms | gap {:.1} ms | hitches {} | superseded {}",
                            self.display.last_cpu_duration().as_secs_f32() * 1_000.0,
                            rate.last_gap_ms,
                            rate.hitches_40ms,
                            self.engine.skipped_video_frames()
                        ));
                    }
                });
            });
        }
        // UI painting is still paced by egui, but the worker is NOT.
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
