use std::{collections::BTreeMap, path::{Path, PathBuf}, process::Command};

use anyhow::Result;
use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};
use image::ImageReader;
use tracing::warn;

use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, LibraryViewMode, RuntimeConfig},
};
use starbyte_frontend::{
    FrontendSession, InstalledStatus, LibraryEntry, LibraryFilter, LibraryService, LibraryTarget,
};

pub struct StarbyteApp {
    session: FrontendSession,
    library_service: LibraryService,
    library_snapshot: starbyte_frontend::LibrarySnapshot,
    framebuffer_texture: Option<TextureHandle>,
    cover_textures: BTreeMap<String, TextureHandle>,
    prefer_dark_mode: bool,
    status_line: String,
    held_input: ControllerState,
    search_query: String,
    selected_game_id: Option<String>,
    loaded_game_id: Option<String>,
    show_properties: bool,
    rom_dir_input: String,
}

impl StarbyteApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets: AssetConfig,
        rom_path: Option<PathBuf>,
        startup_rom_dirs: Vec<PathBuf>,
        prefer_dark_mode: bool,
    ) -> Result<Self> {
        let config_path = assets.config_path();
        let mut config = RuntimeConfig::load_or_default(&config_path)?;
        config.prefer_dark_mode = prefer_dark_mode;
        for rom_dir in startup_rom_dirs {
            if !config.library.rom_dirs.contains(&rom_dir) {
                config.library.rom_dirs.push(rom_dir);
            }
        }
        if let Some(path) = &rom_path {
            if let Some(parent) = path.parent() {
                let parent = parent.to_path_buf();
                if !config.library.rom_dirs.contains(&parent) {
                    config.library.rom_dirs.push(parent);
                }
            }
        }

        apply_theme(&cc.egui_ctx, config.prefer_dark_mode);

        let mut library_service = LibraryService::new(config, assets.clone())?;
        if library_service.config().advanced.refresh_on_startup {
            let _ = library_service.refresh_metadata_index();
        }
        let library_snapshot = library_service.snapshot(LibraryFilter {
            query: String::new(),
            installed_only: library_service.config().library.show_installed_only,
            view_mode: library_service.config().library.active_view,
        })?;

        let mut session = FrontendSession::new(assets)?;
        let mut status_line = "No ROM loaded".to_owned();
        let mut loaded_game_id = None;
        if let Some(path) = rom_path {
            session.load_rom(&path)?;
            loaded_game_id = library_snapshot
                .entries
                .iter()
                .find(|entry| entry.local.as_ref().map(|local| &local.rom_path) == Some(&path))
                .map(|entry| entry.game_id.clone());
            if let Some(game_id) = &loaded_game_id {
                if let Some(entry) = library_snapshot
                    .entries
                    .iter()
                    .find(|entry| &entry.game_id == game_id)
                {
                    let _ = session.set_active_cheats(&entry.cheats);
                }
            }
            let _ = session.run_frame();
            status_line = format!("Loaded {}", path.display());
        }

        Ok(Self {
            session,
            library_service,
            library_snapshot,
            framebuffer_texture: None,
            cover_textures: BTreeMap::new(),
            prefer_dark_mode,
            status_line,
            held_input: ControllerState::default(),
            search_query: String::new(),
            selected_game_id: None,
            loaded_game_id,
            show_properties: false,
            rom_dir_input: String::new(),
        })
    }

    fn refresh_snapshot(&mut self) {
        match self.library_service.snapshot(LibraryFilter {
            query: self.search_query.clone(),
            installed_only: self.library_service.config().library.show_installed_only,
            view_mode: self.library_service.config().library.active_view,
        }) {
            Ok(snapshot) => self.library_snapshot = snapshot,
            Err(error) => self.status_line = error.to_string(),
        }
    }

    fn persist_config(&mut self) {
        self.library_service.config_mut().prefer_dark_mode = self.prefer_dark_mode;
        if let Err(error) = self.library_service.save_config() {
            self.status_line = error.to_string();
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

    fn lookup_entry(&self, game_id: &str) -> Option<LibraryEntry> {
        if let Some(entry) = self
            .library_snapshot
            .entries
            .iter()
            .find(|entry| entry.game_id == game_id)
        {
            return Some(entry.clone());
        }

        self.library_service
            .snapshot(LibraryFilter {
                query: String::new(),
                installed_only: false,
                view_mode: self.library_service.config().library.active_view,
            })
            .ok()?
            .entries
            .into_iter()
            .find(|entry| entry.game_id == game_id)
    }

    fn sync_loaded_game_cheats(&mut self) {
        let Some(game_id) = self.loaded_game_id.clone() else {
            self.session.clear_active_cheats();
            return;
        };
        if let Some(entry) = self.lookup_entry(&game_id) {
            let _ = self.session.set_active_cheats(&entry.cheats);
        } else {
            self.session.clear_active_cheats();
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

    fn play_entry(&mut self, entry: &LibraryEntry, ctx: &egui::Context) {
        let Some(local) = &entry.local else {
            self.status_line = format!("{} is not installed locally.", entry.display_title);
            return;
        };

        match self.session.load_rom(&local.rom_path) {
            Ok(()) => {
                self.loaded_game_id = Some(entry.game_id.clone());
                let _ = self.session.set_active_cheats(&entry.cheats);
                let _ = self.session.run_frame();
                self.refresh_framebuffer(ctx);
                self.status_line = format!("Loaded {}", local.rom_path.display());
            }
            Err(error) => self.status_line = error.to_string(),
        }
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
            return None;
        };
        let Ok(image) = reader.decode() else {
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
            .library_service
            .config_mut()
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
        self.persist_config();
        self.refresh_snapshot();
        if self.loaded_game_id.as_deref() == Some(game_id) {
            self.sync_loaded_game_cheats();
        }
    }

    fn refresh_selected_metadata(&mut self) {
        match self.library_service.refresh_metadata_index() {
            Ok(count) => self.status_line = format!("Refreshed metadata index ({count} records)."),
            Err(error) => self.status_line = error.to_string(),
        }
        self.persist_config();
        self.refresh_snapshot();
        self.sync_loaded_game_cheats();
    }

    fn refresh_selected_covers(&mut self, entry: Option<&LibraryEntry>) {
        let target = entry
            .map(|entry| LibraryTarget {
                game_id: Some(entry.game_id.clone()),
                ..LibraryTarget::default()
            })
            .unwrap_or_default();
        match self.library_service.refresh_covers(&target) {
            Ok(count) => self.status_line = format!("Refreshed covers ({count} file(s))."),
            Err(error) => self.status_line = error.to_string(),
        }
        self.refresh_snapshot();
    }

    fn refresh_selected_cheats(&mut self, entry: Option<&LibraryEntry>) {
        let target = entry
            .map(|entry| LibraryTarget {
                game_id: Some(entry.game_id.clone()),
                ..LibraryTarget::default()
            })
            .unwrap_or_default();
        match self.library_service.refresh_cheats(&target) {
            Ok(count) => self.status_line = format!("Refreshed cheats ({count} record(s))."),
            Err(error) => self.status_line = error.to_string(),
        }
        self.persist_config();
        self.refresh_snapshot();
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
                self.refresh_snapshot();
            }

            let installed_only = &mut self.library_service.config_mut().library.show_installed_only;
            if ui.checkbox(installed_only, "Installed only").changed() {
                self.refresh_snapshot();
                self.persist_config();
            }

            egui::ComboBox::from_label("View")
                .selected_text(match self.library_service.config().library.active_view {
                    LibraryViewMode::List => "List",
                    LibraryViewMode::Grid => "Grid",
                    LibraryViewMode::Detailed => "Detailed",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.library_service.config_mut().library.active_view,
                        LibraryViewMode::List,
                        "List",
                    );
                    ui.selectable_value(
                        &mut self.library_service.config_mut().library.active_view,
                        LibraryViewMode::Grid,
                        "Grid",
                    );
                    ui.selectable_value(
                        &mut self.library_service.config_mut().library.active_view,
                        LibraryViewMode::Detailed,
                        "Detailed",
                    );
                });

            if ui.button("Refresh Metadata").clicked() {
                self.refresh_selected_metadata();
            }
            if ui.button("Refresh Covers").clicked() {
                self.refresh_selected_covers(None);
            }
            if ui.button("Refresh Cheats").clicked() {
                self.refresh_selected_cheats(None);
            }
            if ui.button("Refresh All").clicked() {
                match self.library_service.refresh_all(&LibraryTarget::default()) {
                    Ok(summary) => {
                        self.status_line = format!(
                            "Refreshed metadata {}, covers {}, cheats {}.",
                            summary.metadata_records, summary.covers_written, summary.cheat_records
                        );
                        self.persist_config();
                        self.refresh_snapshot();
                    }
                    Err(error) => self.status_line = error.to_string(),
                }
            }
            if ui.button("Properties").clicked() && self.selected_game_id.is_some() {
                self.show_properties = true;
            }
            if ui.checkbox(&mut self.prefer_dark_mode, "Night Mode").changed() {
                apply_theme(ctx, self.prefer_dark_mode);
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
                    && !self.library_service.config().library.rom_dirs.contains(&path)
                {
                    self.library_service.config_mut().library.rom_dirs.push(path);
                    self.rom_dir_input.clear();
                    self.persist_config();
                    self.refresh_snapshot();
                }
            }
            if ui.button("Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder()
                    && !self.library_service.config().library.rom_dirs.contains(&path)
                {
                    self.library_service.config_mut().library.rom_dirs.push(path);
                    self.persist_config();
                    self.refresh_snapshot();
                }
            }
        });

        let mut remove_index = None;
        for (index, rom_dir) in self
            .library_service
            .config()
            .library
            .rom_dirs
            .iter()
            .enumerate()
        {
            ui.horizontal(|ui| {
                ui.label(rom_dir.display().to_string());
                if ui.button("Remove").clicked() {
                    remove_index = Some(index);
                }
            });
        }
        if let Some(index) = remove_index {
            self.library_service.config_mut().library.rom_dirs.remove(index);
            self.persist_config();
            self.refresh_snapshot();
        }

        ui.separator();
        egui::CollapsingHeader::new("Audio")
            .default_open(true)
            .show(ui, |ui| {
                let audio = &mut self.library_service.config_mut().audio;
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
                let video = &mut self.library_service.config_mut().video;
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
                let cheats = &mut self.library_service.config_mut().cheats;
                let changed = ui
                    .checkbox(&mut cheats.show_cheat_badges, "Show cheat badges")
                    .changed();
                if changed {
                    self.persist_config();
                }
            });

        egui::CollapsingHeader::new("Advanced / Cache")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(format!(
                    "Cache Root: {}",
                    self.library_service.cache_root().display()
                ));
                let advanced = &mut self.library_service.config_mut().advanced;
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
                if changed {
                    self.persist_config();
                    self.refresh_snapshot();
                }
            });
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
        match self.library_service.config().library.active_view {
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
        response.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
    }

    fn draw_grid_entry(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, entry: &LibraryEntry) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(180.0);
            if let Some(texture) = self.ensure_cover_texture(ctx, entry) {
                ui.add(egui::Image::new((texture.id(), Vec2::new(140.0, 196.0))));
            } else {
                ui.allocate_ui(Vec2::new(140.0, 196.0), |ui| {
                    ui.centered_and_justified(|ui| ui.label("No Cover"));
                });
            }
            let button = ui.selectable_label(
                self.selected_game_id.as_deref() == Some(entry.game_id.as_str()),
                &entry.display_title,
            );
            if button.clicked() {
                self.selected_game_id = Some(entry.game_id.clone());
            }
            button.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
            ui.label(match entry.installed_status {
                InstalledStatus::Installed => "Present",
                InstalledStatus::Missing => "Missing",
            });
        });
        ui.add_space(8.0);
    }

    fn draw_detailed_entry(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        entry: &LibraryEntry,
    ) {
        ui.horizontal(|ui| {
            if let Some(texture) = self.ensure_cover_texture(ctx, entry) {
                ui.add(egui::Image::new((texture.id(), Vec2::new(96.0, 134.0))));
            }
            ui.vertical(|ui| {
                let label = ui.selectable_label(
                    self.selected_game_id.as_deref() == Some(entry.game_id.as_str()),
                    RichText::new(&entry.display_title).heading(),
                );
                if label.clicked() {
                    self.selected_game_id = Some(entry.game_id.clone());
                }
                label.context_menu(|ui| self.draw_entry_context_menu(ui, ctx, entry));
                ui.label(format!(
                    "Status: {}",
                    match entry.installed_status {
                        InstalledStatus::Installed => "Present",
                        InstalledStatus::Missing => "Missing",
                    }
                ));
                if let Some(local) = &entry.local {
                    ui.label(format!("Mapper: {}", local.mapper));
                    ui.label(format!("Region: {}", local.region));
                    if let Some(coprocessor) = &local.coprocessor {
                        ui.label(format!("Coprocessor: {coprocessor}"));
                    }
                    ui.label(format!("ROM: {}", local.rom_path.display()));
                }
                if let Some(metadata) = &entry.metadata {
                    ui.label(format!("Source: {}", metadata.source));
                }
                ui.label(format!("Cheats: {}", entry.cheats.len()));
            });
        });
    }

    fn draw_entry_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        entry: &LibraryEntry,
    ) {
        if ui.button("Play").clicked() {
            self.selected_game_id = Some(entry.game_id.clone());
            self.play_entry(entry, ctx);
            ui.close();
        }
        if ui.button("Properties").clicked() {
            self.selected_game_id = Some(entry.game_id.clone());
            self.show_properties = true;
            ui.close();
        }
        if ui.button("Refresh Metadata").clicked() {
            self.refresh_selected_metadata();
            ui.close();
        }
        if ui.button("Refresh Cheats").clicked() {
            self.refresh_selected_cheats(Some(entry));
            ui.close();
        }
        if ui.button("Refresh Cover").clicked() {
            self.refresh_selected_covers(Some(entry));
            ui.close();
        }
        if let Some(local) = &entry.local
            && ui.button("Open ROM Folder").clicked()
        {
            let _ = open_path(local.rom_path.parent().unwrap_or(Path::new(".")));
            ui.close();
        }
    }

    fn draw_properties_window(&mut self, ctx: &egui::Context) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let mut open = self.show_properties;
        egui::Window::new("Game Properties")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(&entry.display_title);
                ui.label(format!("Game ID: {}", entry.game_id));
                ui.label(format!(
                    "Status: {}",
                    match entry.installed_status {
                        InstalledStatus::Installed => "Present",
                        InstalledStatus::Missing => "Missing",
                    }
                ));
                if let Some(local) = &entry.local {
                    ui.separator();
                    ui.label(format!("ROM Path: {}", local.rom_path.display()));
                    ui.label(format!("Mapper: {}", local.mapper));
                    ui.label(format!("Region: {}", local.region));
                    ui.label(format!("Size: {} bytes", local.file_size_bytes));
                    if let Some(coprocessor) = &local.coprocessor {
                        ui.label(format!("Coprocessor: {coprocessor}"));
                    }
                }
                if let Some(metadata) = &entry.metadata {
                    ui.separator();
                    ui.label(format!("Metadata Source: {}", metadata.source));
                    ui.label(format!("Fetched: {}", metadata.fetched_at_unix));
                    ui.label(format!(
                        "Cover URL: {}",
                        metadata.cover_url.clone().unwrap_or_else(|| "none".to_owned())
                    ));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Play").clicked() {
                        self.play_entry(&entry, ctx);
                    }
                    if ui.button("Refresh Cheats").clicked() {
                        self.refresh_selected_cheats(Some(&entry));
                    }
                    if ui.button("Refresh Cover").clicked() {
                        self.refresh_selected_covers(Some(&entry));
                    }
                });
                ui.separator();
                ui.heading("Cheats");
                if entry.cheats.is_empty() {
                    ui.label("No cached cheats. Use Refresh Cheats to download them.");
                } else {
                    for cheat in entry.cheats {
                        let mut enabled = cheat.enabled;
                        if ui.checkbox(&mut enabled, cheat.name).changed() {
                            self.toggle_cheat(&entry.game_id, &cheat.id, enabled);
                        }
                        ui.label(RichText::new(cheat.code).small());
                    }
                }
            });
        self.show_properties = open;
    }
}

impl eframe::App for StarbyteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| self.draw_top_bar(ui, ctx));

        egui::SidePanel::left("settings")
            .resizable(true)
            .min_width(250.0)
            .show(ctx, |ui| self.draw_settings_panel(ui));

        egui::SidePanel::right("session")
            .resizable(true)
            .min_width(280.0)
            .show(ctx, |ui| self.draw_session_panel(ui, ctx));

        egui::CentralPanel::default().show(ctx, |ui| self.draw_library_browser(ui, ctx));

        if self.show_properties {
            self.draw_properties_window(ctx);
        }
    }
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
