//! Renderer-side consumer of the independent emulation worker.
//!
//! Does not own FrontendSession and never controls when SNES frames execute.

use std::time::Duration;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use starbyte_frontend::SessionSnapshot;

use crate::{
    scheduler::EmulatorScheduler,
    session_engine::{OwnedSession, PublishedGameFrame},
};

#[derive(Default)]
pub struct WorkerDisplay {
    texture: Option<TextureHandle>,
    status: Option<SessionSnapshot>,
    emulation_cpu: Duration,
    stereo_pairs: usize,
    nonzero_pairs: usize,
}

impl WorkerDisplay {
    /// Import only the most recent available worker frame. When a compositor
    /// suppresses egui callbacks, emulation still proceeds; obsolete video
    /// frames are replaced in the producer mailbox.
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        worker: &EmulatorScheduler<OwnedSession, PublishedGameFrame>,
    ) -> bool {
        let Some(frame) = worker.take_latest_frame() else {
            return false;
        };
        self.import_frame(ctx, frame);
        true
    }

    /// Import a fresh frame received after a paused save-state restore,
    /// or the newest coalesced frame from the continuous worker.
    pub fn import_frame(&mut self, ctx: &egui::Context, frame: PublishedGameFrame) {
        if let Some(rgba) = frame.rgba {
            let width = frame.status.framebuffer_width as usize;
            let height = frame.status.framebuffer_height as usize;
            let expected = width.saturating_mul(height).saturating_mul(4);
            if width > 0 && height > 0 && rgba.len() == expected {
                let image = ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                if let Some(texture) = &mut self.texture {
                    texture.set(image, TextureOptions::NEAREST);
                } else {
                    self.texture = Some(ctx.load_texture(
                        "starbyte-worker-framebuffer",
                        image,
                        TextureOptions::NEAREST,
                    ));
                }
            }
        }
        self.emulation_cpu = frame.last_frame_cpu;
        self.stereo_pairs = frame.stereo_pairs;
        self.nonzero_pairs = frame.nonzero_pairs;
        self.status = Some(frame.status);
    }

    pub fn status(&self) -> Option<&SessionSnapshot> {
        self.status.as_ref()
    }

    pub fn texture(&self) -> Option<&TextureHandle> {
        self.texture.as_ref()
    }

    pub fn last_cpu_duration(&self) -> Duration {
        self.emulation_cpu
    }

    pub fn last_audio_pairs(&self) -> (usize, usize) {
        (self.stereo_pairs, self.nonzero_pairs)
    }

    /// Presentation is pure UI work: it never blocks the emulator or asks
    /// the worker to advance a frame.
    pub fn draw(&self, ui: &mut egui::Ui, available: Vec2, integer_scale: bool) {
        let Some(texture) = &self.texture else {
            ui.label("Waiting for first emulated frame...");
            return;
        };
        let source = texture.size_vec2();
        if source.x <= 0.0 || source.y <= 0.0 {
            return;
        }
        let fit = (available.x / source.x).min(available.y / source.y);
        let scale = if integer_scale && fit >= 1.0 {
            fit.floor()
        } else {
            fit
        };
        let size = source * scale.max(0.01);
        ui.centered_and_justified(|ui| {
            ui.add(egui::Image::new((texture.id(), size)));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_can_update_without_copying_hidden_video() {
        let mut display = WorkerDisplay::default();
        let ctx = egui::Context::default();
        display.import_frame(
            &ctx,
            PublishedGameFrame {
                status: SessionSnapshot {
                    rom_path: None,
                    has_rom: true,
                    frame: 42,
                    apu_steps: 120,
                    audio_sample_count: 1064,
                    framebuffer_width: 256,
                    framebuffer_height: 224,
                },
                rgba: None,
                last_frame_cpu: Duration::from_millis(3),
                stereo_pairs: 532,
                nonzero_pairs: 200,
            },
        );
        assert_eq!(display.status().unwrap().frame, 42);
        assert!(display.texture().is_none());
        assert_eq!(display.last_audio_pairs(), (532, 200));
    }
}
