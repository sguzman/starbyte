//! Opt-in native acceptance shell for the independent emulation engine.
//!
//! Deliberately separate from the user's established library UI until all
//! existing game controls, cheats, saves and layout panels are migrated.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use eframe::egui::{self, Color32};
use starbyte_core::{
    input::ControllerState,
    manifest::{AssetConfig, AudioPitchMode, InputDeviceMode, RuntimeConfig},
};
use starbyte_frontend::SoundSessionMode;

use crate::{
    app::{input_actions, parse_egui_key, set_controller_flag, write_png_screenshot},
    preview_gamepads::{merge_sources, PreviewGamepads},
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

enum PendingOperation {
    Save(Receiver<Result<(), String>>),
    Restore(Receiver<Result<PublishedGameFrame, String>>),
    Screenshot(Receiver<Result<PublishedGameFrame, String>>),
    PngWrite(Receiver<Result<PathBuf, String>>),
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
    keyboard_bindings: BTreeMap<String, String>,
    gamepads: PreviewGamepads,
    input_mode: InputDeviceMode,
    exit_after_frames: Option<u64>,
    last_error: Option<String>,
    screenshot_root: PathBuf,
    status_note: Option<String>,
    pending: Vec<PendingOperation>,
}

impl ThreadedPreviewApp {
    pub fn new(
        assets: AssetConfig,
        config: &RuntimeConfig,
        rom_path: PathBuf,
        exit_after_frames: Option<u64>,
        preview_background: bool,
    ) -> Result<Self> {
        let screenshot_root = assets.screenshot_root();
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
        // Native smoke may run under a compositor without keyboard focus.
        // This opt-in does not mutate RuntimeConfig or the default policy.
        let pause_when_unfocused = config.ui.pause_when_unfocused && !preview_background;
        engine.set_pause_when_unfocused(pause_when_unfocused);
        engine.set_focused(true);
        engine.set_playing(true);
        Ok(Self {
            engine,
            input,
            render,
            display: WorkerDisplay::default(),
            focused: true,
            playing: true,
            pause_when_unfocused,
            integer_scale: config.video.integer_scale,
            speed_percent,
            pitch_mode: config.audio.pitch_mode,
            pitch_percent: config.audio.pitch_percent,
            keyboard_bindings: config.input.keyboard_bindings.clone(),
            gamepads: PreviewGamepads::new(config.input.gamepad_bindings.clone()),
            input_mode: config.input.active_device,
            exit_after_frames,
            last_error: None,
            screenshot_root,
            status_note: None,
            pending: Vec::new(),
        })
    }

    fn queue_save(&mut self, result: Result<Receiver<Result<(), String>>, String>) {
        match result {
            Ok(reply) => self.pending.push(PendingOperation::Save(reply)),
            Err(error) => self.last_error = Some(error),
        }
    }

    fn queue_restore(
        &mut self,
        result: Result<Receiver<Result<PublishedGameFrame, String>>, String>,
    ) {
        match result {
            Ok(reply) => self.pending.push(PendingOperation::Restore(reply)),
            Err(error) => self.last_error = Some(error),
        }
    }

    fn queue_screenshot(&mut self) {
        match self.engine.queue_screenshot_frame() {
            Ok(reply) => self.pending.push(PendingOperation::Screenshot(reply)),
            Err(error) => self.last_error = Some(error),
        }
    }

    fn write_screenshot_async(
        &mut self,
        frame: PublishedGameFrame,
    ) -> Result<Receiver<Result<PathBuf, String>>, String> {
        let rgba = frame.rgba.ok_or_else(|| "No screenshot pixels available".to_owned())?;
        let width = frame.status.framebuffer_width;
        let height = frame.status.framebuffer_height;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self.screenshot_root.join(format!(
            "starbyte-{timestamp}-frame-{}.png",
            frame.status.frame
        ));
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
        for operation in std::mem::take(&mut self.pending) {
            match operation {
                PendingOperation::Save(reply) => match reply.try_recv() {
                    Ok(Ok(())) => self.last_error = None,
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::Save(reply));
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Save worker stopped unexpectedly".to_owned());
                    }
                },
                PendingOperation::Screenshot(reply) => match reply.try_recv() {
                    Ok(Ok(frame)) => match self.write_screenshot_async(frame) {
                        Ok(io_reply) => {
                            still_pending.push(PendingOperation::PngWrite(io_reply));
                        }
                        Err(error) => self.last_error = Some(error),
                    },
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::Screenshot(reply));
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Screenshot worker stopped".to_owned());
                    }
                },
                PendingOperation::PngWrite(reply) => match reply.try_recv() {
                    Ok(Ok(path)) => {
                        self.status_note = Some(format!(
                            "Screenshot saved to {}",
                            path.display()
                        ));
                        self.last_error = None;
                    }
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::PngWrite(reply));
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Screenshot writer stopped".to_owned());
                    }
                },
                PendingOperation::Restore(reply) => match reply.try_recv() {
                    Ok(Ok(frame)) => {
                        // Never paint stale video queued before the restored
                        // frame, even when loading during a manual pause.
                        let _ = self.engine.take_latest_frame();
                        self.display.import_frame(ctx, frame);
                        self.last_error = None;
                    }
                    Ok(Err(error)) => self.last_error = Some(error),
                    Err(TryRecvError::Empty) => {
                        still_pending.push(PendingOperation::Restore(reply));
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.last_error = Some("Load worker stopped unexpectedly".to_owned());
                    }
                },
            }
        }
        self.pending = still_pending;
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
                if let Some(binding) = self.keyboard_bindings.get(action)
                    && let Some(key) = parse_egui_key(binding)
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

impl eframe::App for ThreadedPreviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Deliberately independent of frame production: an absent heartbeat
        // tells the worker that Wayland stopped delivering redraws. With
        // background pause disabled, emulation continues regardless.
        self.engine.gui_heartbeat();
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
        // Poll device releases even when the window lacks focus, but only
        // transmit gameplay input while focused. This matches the existing
        // Keyboard / Gamepad / Auto input selection.
        let gamepad = self.gamepads.poll(
            focused && self.input_mode != InputDeviceMode::Keyboard,
        );
        if focused {
            let keyboard = if self.input_mode == InputDeviceMode::Gamepad {
                (ControllerState::default(), ControllerState::default())
            } else {
                self.keyboard(ctx)
            };
            let (held, pressed) = merge_sources(keyboard, gamepad);
            self.input.submit(held, pressed);
        } else {
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
                        let job = self.engine.queue_quick_save();
                        self.queue_save(job);
                        ui.close();
                    }
                    if ui.button("Quick Load (F8)").clicked() {
                        let job = self.engine.queue_quick_load_with_frame();
                        self.queue_restore(job);
                        ui.close();
                    }
                    if ui.button("Screenshot (F12)").clicked() {
                        self.queue_screenshot();
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Disk save slots", |ui| {
                        for slot in 1..=3_u8 {
                            if ui.button(format!("Save slot {slot}")).clicked() {
                                let job = self.engine.queue_save_slot(slot);
                                self.queue_save(job);
                                ui.close();
                            }
                            if ui.button(format!("Load slot {slot}")).clicked() {
                                let job = self.engine.queue_load_slot_with_frame(slot);
                                self.queue_restore(job);
                                ui.close();
                            }
                        }
                    });
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
            let job = self.engine.queue_quick_save();
            self.queue_save(job);
        }
        if focused && ctx.input(|input| input.key_pressed(egui::Key::F8)) {
            let job = self.engine.queue_quick_load_with_frame();
            self.queue_restore(job);
        }

        if focused && ctx.input(|input| input.key_pressed(egui::Key::F12)) {
            self.queue_screenshot();
        }
        self.display.poll(ctx, &self.engine);
        self.poll_operations(ctx);
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
            } else if let Some(message) = &self.status_note {
                ui.label(message);
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
