use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::Result;
use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};
use image::ImageReader;
use tracing::{debug, info, warn};

use crate::{
    logging::SharedLogBuffer,
    worker::{AppWorker, WorkerCommand, WorkerCommandKind, WorkerEvent},
};

use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, LibraryViewMode, RuntimeConfig},
};
use starbyte_frontend::{
    FrontendSession, InstalledStatus, LibraryEntry, LibraryFilter, LibrarySnapshot, LibraryTarget,
};

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
            let _ = session.run_frame();
            status_line = format!("Loaded {}", path.display());
        }

        let worker = AppWorker::spawn(assets.clone());
        let mut app = Self {
            assets,
            config,
            cache_root,
            session,
            worker,
            library_snapshot: empty_snapshot(),
            framebuffer_texture: None,
            cover_textures: BTreeMap::new(),
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
        };
        app.persist_config();
        if app.config.advanced.refresh_on_startup {
            app.queue_job(WorkerCommandKind::RefreshMetadata);
        } else {
            app.queue_job(WorkerCommandKind::RefreshSnapshot);
        }
        Ok(app)
    }

    fn current_filter(&self) -> LibraryFilter {
        LibraryFilter {
            query: self.search_query.clone(),
            installed_only: self.config.library.show_installed_only,
            view_mode: self.config.library.active_view,
        }
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
                    self.config = config;
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
                        let _ = self.session.run_frame();
                        self.refresh_framebuffer(ctx);
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
        self.library_snapshot
            .entries
            .iter()
            .find(|entry| entry.game_id == selected)
            .cloned()
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

    fn run_frame(&mut self, ctx: &egui::Context) {
        self.session.set_controller1(self.held_input);
        match self.session.run_frame() {
            Ok(()) => {
                self.refresh_framebuffer(ctx);
                self.status_line = self.session.snapshot().status_line();
            }
            Err(error) => {
                warn!("{error}");
                self.status_line = error.to_string();
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
        let cover = entry.cover.as_ref()?;
        let Ok(reader) = ImageReader::open(&cover.cache_path) else {
            warn!("failed to open cached cover {}", cover.cache_path.display());
            return None;
        };
        let Ok(image) = reader.decode() else {
            warn!("failed to decode cached cover {}", cover.cache_path.display());
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

    fn draw_top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Starbyte");
            ui.label("Library-first frontend");
            ui.separator();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search library"),
                )
                .changed()
            {
                self.queue_job(WorkerCommandKind::RefreshSnapshot);
            }

            if ui
                .checkbox(&mut self.config.library.show_installed_only, "Installed only")
                .changed()
            {
                self.persist_config();
                self.queue_job(WorkerCommandKind::RefreshSnapshot);
            }

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

            if ui.button("Refresh Metadata").clicked() {
                self.queue_job(WorkerCommandKind::RefreshMetadata);
            }
            if ui.button("Refresh Covers").clicked() {
                self.queue_job(WorkerCommandKind::RefreshCovers {
                    target: LibraryTarget::default(),
                });
            }
            if ui.button("Refresh Cheats").clicked() {
                self.queue_job(WorkerCommandKind::RefreshCheats {
                    target: LibraryTarget::default(),
                });
            }
            if ui.button("Refresh All").clicked() {
                self.queue_job(WorkerCommandKind::RefreshAll);
            }
            if ui.checkbox(&mut self.config.prefer_dark_mode, "Night Mode").changed() {
                apply_theme(ctx, self.config.prefer_dark_mode);
                self.persist_config();
            }

            ui.separator();
            ui.checkbox(&mut self.config.ui.show_left_panel, "Left");
            ui.checkbox(&mut self.config.ui.show_details_panel, "Details");
            ui.checkbox(&mut self.config.ui.show_right_panel, "Session");
            ui.checkbox(&mut self.config.ui.show_log_panel, "Logs");
            if ui.button("Save Layout").clicked() {
                self.persist_config();
            }
        });
    }

    fn draw_settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.label(format!(
            "Showing {} of {} entries",
            self.library_snapshot.entries.len(),
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
        egui::CollapsingHeader::new("Audio")
            .default_open(true)
            .show(ui, |ui| {
                let audio = &mut self.config.audio;
                let mut changed = false;
                changed |= ui.checkbox(&mut audio.enabled, "Enabled").changed();
                changed |= ui.checkbox(&mut audio.mute_on_startup, "Mute on startup").changed();
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
                changed |= ui.checkbox(&mut video.fullscreen, "Fullscreen").changed();
                changed |= ui.checkbox(&mut video.integer_scale, "Integer scale").changed();
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
                ui.label("Controller 1");
                input_checkbox(ui, &mut self.held_input.up, "Up");
                input_checkbox(ui, &mut self.held_input.down, "Down");
                input_checkbox(ui, &mut self.held_input.left, "Left");
                input_checkbox(ui, &mut self.held_input.right, "Right");
                input_checkbox(ui, &mut self.held_input.start, "Start");
                input_checkbox(ui, &mut self.held_input.select, "Select");
                input_checkbox(ui, &mut self.held_input.a, "A");
                input_checkbox(ui, &mut self.held_input.b, "B");
                input_checkbox(ui, &mut self.held_input.x, "X");
                input_checkbox(ui, &mut self.held_input.y, "Y");
                input_checkbox(ui, &mut self.held_input.l, "L");
                input_checkbox(ui, &mut self.held_input.r, "R");
            });

        egui::CollapsingHeader::new("Cheats")
            .default_open(false)
            .show(ui, |ui| {
                if ui
                    .checkbox(&mut self.config.cheats.show_cheat_badges, "Show cheat badges")
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
                    .checkbox(&mut advanced.providers.enable_network, "Enable network providers")
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
                if metadata.has_cheat_files { "yes" } else { "no" }
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

    fn draw_session_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Session");
        let snapshot = self.session.snapshot();
        if let Some(path) = &snapshot.rom_path {
            ui.label(path.display().to_string());
        } else {
            ui.label("No ROM selected");
        }
        if ui.button("Run Frame").clicked() {
            self.run_frame(ctx);
        }
        if ui.button("Run 60 Frames").clicked() {
            self.session.set_controller1(self.held_input);
            match self.session.run_frames(60) {
                Ok(()) => {
                    self.refresh_framebuffer(ctx);
                    self.status_line = self.session.snapshot().status_line();
                }
                Err(error) => self.status_line = error.to_string(),
            }
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
        let entries = self.library_snapshot.entries.clone();
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
        ui.group(|ui| {
            ui.set_width(200.0);
            if let Some(texture) = self.ensure_cover_texture(ctx, entry) {
                let size = fit_size(texture.size_vec2(), Vec2::new(180.0, 180.0));
                ui.add(egui::Image::new((texture.id(), size)));
            } else {
                ui.allocate_space(Vec2::new(180.0, 120.0));
                ui.label("No cover");
            }

            let response = ui.selectable_label(
                self.selected_game_id.as_deref() == Some(entry.game_id.as_str()),
                entry.display_title.as_str(),
            );
            if response.clicked() {
                self.selected_game_id = Some(entry.game_id.clone());
            }
            if response.double_clicked() {
                self.queue_load_entry(entry);
            }
            ui.label(match entry.installed_status {
                InstalledStatus::Installed => "Installed",
                InstalledStatus::Missing => "Missing",
            });
            response.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
        });
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
                starbyte_frontend::LocalRomSourceKind::File => local.rom_path.parent().map(Path::to_path_buf),
                starbyte_frontend::LocalRomSourceKind::ZipArchiveMember => local.rom_path.parent().map(Path::to_path_buf),
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

    fn draw_log_panel(&mut self, ctx: &egui::Context) {
        if !self.config.ui.show_log_panel {
            return;
        }

        let response = egui::TopBottomPanel::bottom("logs")
            .resizable(true)
            .default_height(self.config.ui.log_panel_height)
            .min_height(120.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Logs");
                    if ui.button("Open Log Folder").clicked() {
                        let _ = open_path(&self.cache_root.join("logs"));
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
            });
        self.config.ui.log_panel_height = response.response.rect.height();
    }
}

impl eframe::App for StarbyteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker_events(ctx);

        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| self.draw_top_bar(ui, ctx));
        self.draw_log_panel(ctx);

        if self.config.ui.show_left_panel {
            let response = egui::SidePanel::left("settings")
                .resizable(true)
                .default_width(self.config.ui.left_panel_width)
                .min_width(240.0)
                .show(ctx, |ui| self.draw_settings_panel(ui));
            self.config.ui.left_panel_width = response.response.rect.width();
        }

        if self.config.ui.show_right_panel {
            let response = egui::SidePanel::right("session")
                .resizable(true)
                .default_width(self.config.ui.right_panel_width)
                .min_width(280.0)
                .show(ctx, |ui| self.draw_session_panel(ui, ctx));
            self.config.ui.right_panel_width = response.response.rect.width();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.config.ui.show_details_panel {
                let response = egui::SidePanel::right("details")
                    .resizable(true)
                    .default_width(self.config.ui.details_panel_width)
                    .min_width(260.0)
                    .show_inside(ui, |ui| self.draw_details_panel(ui, ctx));
                self.config.ui.details_panel_width = response.response.rect.width();
            }

            self.draw_library_browser(ui, ctx);
        });

        if self.show_properties {
            self.draw_properties_window(ctx);
        }

        ctx.request_repaint_after(Duration::from_millis(100));
    }
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

fn fit_size(source: Vec2, available: Vec2) -> Vec2 {
    if source.x <= 0.0 || source.y <= 0.0 {
        return source;
    }
    let scale = (available.x / source.x)
        .min(available.y / source.y)
        .clamp(0.1, 4.0);
    Vec2::new(source.x * scale, source.y * scale)
}

fn input_checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    ui.checkbox(value, label);
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
