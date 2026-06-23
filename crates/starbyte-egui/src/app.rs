use std::path::PathBuf;

use anyhow::Result;
use eframe::egui::{
    self, ColorImage, RichText, TextureHandle, TextureOptions, Vec2,
};
use tracing::warn;

use starbyte_core::input::ControllerState;
use starbyte_core::manifest::AssetConfig;

use crate::session::EmulatorSession;

pub struct StarbyteApp {
    session: EmulatorSession,
    framebuffer_texture: Option<TextureHandle>,
    prefer_dark_mode: bool,
    status_line: String,
    held_input: ControllerState,
}

impl StarbyteApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets: AssetConfig,
        rom_path: Option<PathBuf>,
        prefer_dark_mode: bool,
    ) -> Result<Self> {
        apply_theme(&cc.egui_ctx, prefer_dark_mode);

        let mut session = EmulatorSession::new(assets)?;
        let mut status_line = "No ROM loaded".to_owned();
        if let Some(path) = rom_path {
            session.load_rom(path.clone())?;
            status_line = format!("Loaded {}", path.display());
        }

        Ok(Self {
            session,
            framebuffer_texture: None,
            prefer_dark_mode,
            status_line,
            held_input: ControllerState::default(),
        })
    }

    fn run_frame(&mut self, ctx: &egui::Context) {
        self.session.hold_controller1(self.held_input);
        match self.session.run_frame() {
            Ok(()) => {
                self.refresh_framebuffer(ctx);
                self.status_line = format!(
                    "Frame {} | APU steps {} | Audio samples {}",
                    self.session.emulator().timing().frame,
                    self.session.emulator().apu_status().spc700_steps,
                    self.session.emulator().audio_samples().samples.len()
                );
            }
            Err(error) => {
                warn!("{error}");
                self.status_line = error.to_string();
            }
        }
    }

    fn refresh_framebuffer(&mut self, ctx: &egui::Context) {
        let framebuffer = self.session.emulator().framebuffer();
        let image = ColorImage::from_rgba_unmultiplied(
            [framebuffer.width(), framebuffer.height()],
            framebuffer.pixels(),
        );

        if let Some(texture) = &mut self.framebuffer_texture {
            texture.set(image, TextureOptions::NEAREST);
        } else {
            self.framebuffer_texture = Some(ctx.load_texture(
                "starbyte-framebuffer",
                image,
                TextureOptions::NEAREST,
            ));
        }
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Session");
        if let Some(path) = self.session.rom_path() {
            ui.label(path.display().to_string());
        } else {
            ui.label("No ROM selected");
        }
        ui.label(self.status_line.as_str());
        ui.separator();

        if ui.button("Run Frame").clicked() {
            self.run_frame(ctx);
        }
        if ui.button("Run 60 Frames").clicked() {
            for _ in 0..60 {
                self.run_frame(ctx);
            }
        }

        if ui
            .checkbox(&mut self.prefer_dark_mode, "Night Mode")
            .changed()
        {
            apply_theme(ctx, self.prefer_dark_mode);
        }

        ui.separator();
        ui.heading("Controller 1");
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
    }

    fn draw_frame(&mut self, ui: &mut egui::Ui) {
        ui.heading("Display");
        ui.label(RichText::new("bsnes-inspired information density, but still intentionally modular").small());
        ui.separator();

        if let Some(texture) = &self.framebuffer_texture {
            let available = ui.available_size();
            let size = fit_size(texture.size_vec2(), available);
            ui.add(egui::Image::new((texture.id(), size)));
        } else {
            ui.label("Run at least one frame to populate the software framebuffer.");
        }
    }
}

impl eframe::App for StarbyteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Starbyte");
                ui.label("Correctness-first frontend bootstrap");
            });
        });

        egui::SidePanel::left("controls")
            .resizable(true)
            .min_width(220.0)
            .show(ctx, |ui| self.draw_controls(ui, ctx));

        egui::CentralPanel::default().show(ctx, |ui| self.draw_frame(ui));
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
    let scale = (available.x / source.x).min(available.y / source.y).max(1.0);
    Vec2::new(source.x * scale, source.y * scale)
}

fn input_checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) {
    ui.checkbox(value, label);
}
