//! Bridge from the window-independent clock to a single-owned SNES session.
//!
//! The GUI must command this worker rather than borrowing FrontendSession
//! directly. No egui or Wayland surface is required to execute a frame.

use std::{
    io,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use starbyte_core::{input::ControllerState, manifest::AssetConfig};
use starbyte_frontend::{
    CheatEntry, FrontendSession, SessionSnapshot, SoundSessionMode,
};

use crate::{
    audio::AudioOutput,
    scheduler::{EmulatorScheduler, NTSC_FRAME_INTERVAL},
};

const FRAME_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Controller bridge updated directly by the GUI. Held state replaces its
/// predecessor; short press edges are OR-latched until consumed by one SNES
/// frame. No command queue can accumulate stale high-frequency input.
#[derive(Clone, Default)]
pub struct InputPort {
    held_bits: Arc<AtomicU16>,
    pending_press_bits: Arc<AtomicU16>,
}

impl InputPort {
    /// Submit the current controller state and *new* press edges separately.
    pub fn submit(&self, held: ControllerState, just_pressed: ControllerState) {
        self.held_bits.store(held.to_bits(), Ordering::Release);
        self.pending_press_bits
            .fetch_or(just_pressed.to_bits(), Ordering::AcqRel);
    }

    /// Called once by the worker immediately before emulating a frame.
    fn take_frame_state(&self) -> ControllerState {
        let held = self.held_bits.load(Ordering::Acquire);
        let edges = self.pending_press_bits.swap(0, Ordering::AcqRel);
        controller_from_bits(held | edges)
    }

    fn reset(&self) {
        self.held_bits.store(0, Ordering::Release);
        self.pending_press_bits.store(0, Ordering::Release);
    }
}

fn controller_from_bits(bits: u16) -> ControllerState {
    ControllerState {
        b: bits & 0x8000 != 0,
        y: bits & 0x4000 != 0,
        select: bits & 0x2000 != 0,
        start: bits & 0x1000 != 0,
        up: bits & 0x0800 != 0,
        down: bits & 0x0400 != 0,
        left: bits & 0x0200 != 0,
        right: bits & 0x0100 != 0,
        a: bits & 0x0080 != 0,
        x: bits & 0x0040 != 0,
        l: bits & 0x0020 != 0,
        r: bits & 0x0010 != 0,
    }
}

/// A snapshot for the renderer; video frames coalesce when the UI is hidden.
pub struct PublishedGameFrame {
    pub status: SessionSnapshot,
    pub rgba: Vec<u8>,
    pub last_frame_cpu: Duration,
    pub stereo_pairs: usize,
    pub nonzero_pairs: usize,
}

/// Exclusive, thread-owned game state. All save mutations stay serialized
/// with emulated frames, including when a compositor hides the entire window.
pub struct OwnedSession {
    session: FrontendSession,
    audio: Option<AudioOutput>,
    sound_enabled: bool,
    muted: bool,
    volume: f32,
    speed_percent: u16,
    pitch_factor: f32,
    input: InputPort,
}

impl OwnedSession {
    pub fn create(assets: AssetConfig) -> anyhow::Result<Self> {
        Ok(Self::from_session(FrontendSession::new(assets)?))
    }

    pub fn from_session(session: FrontendSession) -> Self {
        Self {
            session,
            audio: None,
            sound_enabled: true,
            muted: false,
            volume: 0.15,
            speed_percent: 100,
            pitch_factor: 1.0,
            input: InputPort::default(),
        }
    }

    pub fn spawn(self) -> io::Result<EmulatorScheduler<Self, PublishedGameFrame>> {
        EmulatorScheduler::spawn(self, NTSC_FRAME_INTERVAL, Self::advance)
    }

    /// Transfer engine ownership while retaining the lock-free input port.
    pub fn spawn_with_input(
        self,
    ) -> io::Result<(EmulatorScheduler<Self, PublishedGameFrame>, InputPort)> {
        let input = self.input.clone();
        Ok((self.spawn()?, input))
    }

    pub fn set_input(&mut self, input: ControllerState) {
        self.input.submit(input, input);
    }

    pub fn set_audio(&mut self, enabled: bool, muted: bool, volume: f32) {
        self.sound_enabled = enabled;
        self.muted = muted;
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(output) = &self.audio {
            output.set_volume(if enabled && !muted { self.volume } else { 0.0 });
            if !enabled || muted {
                output.clear();
            }
        }
    }

    pub fn set_speed(&mut self, speed_percent: u16, pitch_factor: f32) {
        self.speed_percent = speed_percent.clamp(1, 1_600);
        self.pitch_factor = pitch_factor.clamp(0.25, 4.0);
        if let Some(output) = &self.audio {
            output.clear();
        }
    }

    pub fn load_rom(&mut self, path: &std::path::Path, mode: SoundSessionMode) -> anyhow::Result<()> {
        if let Some(output) = &self.audio {
            output.clear();
        }
        self.input.reset();
        match mode {
            SoundSessionMode::Compatibility => self.session.load_rom(path),
            SoundSessionMode::Isolated => self.session.load_rom_with_isolated_audio_probe(path),
            SoundSessionMode::Persistent => self.session.load_rom_with_persistent_audio(path),
        }
    }

    pub fn set_cheats(&mut self, cheats: &[CheatEntry]) -> usize {
        self.session.set_active_cheats(cheats)
    }

    pub fn quick_save(&mut self) -> anyhow::Result<()> {
        self.session.quick_save()
    }

    pub fn quick_load(&mut self) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.session.quick_load()
    }

    pub fn save_slot(&mut self, slot: u8) -> anyhow::Result<std::path::PathBuf> {
        self.session.save_state_slot(slot)
    }

    pub fn load_slot(&mut self, slot: u8) -> anyhow::Result<()> {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.session.load_state_slot(slot)
    }

    pub fn flush_sram(&mut self) -> anyhow::Result<()> {
        self.session.flush_save_ram()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        self.session.snapshot()
    }

    /// Produce one game frame entirely on the scheduler thread. The Rodio
    /// queue is already bounded and independent of egui. Never enqueue PCM
    /// for the renderer to replay later after a hidden-window interval.
    pub fn advance(&mut self) -> Result<PublishedGameFrame, String> {
        let started = Instant::now();
        self.session.set_controller1(self.input.take_frame_state());
        self.session
            .run_frame_with_timeout(FRAME_TIMEOUT)
            .map_err(|error| error.to_string())?;

        let status = self.session.snapshot();
        let samples = &self.session.emulator().audio_samples().samples;
        let stereo_pairs = samples.len() / 2;
        let nonzero_pairs = samples.chunks_exact(2)
            .filter(|pair| pair[0] != 0 || pair[1] != 0)
            .count();

        if self.sound_enabled && !self.muted && self.volume > 0.0 {
            if self.audio.is_none() && nonzero_pairs > 0 {
                match AudioOutput::start() {
                    Ok(output) => self.audio = Some(output),
                    Err(error) => tracing::warn!("emulation worker could not start audio: {error}"),
                }
            }
            if let Some(audio) = &self.audio {
                audio.set_volume(self.volume);
                audio.enqueue(
                    samples,
                    f32::from(self.speed_percent) / 100.0,
                    self.pitch_factor,
                );
            }
        }

        Ok(PublishedGameFrame {
            status,
            rgba: self.session.framebuffer_rgba().to_vec(),
            last_frame_cpu: started.elapsed(),
            stereo_pairs,
            nonzero_pairs,
        })
    }
}

impl Drop for OwnedSession {
    fn drop(&mut self) {
        if let Err(error) = self.flush_sram() {
            tracing::warn!("could not flush worker-owned SRAM on shutdown: {error}");
        }
        if let Some(audio) = &self.audio {
            audio.clear();
        }
    }
}

/// Typed API for the GUI handoff. Instead of locking the live emulator,
/// the GUI sends gamepad/audio state and explicit save commands to the owner.
impl EmulatorScheduler<OwnedSession, PublishedGameFrame> {
    pub fn set_gamepad_state(&self, state: ControllerState) -> Result<(), String> {
        self.post(move |engine| engine.set_input(state))
    }

    pub fn configure_sound(
        &self,
        enabled: bool,
        muted: bool,
        volume: f32,
    ) -> Result<(), String> {
        self.post(move |engine| engine.set_audio(enabled, muted, volume))
    }

    pub fn configure_playback_speed(
        &self,
        speed_percent: u16,
        pitch_factor: f32,
    ) -> Result<(), String> {
        self.set_speed_percent(speed_percent);
        self.post(move |engine| engine.set_speed(speed_percent, pitch_factor))
    }

    pub fn load_game(
        &self,
        path: std::path::PathBuf,
        mode: SoundSessionMode,
    ) -> Result<(), String> {
        self.transact(move |engine| engine.load_rom(&path, mode).map_err(|e| e.to_string()))
    }

    pub fn update_cheats(&self, cheats: Vec<CheatEntry>) -> Result<usize, String> {
        self.transact(move |engine| Ok(engine.set_cheats(&cheats)))
    }

    pub fn quick_save_game(&self) -> Result<(), String> {
        self.transact(|engine| engine.quick_save().map_err(|e| e.to_string()))
    }

    pub fn quick_load_game(&self) -> Result<(), String> {
        self.transact(|engine| engine.quick_load().map_err(|e| e.to_string()))
    }

    pub fn save_game_slot(&self, slot: u8) -> Result<std::path::PathBuf, String> {
        self.transact(move |engine| engine.save_slot(slot).map_err(|e| e.to_string()))
    }

    pub fn load_game_slot(&self, slot: u8) -> Result<(), String> {
        self.transact(move |engine| engine.load_slot(slot).map_err(|e| e.to_string()))
    }

    pub fn persist_sram(&self) -> Result<(), String> {
        self.transact(|engine| engine.flush_sram().map_err(|e| e.to_string()))
    }

    pub fn read_status(&self) -> Result<SessionSnapshot, String> {
        self.transact(|engine| Ok(engine.snapshot()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_button_edges_survive_a_quick_release() {
        let port = InputPort::default();
        let pressed = ControllerState {
            start: true,
            ..ControllerState::default()
        };
        port.submit(pressed, pressed);
        port.submit(ControllerState::default(), ControllerState::default());
        assert!(port.take_frame_state().start);
        assert!(!port.take_frame_state().start);
    }

    #[test]
    fn controller_bits_round_trip() {
        let all = ControllerState {
            b: true, y: true, select: true, start: true,
            up: true, down: true, left: true, right: true,
            a: true, x: true, l: true, r: true,
        };
        assert_eq!(controller_from_bits(all.to_bits()), all);
    }

    #[test]
    fn detached_session_has_no_game_until_loaded() {
        let engine = OwnedSession::create(AssetConfig::default()).unwrap();
        assert!(!engine.snapshot().has_rom);
    }

    #[test]
    fn sound_and_speed_preferences_are_clamped() {
        let mut engine = OwnedSession::create(AssetConfig::default()).unwrap();
        engine.set_audio(true, false, 5.0);
        engine.set_speed(10_000, 40.0);
        assert_eq!(engine.volume, 1.0);
        assert_eq!(engine.speed_percent, 1_600);
        assert_eq!(engine.pitch_factor, 4.0);
    }
}
