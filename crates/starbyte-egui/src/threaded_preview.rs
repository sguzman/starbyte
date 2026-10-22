//! Opt-in native acceptance shell for the independent emulation engine.
//!
//! Deliberately separate from the user's established library UI until all
//! existing game controls, cheats, saves and layout panels are migrated.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use eframe::egui::{self, Color32};
use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, AudioPitchMode, RuntimeConfig},
};
use starbyte_frontend::SoundSessionMode;

use crate::{
    scheduler::EmulatorScheduler,
    scheduler_video::WorkerDisplay,
    session_engine::{InputPort, OwnedSession, PublishedGameFrame, RenderPort},
};

fn pitch_for(mode: AudioPitchMode, custom_percent: u16, speed: u16) -> f32 {
    match mode {
        AudioPitchMode::Preserve => 1.0,
        AudioPitchMode::FollowSpeed => f32::from(speed) / 100.0,
        AudioPitchMode::Custom => f32::from(custom_percent.clamp(50, 200)) / 100.0,
    }
}

pub struct ThreadedPreviewApp {
    engine: EmulatorScheduler<OwnedSession, PublishedGameFrame>,
    input: InputPort,
    render: RenderPort,
    display: WorkerDisplay,
    focused: bool,
    playing: bool,
    pause_when_unfocused: bool,
    integer_scale: bool,
    speed_percent: u16,
    pitch_mode: AudioPitchMode,
    pitch_percent: u16,
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
        let (engine, input, render) = session.spawn_with_ports()?;
        engine.set_speed_percent(speed_percent);
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
            speed_percent,
            pitch_mode: config.audio.pitch_mode,
            pitch_percent: config.audio.pitch_percent,
            exit_after_frames,
            last_error: None,
        })
    }

    fn keyboard(&self, ctx: &egui::Context) {
        let (held, pressed) = ctx.input(|input| {
            let state = |pressed: bool| {
                let key = |button| {
                    if pressed {
                        input.key_pressed(button)
                    } else {
                        input.key_down(button)
                    }
                };
                ControllerState {
                    up: key(egui::Key::ArrowUp),
                    down: key(egui::Key::ArrowDown),
                    left: key(egui::Key::ArrowLeft),
                    right: key(egui::Key::ArrowRight),
                    a: key(egui::Key::Z),
                    b: key(egui::Key::X),
                    start: key(egui::Key::Enter),
                    select: key(egui::Key::Backspace),
                    ..ControllerState::default()
                }
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
        // Focus is not the same as visibility: a window can be on-screen
        // and unfocused while the game continues in the background. Keep
        // framebuffer publication enabled in this preview; the single-slot
        // mailbox safely discards frames when Wayland withholds redraws.
        self.render.set_visible(true);
        if focused {
            self.keyboard(ctx);
        } else {
            // Flush short press edges too; a previous key must not remain
            // latched after focus has moved to another application.
            self.input.clear();
        }

        if let Some(error) = self.engine.take_error() {
            self.last_error = Some(error);
            self.playing = false;
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
                        match self.engine.quick_load_with_frame() {
                            Ok(frame) => {
                                // Drop any still-queued picture from before
                                // the restored state to avoid visual rollback.
                                let _ = self.engine.take_latest_frame();
                                self.display.import_frame(ctx, frame);
                                self.last_error = None;
                            }
                            Err(error) => self.last_error = Some(error),
                        }
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut self.pause_when_unfocused, "Pause in background")
                        .changed()
                    {
                        self.engine.set_pause_when_unfocused(self.pause_when_unfocused);
                    }
                    ui.checkbox(&mut self.integer_scale, "Integer scale");
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
                            let pitch = pitch_for(
                                self.pitch_mode,
                                self.pitch_percent,
                                speed,
                            );
                            if let Err(error) =
                                self.engine.configure_playback_speed(speed, pitch)
                            {
                                self.last_error = Some(error);
                            }
                            ui.close();
                        }
                    }
                });
                ui.label("Threaded preview (not the normal Starbyte library)");
            });
        });
        if focused && ctx.input(|input| input.key_pressed(egui::Key::F5)) {
            self.last_error = self.engine.quick_save_game().err();
        }
        if focused && ctx.input(|input| input.key_pressed(egui::Key::F8)) {
            match self.engine.quick_load_with_frame() {
                Ok(frame) => {
                    let _ = self.engine.take_latest_frame();
                    self.display.import_frame(ctx, frame);
                    self.last_error = None;
                }
                Err(error) => self.last_error = Some(error),
            }
        }

        self.display.poll(ctx, &self.engine);
        // Acceptance requires both worker progress and a real texture upload
        // through the window renderer, not just counting hidden ticks.
        if let Some(limit) = self.exit_after_frames {
            let rendered = self.display.status().map_or(0, |status| status.frame);
            if rendered >= limit && self.display.texture().is_some() {
                println!(
                    "STARBYTE_THREADED_PREVIEW_SMOKE_OK worker_frames={} rendered_frame={rendered}",
                    self.engine.completed_frames()
                );
                self.exit_after_frames = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let bounds = ui.max_rect();
            ui.painter().rect_filled(bounds, 0.0, Color32::BLACK);
            if let Some(error) = &self.last_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            let available = ui.available_size();
            self.display.draw(ui, available, self.integer_scale);
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
                ui.label(format!(
                    "CPU {:.2} ms",
                    self.display.last_cpu_duration().as_secs_f32() * 1_000.0
                ));
                ui.separator();
                let rate = self.engine.frame_stats();
                if let Some(fps) = rate.fps {
                    ui.label(format!(
                        "Worker FPS {fps:.1} / {:.1}",
                        60.0985 * f32::from(self.speed_percent) / 100.0
                    ));
                } else {
                    ui.label("Worker FPS: measuring");
                }
                ui.separator();
                ui.label(format!(
                    "Frame gap {:.1} ms | 40ms+ hitches {}",
                    rate.last_gap_ms, rate.hitches_40ms
                ));
                ui.separator();
                ui.label(format!(
                    "Video frames superseded {}",
                    self.engine.skipped_video_frames()
                ));
            });
        });
        // UI painting is still paced by egui, but the worker is NOT.
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
