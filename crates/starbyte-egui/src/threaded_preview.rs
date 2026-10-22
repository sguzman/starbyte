//! Opt-in native acceptance shell for the independent emulation engine.
//!
//! Deliberately separate from the user's established library UI until all
//! existing game controls, cheats, saves and layout panels are migrated.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use eframe::egui::{self, Color32};
use starbyte_core::{input::ControllerState, manifest::{AssetConfig, RuntimeConfig}};
use starbyte_frontend::SoundSessionMode;

use crate::{
    scheduler::EmulatorScheduler,
    scheduler_video::WorkerDisplay,
    session_engine::{InputPort, OwnedSession, PublishedGameFrame, RenderPort},
};

pub struct ThreadedPreviewApp {
    engine: EmulatorScheduler<OwnedSession, PublishedGameFrame>,
    input: InputPort,
    render: RenderPort,
    display: WorkerDisplay,
    focused: bool,
    playing: bool,
    pause_when_unfocused: bool,
    integer_scale: bool,
    exit_after_frames: Option<u64>,
    last_error: Option<String>,
}

impl ThreadedPreviewApp {
    pub fn new(
        assets: AssetConfig,
        config: &RuntimeConfig,
        rom_path: PathBuf,
        exit_after_frames: Option<u64>,
    ) -> Result<Self> {
        let mut session = OwnedSession::create(assets)?;
        session.load_rom(&rom_path, SoundSessionMode::Persistent)?;
        session.set_audio(
            config.audio.enabled,
            config.audio.mute_on_startup,
            config.audio.volume,
        );
        let (engine, input, render) = session.spawn_with_ports()?;
        render.set_visible(true);
        engine.set_pause_when_unfocused(config.ui.pause_when_unfocused);
        engine.set_focused(true);
        engine.set_playing(true);
        Ok(Self {
            engine,
            input,
            render,
            display: WorkerDisplay::default(),
            focused: true,
            playing: true,
            pause_when_unfocused: config.ui.pause_when_unfocused,
            integer_scale: config.video.integer_scale,
            exit_after_frames,
            last_error: None,
        })
    }

    fn keyboard(&self, ctx: &egui::Context) {
        let (held, pressed) = ctx.input(|input| {
            let state = |pressed: bool| ControllerState {
                up: if pressed { input.key_pressed(egui::Key::ArrowUp) } else { input.key_down(egui::Key::ArrowUp) },
                down: if pressed { input.key_pressed(egui::Key::ArrowDown) } else { input.key_down(egui::Key::ArrowDown) },
                left: if pressed { input.key_pressed(egui::Key::ArrowLeft) } else { input.key_down(egui::Key::ArrowLeft) },
                right: if pressed { input.key_pressed(egui::Key::ArrowRight) } else { input.key_down(egui::Key::ArrowRight) },
                a: if pressed { input.key_pressed(egui::Key::Z) } else { input.key_down(egui::Key::Z) },
                b: if pressed { input.key_pressed(egui::Key::X) } else { input.key_down(egui::Key::X) },
                start: if pressed { input.key_pressed(egui::Key::Enter) } else { input.key_down(egui::Key::Enter) },
                select: if pressed { input.key_pressed(egui::Key::Backspace) } else { input.key_down(egui::Key::Backspace) },
                ..ControllerState::default()
            };
            (state(false), state(true))
        });
        self.input.submit(held, pressed);
    }
}

impl eframe::App for ThreadedPreviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|input| input.focused);
        if focused != self.focused {
            self.focused = focused;
            self.engine.set_focused(focused);
        }
        // A hidden/occluded window need not receive copied RGBA bytes.
        // Emulation and audio continue on their independent worker clock.
        self.render.set_visible(focused);
        if focused {
            self.keyboard(ctx);
        } else {
            self.input.submit(ControllerState::default(), ControllerState::default());
        }

        if let Some(error) = self.engine.take_error() {
            self.last_error = Some(error);
            self.playing = false;
        }
        if let Some(limit) = self.exit_after_frames {
            if self.engine.completed_frames() >= limit {
                println!(
                    "STARBYTE_THREADED_PREVIEW_SMOKE_OK frames={}",
                    self.engine.completed_frames()
                );
                self.exit_after_frames = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        egui::TopBottomPanel::top("worker_preview_menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Game", |ui| {
                    if ui.button(if self.playing { "Pause" } else { "Resume" }).clicked() {
                        self.playing = !self.playing;
                        self.engine.set_playing(self.playing);
                        ui.close();
                    }
                    if ui.button("Quick Save (F5)").clicked() {
                        self.last_error = self.engine.quick_save_game().err();
                        ui.close();
                    }
                    if ui.button("Quick Load (F8)").clicked() {
                        self.last_error = self.engine.quick_load_game().err();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.checkbox(&mut self.pause_when_unfocused, "Pause in background").changed() {
                        self.engine.set_pause_when_unfocused(self.pause_when_unfocused);
                    }
                    ui.checkbox(&mut self.integer_scale, "Integer scale");
                });
                ui.label("Threaded preview (not the normal Starbyte library)");
            });
        });
        if focused && ctx.input(|input| input.key_pressed(egui::Key::F5)) {
            self.last_error = self.engine.quick_save_game().err();
        }
        if focused && ctx.input(|input| input.key_pressed(egui::Key::F8)) {
            self.last_error = self.engine.quick_load_game().err();
        }

        self.display.poll(ctx, &self.engine);
        egui::CentralPanel::default().show(ctx, |ui| {
            let bounds = ui.max_rect();
            ui.painter().rect_filled(bounds, 0.0, Color32::BLACK);
            if let Some(error) = &self.last_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            self.display.draw(ui, ui.available_size(), self.integer_scale);
        });
        egui::TopBottomPanel::bottom("worker_preview_stats").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(status) = self.display.status() {
                    ui.label(format!(
                        "Worker frame {} | APU steps {} | Samples {}",
                        status.frame, status.apu_steps, status.audio_sample_count
                    ));
                }
                ui.separator();
                ui.label(format!("CPU {:.2} ms", self.display.last_cpu_duration().as_secs_f32() * 1_000.0));
                ui.separator();
                ui.label(format!("Video frames superseded {}", self.engine.skipped_video_frames()));
            });
        });
        // UI painting is still paced by egui, but the worker is NOT.
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
