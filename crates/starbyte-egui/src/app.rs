use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};
use gilrs::{EventType, GamepadId, Gilrs};
use image::ImageReader;
use tracing::{debug, info, warn};

use crate::{
    logging::SharedLogBuffer,
    worker::{AppWorker, WorkerCommand, WorkerCommandKind, WorkerEvent},
};

use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, InputDeviceMode, LibraryViewMode, RuntimeConfig},
};
use starbyte_frontend::{
    FrontendSession, InstalledStatus, LibraryEntry, LibraryFilter, LibraryService, LibrarySnapshot,
    LibraryTarget,
};

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const COMPACT_LAYOUT_WIDTH: f32 = 960.0;

fn is_compact_layout(width: f32, height: f32) -> bool {
    width < COMPACT_LAYOUT_WIDTH || height < 640.0
}

/// Track real emulation-frame runtime, excluding texture uploads/UI rendering.
#[derive(Debug, Default, Clone, Copy)]
struct FramePerformance {
    samples: u64,
    avg_ms: f32,
    last_ms: f32,
    peak_ms: f32,
    slow_frames: u64,
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
        self.next_frame_at = now + FRAME_INTERVAL;
        true
    }

    fn time_until_next_frame(&self, now: Instant) -> Duration {
        self.next_frame_at.saturating_duration_since(now)
    }
}

#[derive(Debug, Clone)]
struct JobRecord {
    id: u64,
    label: String,
    state: &'static str,
    detail: String,
}

pub struct StarbyteApp {
    assets: AssetConfig,
    config: RuntimeConfig,
    cache_root: PathBuf,
    session: FrontendSession,
    worker: AppWorker,
    library_snapshot: LibrarySnapshot,
    framebuffer_texture: Option<TextureHandle>,
    cover_textures: BTreeMap<String, TextureHandle>,
    failed_cover_ids: BTreeSet<String>,
    held_input: ControllerState,
    status_line: String,
    search_query: String,
    selected_game_id: Option<String>,
    loaded_game_id: Option<String>,
    show_properties: bool,
    rom_dir_input: String,
    logs: SharedLogBuffer,
    jobs: Vec<JobRecord>,
    next_job_id: u64,
    gilrs: Option<Gilrs>,
    gamepad_buttons_down: BTreeSet<String>,
    gamepad_buttons_by_id: HashMap<GamepadId, BTreeSet<String>>,
    pending_keyboard_bind: Option<String>,
    pending_gamepad_bind: Option<String>,
    is_playing: bool,
    play_view: bool,
    frame_clock: FrameClock,
    frame_performance: FramePerformance,
    show_performance_overlay: bool,
    pending_step_frames: u32,
    last_sram_flush: Instant,
    show_compact_settings: bool,
    show_compact_session: bool,
    show_compact_logs: bool,
}

impl StarbyteApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets: AssetConfig,
        mut config: RuntimeConfig,
        rom_path: Option<PathBuf>,
        startup_rom_dirs: Vec<PathBuf>,
        prefer_dark_mode_override: Option<bool>,
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
        if let Some(path) = rom_path {
            session.load_rom(&path)?;
            record_recent_rom(&mut config.library.recent_roms, path.as_path());
            let _ = session.run_frame();
            status_line = format!("Loaded {}", path.display());
        }

        let start_playing = session.snapshot().has_rom;
        let worker = AppWorker::spawn(assets.clone());
        let gilrs = Gilrs::new().ok();
        let cached_snapshot = LibraryService::new(config.clone(), assets.clone())
            .ok()
            .and_then(|service| service.load_cached_snapshot().ok().flatten());
        let status_line = if cached_snapshot.is_some() {
            "Loaded cached library snapshot.".to_owned()
        } else {
            status_line
        };
        let mut app = Self {
            assets,
            config,
            cache_root,
            session,
            worker,
            library_snapshot: cached_snapshot.unwrap_or_else(empty_snapshot),
            framebuffer_texture: None,
            cover_textures: BTreeMap::new(),
            failed_cover_ids: BTreeSet::new(),
            held_input: ControllerState::default(),
            status_line,
            search_query: String::new(),
            selected_game_id: None,
            loaded_game_id: None,
            show_properties: false,
            rom_dir_input: String::new(),
            logs,
            jobs: Vec::new(),
            next_job_id: 1,
            gilrs,
            gamepad_buttons_down: BTreeSet::new(),
            gamepad_buttons_by_id: HashMap::new(),
            pending_keyboard_bind: None,
            pending_gamepad_bind: None,
            is_playing: start_playing,
            play_view: start_playing,
            frame_clock: FrameClock::new(Instant::now()),
            frame_performance: FramePerformance::default(),
            show_performance_overlay: false,
            pending_step_frames: 0,
            last_sram_flush: Instant::now(),
            show_compact_settings: false,
            show_compact_session: false,
            show_compact_logs: false,
        };
        app.persist_config();
        if app.config.advanced.refresh_on_startup {
            app.queue_job(WorkerCommandKind::RefreshMetadata);
        } else if app.library_snapshot.total_count == 0 && app.library_snapshot.entries.is_empty() {
            app.queue_job(WorkerCommandKind::RefreshSnapshot);
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
                    // Background metadata refresh must not erase games opened
                    // since the asynchronous refresh began.
                    let recent_roms = self.config.library.recent_roms.clone();
                    self.config = config;
                    self.config.library.recent_roms = recent_roms;
                    self.cache_root = resolve_cache_root(&self.config, &self.assets);
                    self.library_snapshot = snapshot;
                    self.update_snapshot_cheat_flags();
                    self.sync_loaded_game_cheats();
                    self.update_job(job_id, "Library", "done", &status);
                    self.status_line = status;
                }
                WorkerEvent::RomReady {
                    job_id,
                    entry,
                    rom_path,
                } => match self.session.load_rom(&rom_path) {
                    Ok(()) => {
                        self.loaded_game_id = Some(entry.game_id.clone());
                        let _ = self.session.set_active_cheats(&entry.cheats);
                        self.remember_recent_rom(&rom_path);
                        let _ = self.session.run_frame();
                        self.refresh_framebuffer(ctx);
                        self.is_playing = true;
                        self.play_view = true;
                        self.frame_clock.reset(Instant::now());
                        self.frame_performance = FramePerformance::default();
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
                cheat.enabled = enabled.iter().any(|value| value == &cheat.id);
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
        // Do not trigger in-game buttons while typing into the library search or settings.
        if self.config.input.active_device == InputDeviceMode::Keyboard
            && ctx.wants_keyboard_input()
        {
            return ControllerState::default();
        }
        let mut state = self.held_input;
        match self.config.input.active_device {
            InputDeviceMode::Keyboard => {
                for &action in input_actions() {
                    if let Some(binding) = self.config.input.keyboard_bindings.get(action)
                        && let Some(key) = parse_egui_key(binding)
                        && ctx.input(|input| input.key_down(key))
                    {
                        set_controller_flag(&mut state, action, true);
                    }
                }
            }
            InputDeviceMode::Gamepad => {
                for &action in input_actions() {
                    if let Some(binding) = self.config.input.gamepad_bindings.get(action)
                        && self.gamepad_buttons_down.contains(binding)
                    {
                        set_controller_flag(&mut state, action, true);
                    }
                }
            }
        }
        state
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

    fn poll_gamepad_events(&mut self) {
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
                    if let Some(action) = self.pending_gamepad_bind.clone() {
                        new_binding = Some((action, name));
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(buttons) = self.gamepad_buttons_by_id.get_mut(&event.id) {
                        buttons.remove(&format!("{button:?}"));
                    }
                }
                EventType::Disconnected => {
                    // A controller cannot emit release events after unplugging.
                    // Clear only its held buttons, not those on other pads.
                    self.gamepad_buttons_by_id.remove(&event.id);
                }
                _ => {}
            }
        }
        self.gamepad_buttons_down = merged_gamepad_buttons(&self.gamepad_buttons_by_id);
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

    fn run_frame(&mut self, ctx: &egui::Context) {
        self.session
            .set_controller1(self.effective_controller_state(ctx));
        let started = Instant::now();
        match self.session.run_frame() {
            Ok(()) => {
                self.frame_performance.record(started.elapsed());
                self.refresh_framebuffer(ctx);
                self.status_line = self.session.snapshot().status_line();
            }
            Err(error) => {
                warn!("{error}");
                self.status_line = error.to_string();
                self.is_playing = false;
                self.pending_step_frames = 0;
            }
        }
    }

    fn remember_recent_rom(&mut self, path: &Path) {
        record_recent_rom(&mut self.config.library.recent_roms, path);
        self.persist_config();
    }

    fn open_recent_rom(&mut self, path: &Path, ctx: &egui::Context) {
        match self.session.load_rom(path) {
            Ok(()) => {
                let matching = self
                    .library_snapshot
                    .entries
                    .iter()
                    .find(|entry| {
                        entry.local.as_ref().is_some_and(|local| {
                            local.rom_path.as_path() == path
                                || local.extracted_cache_path.as_deref() == Some(path)
                        })
                    })
                    .cloned();
                self.loaded_game_id = matching.as_ref().map(|entry| entry.game_id.clone());
                if let Some(entry) = matching {
                    let _ = self.session.set_active_cheats(&entry.cheats);
                }
                self.remember_recent_rom(path);
                if let Err(error) = self.session.run_frame() {
                    self.status_line = error.to_string();
                    self.is_playing = false;
                    return;
                }
                self.refresh_framebuffer(ctx);
                self.is_playing = true;
                self.play_view = true;
                self.pending_step_frames = 0;
                self.frame_performance = FramePerformance::default();
                self.frame_clock.reset(Instant::now());
                self.last_sram_flush = Instant::now();
                self.status_line = format!("Loaded {}", path.display());
            }
            Err(error) => {
                self.status_line = format!("Could not load recent game: {error}");
            }
        }
    }

    fn queue_load_entry(&mut self, entry: &LibraryEntry) {
        if entry.installed_status == InstalledStatus::Missing {
            self.status_line = format!("{} is not installed locally.", entry.display_title);
            return;
        }
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
                if ui
                    .checkbox(&mut self.show_performance_overlay, "Timing")
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
                        self.open_recent_rom(&path, ctx);
                        ui.close();
                    }
                }
            });
            if ui
                .add_enabled(
                    has_rom,
                    egui::Button::new(if self.is_playing { "Pause" } else { "Play" }),
                )
                .clicked()
            {
                self.is_playing = !self.is_playing;
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
            ui.separator();

            ui.label("ROM Directories");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.rom_dir_input);
                if ui.button("Add").clicked() {
                    let path = PathBuf::from(self.rom_dir_input.trim());
                    if !self.rom_dir_input.trim().is_empty()
                        && !self.config.library.rom_dirs.contains(&path)
                    {
                        self.config.library.rom_dirs.push(path);
                        self.rom_dir_input.clear();
                        self.persist_config();
                        self.queue_job(WorkerCommandKind::RefreshSnapshot);
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
            egui::CollapsingHeader::new("Audio (output not connected yet)")
                .default_open(true)
                .show(ui, |ui| {
                    let audio = &mut self.config.audio;
                    let mut changed = false;
                    changed |= ui.checkbox(&mut audio.enabled, "Enabled").changed();
                    changed |= ui
                        .checkbox(&mut audio.mute_on_startup, "Mute on startup")
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut audio.volume, 0.0..=1.0).text("Volume"))
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut audio.sample_rate_hz)
                                .speed(1_000)
                                .range(8_000..=96_000)
                                .prefix("Hz "),
                        )
                        .changed();
                    if changed {
                        self.persist_config();
                    }
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
        if entry.cheats.is_empty() {
            ui.label("No cached cheats for this title.");
        } else {
            for cheat in entry.cheats {
                let mut enabled = cheat.enabled;
                if ui.checkbox(&mut enabled, cheat.name.as_str()).changed() {
                    self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                }
                ui.label(RichText::new(cheat.code).small());
            }
        }
    }

    fn draw_input_settings(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("Active Device")
            .selected_text(match self.config.input.active_device {
                InputDeviceMode::Keyboard => "Keyboard",
                InputDeviceMode::Gamepad => "Gamepad",
            })
            .show_ui(ui, |ui| {
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

        if ui.button("Save Input").clicked() {
            self.persist_config();
        }
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
                }
            });
        }
        if let Some(action) = &self.pending_gamepad_bind {
            ui.label(format!(
                "Press a gamepad button to bind {}...",
                action_label(action)
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
                self.refresh_framebuffer(ctx);
                self.frame_clock.reset(Instant::now());
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
                self.refresh_framebuffer(ctx);
                self.frame_clock.reset(Instant::now());
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
        if self.show_performance_overlay && self.frame_performance.samples > 0 {
            let stats = self.frame_performance;
            let label = format!(
                "Emulation {:.1} ms avg | {:.1} ms peak | {}/{} over budget",
                stats.avg_ms, stats.peak_ms, stats.slow_frames, stats.samples,
            );
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
            ui.label(format!(
                "Emulation: {:.1} ms/frame avg; {:.1} ms peak; {} of {} frames over 16.7 ms",
                self.frame_performance.avg_ms,
                self.frame_performance.peak_ms,
                self.frame_performance.slow_frames,
                self.frame_performance.samples,
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
        const CARD_HEIGHT: f32 = 292.0;
        const COVER_BOX: Vec2 = Vec2::new(164.0, 190.0);

        ui.allocate_ui_with_layout(
            Vec2::new(CARD_WIDTH, CARD_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let inner = egui::Frame::group(ui.style()).show(ui, |ui| {
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
                if entry.cheats.is_empty() {
                    ui.label("No cheats cached for this title.");
                } else {
                    for cheat in entry.cheats {
                        let mut enabled = cheat.enabled;
                        if ui.checkbox(&mut enabled, cheat.name.as_str()).changed() {
                            self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                        }
                        ui.label(RichText::new(cheat.code).small());
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
        apply_theme(ctx, self.config.prefer_dark_mode);
        self.capture_keyboard_binding(ctx);
        self.poll_gamepad_events();
        self.poll_worker_events(ctx);
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
        if self.play_view {
            egui::CentralPanel::default().show(ctx, |ui| self.draw_play_view(ui));
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

        if self.is_playing && self.session.snapshot().has_rom {
            self.pending_step_frames = 0;
            if self.frame_clock.take_due_frame(Instant::now()) {
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
        FRAME_INTERVAL, FrameClock, FramePerformance, Vec2, fit_game_size, is_compact_layout,
        merged_gamepad_buttons, record_recent_rom, write_png_screenshot,
    };

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

fn merged_gamepad_buttons<K: Eq + std::hash::Hash>(
    by_controller: &HashMap<K, BTreeSet<String>>,
) -> BTreeSet<String> {
    by_controller
        .values()
        .flat_map(|buttons| buttons.iter().cloned())
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
