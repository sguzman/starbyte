//! Frontend-agnostic session orchestration shared by native shells.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use starbyte_core::{
    Emulator, EmulatorBuilder, cartridge::Cartridge, input::ControllerState, manifest::AssetConfig,
};

/// Read-only session status exported to frontend shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Loaded ROM path if any.
    pub rom_path: Option<PathBuf>,
    /// Whether a ROM is currently installed.
    pub has_rom: bool,
    /// Current video frame index.
    pub frame: u64,
    /// Current SPC700 step count.
    pub apu_steps: u64,
    /// Current buffered audio sample count.
    pub audio_sample_count: usize,
    /// Current framebuffer width in pixels.
    pub framebuffer_width: u32,
    /// Current framebuffer height in pixels.
    pub framebuffer_height: u32,
}

impl SessionSnapshot {
    /// Render a compact host-facing status string.
    #[must_use]
    pub fn status_line(&self) -> String {
        if !self.has_rom {
            return "No ROM loaded".to_owned();
        }

        format!(
            "Frame {} | APU steps {} | Audio samples {}",
            self.frame, self.apu_steps, self.audio_sample_count
        )
    }
}

/// Frontend-agnostic emulator session used by UI shells.
#[derive(Debug)]
pub struct FrontendSession {
    emulator: Emulator,
    rom_path: Option<PathBuf>,
}

impl FrontendSession {
    /// Build a frontend session with the provided asset configuration.
    pub fn new(assets: AssetConfig) -> Result<Self> {
        let mut emulator = EmulatorBuilder::new().assets(assets).build();
        let _ = emulator.load_apu_ipl_rom();
        Ok(Self {
            emulator,
            rom_path: None,
        })
    }

    /// Load a ROM from disk and reset the emulator around it.
    pub fn load_rom<P>(&mut self, rom_path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let path = rom_path.as_ref().to_path_buf();
        let cartridge = Cartridge::load(&path)
            .with_context(|| format!("failed to load ROM at {}", path.display()))?;
        self.emulator.load_rom(cartridge);
        self.rom_path = Some(path);
        Ok(())
    }

    /// Advance the emulator by one frame.
    pub fn run_frame(&mut self) -> Result<()> {
        self.emulator
            .run_until_frame()
            .context("failed to run frame")
    }

    /// Advance the emulator by a fixed number of frames.
    pub fn run_frames(&mut self, frame_count: usize) -> Result<()> {
        for _ in 0..frame_count {
            self.run_frame()?;
        }
        Ok(())
    }

    /// Update controller-1 state.
    pub fn set_controller1(&mut self, state: ControllerState) {
        self.emulator.set_controller1(state);
    }

    /// Snapshot current session status for a host shell.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        let framebuffer = self.emulator.framebuffer();
        SessionSnapshot {
            rom_path: self.rom_path.clone(),
            has_rom: self.emulator.cartridge().is_some(),
            frame: self.emulator.timing().frame,
            apu_steps: self.emulator.apu_status().spc700_steps,
            audio_sample_count: self.emulator.audio_samples().samples.len(),
            framebuffer_width: framebuffer.width() as u32,
            framebuffer_height: framebuffer.height() as u32,
        }
    }

    /// Borrow framebuffer pixels as RGBA8.
    #[must_use]
    pub fn framebuffer_rgba(&self) -> &[u8] {
        self.emulator.framebuffer().pixels()
    }

    /// Borrow the underlying emulator for specialized host work.
    #[must_use]
    pub const fn emulator(&self) -> &Emulator {
        &self.emulator
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use starbyte_core::input::ControllerState;
    use tempfile::tempdir;

    use super::FrontendSession;

    fn synthetic_rom_bytes() -> Vec<u8> {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 20].copy_from_slice(b"STARBYTE FRONTEND   ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0xFF;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0x00;
        rom[base + 0x1F] = 0x00;
        rom
    }

    #[test]
    fn snapshot_reports_empty_session() {
        let session = FrontendSession::new(Default::default()).unwrap();
        let snapshot = session.snapshot();
        assert!(!snapshot.has_rom);
        assert_eq!(snapshot.status_line(), "No ROM loaded");
    }

    #[test]
    fn session_runs_frames_without_frontend_specific_state() {
        let temp_dir = tempdir().unwrap();
        let rom_path = temp_dir.path().join("frontend-test.sfc");
        fs::write(&rom_path, synthetic_rom_bytes()).unwrap();

        let mut session = FrontendSession::new(Default::default()).unwrap();
        session.load_rom(&rom_path).unwrap();
        session.set_controller1(ControllerState {
            start: true,
            ..ControllerState::default()
        });

        session.run_frames(2).unwrap();

        let snapshot = session.snapshot();
        assert!(snapshot.has_rom);
        assert!(snapshot.frame >= 2);
        assert!(snapshot.audio_sample_count > 0);
        assert_eq!(snapshot.rom_path, Some(PathBuf::from(&rom_path)));
        assert!(snapshot.status_line().contains("Audio samples"));
        assert_eq!(
            session.framebuffer_rgba().len(),
            (snapshot.framebuffer_width * snapshot.framebuffer_height * 4) as usize
        );
    }
}
