//! Frontend-agnostic session orchestration and library services shared by native shells.

mod library;

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};

use starbyte_core::{
    Emulator, EmulatorBuilder, cartridge::Cartridge, input::ControllerState, manifest::AssetConfig,
};

pub use self::library::{
    CheatEntry, CheatProvider, CoverAsset, CoverProvider, GameId, GameMetadata,
    GameMetadataProvider, InstalledStatus, LibraryEntry, LibraryFilter, LibraryService,
    LibrarySnapshot, LibraryTarget, LocalRomInfo, LocalRomSourceKind, RefreshSummary,
    RomDownloadProvider,
};
pub use starbyte_core::manifest::LibraryViewMode;

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
    cartridge_identity: Option<String>,
    active_cheat_patches: Vec<CheatPatch>,
    quick_state: Option<String>,
}

impl FrontendSession {
    /// Build a frontend session with the provided asset configuration.
    pub fn new(assets: AssetConfig) -> Result<Self> {
        let mut emulator = EmulatorBuilder::new().assets(assets).build();
        let _ = emulator.load_apu_ipl_rom();
        Ok(Self {
            emulator,
            rom_path: None,
            cartridge_identity: None,
            active_cheat_patches: Vec::new(),
            quick_state: None,
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
        // Hash once at load time, not every egui frame while a save-slot menu
        // is visible. Slot identity follows ROM contents, not its filename.
        let mut hasher = Sha1::new();
        hasher.update(cartridge.rom());
        let identity = format!("{:x}", hasher.finalize());
        self.emulator.load_rom(cartridge);
        self.cartridge_identity = Some(identity);
        self.quick_state = None;
        self.rom_path = Some(path);
        self.apply_active_cheats();
        Ok(())
    }

    /// Return the currently loaded ROM path, if any.
    #[must_use]
    pub fn loaded_rom_path(&self) -> Option<&Path> {
        self.rom_path.as_deref()
    }

    /// Advance the emulator by one frame.
    pub fn run_frame(&mut self) -> Result<()> {
        self.apply_active_cheats();
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

    /// Save a memory-only quick slot for the loaded ROM.
    ///
    /// The slot is discarded whenever another ROM is loaded or the app closes.
    pub fn quick_save(&mut self) -> Result<()> {
        anyhow::ensure!(self.rom_path.is_some(), "No ROM loaded for quick save");
        self.quick_state = Some(self.emulator.save_state().context("quick save failed")?);
        Ok(())
    }

    /// Whether a memory-only quick-save slot is available.
    #[must_use]
    pub fn has_quick_save(&self) -> bool {
        self.quick_state.is_some()
    }

    /// Load the quick slot and refresh the restored framebuffer.
    pub fn quick_load(&mut self) -> Result<()> {
        let state = self
            .quick_state
            .as_deref()
            .context("No quick save in this session")?;
        self.emulator
            .load_state(state)
            .context("quick load failed")?;
        self.emulator.refresh_framebuffer();
        self.apply_active_cheats();
        Ok(())
    }

    /// Deterministic, game-scoped disk slot. Slot names never contain untrusted
    /// paths or game titles; SHA-1 is for local naming, not authentication.
    pub fn state_slot_path(&self, slot: u8) -> Result<PathBuf> {
        anyhow::ensure!((1..=3).contains(&slot), "save slot must be 1, 2, or 3");
        let identity = self
            .cartridge_identity
            .as_deref()
            .context("No ROM loaded for save slots")?;
        Ok(self
            .emulator
            .assets()
            .state_root()
            .join(format!("{identity}.slot{slot}.state.json")))
    }

    /// Whether a game-scoped persistent slot currently exists.
    #[must_use]
    pub fn has_state_slot(&self, slot: u8) -> bool {
        self.state_slot_path(slot).is_ok_and(|path| path.is_file())
    }

    /// Atomically replace a durable slot in the user's XDG state directory.
    ///
    /// The temporary file is created in the destination directory to avoid
    /// partial/truncated states after a failed write or an interrupted save.
    pub fn save_state_slot(&self, slot: u8) -> Result<PathBuf> {
        let path = self.state_slot_path(slot)?;
        let parent = path.parent().context("save slot path has no parent")?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create state directory {}", parent.display()))?;
        let state = self
            .emulator
            .save_state()
            .context("could not serialize state")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .context("failed to create temporary save-state file")?;
        temp.write_all(state.as_bytes())
            .context("failed writing save state")?;
        temp.as_file()
            .sync_all()
            .context("failed syncing save state")?;
        temp.persist(&path)
            .with_context(|| format!("failed to persist save state to {}", path.display()))?;
        Ok(path)
    }

    /// Restore a durable slot for the currently loaded cartridge.
    ///
    /// The core checks full ROM bytes before any state mutation.
    pub fn load_state_slot(&mut self, slot: u8) -> Result<()> {
        let path = self.state_slot_path(slot)?;
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read save state {}", path.display()))?;
        self.emulator
            .load_state(&data)
            .context("failed to load saved state")?;
        self.emulator.refresh_framebuffer();
        self.apply_active_cheats();
        Ok(())
    }

    /// Update controller-1 state.
    pub fn set_controller1(&mut self, state: ControllerState) {
        self.emulator.set_controller1(state);
    }

    /// Replace the active cheat set with the enabled cheats for the current game.
    pub fn set_active_cheats(&mut self, cheats: &[CheatEntry]) -> usize {
        self.active_cheat_patches = cheats
            .iter()
            .filter(|cheat| cheat.enabled)
            .flat_map(|cheat| parse_cheat_patches(&cheat.code))
            .collect();
        self.apply_active_cheats();
        self.active_cheat_patches.len()
    }

    /// Disable all runtime-applied cheats for the current session.
    pub fn clear_active_cheats(&mut self) {
        self.active_cheat_patches.clear();
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

    /// Read one byte from the current emulator address space for host-side tooling.
    #[must_use]
    pub fn host_read_u8(&mut self, address: u32) -> u8 {
        self.emulator.host_read_u8(address)
    }

    fn apply_active_cheats(&mut self) {
        for patch in self.active_cheat_patches.iter().copied() {
            self.emulator.host_write_u8(patch.address, patch.value);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CheatPatch {
    address: u32,
    value: u8,
}

fn parse_cheat_patches(code: &str) -> Vec<CheatPatch> {
    code.split(['+', ',', ';', '|'])
        .filter_map(parse_cheat_patch)
        .collect()
}

fn parse_cheat_patch(segment: &str) -> Option<CheatPatch> {
    let segment = segment.trim();
    if segment.is_empty() || segment.contains('-') {
        return None;
    }

    if let Some((address, value)) = segment.split_once(':').or_else(|| segment.split_once('=')) {
        return Some(CheatPatch {
            address: parse_hex_u32(address)?,
            value: parse_hex_u8(value)?,
        });
    }

    let mut parts = segment.split_whitespace();
    if let (Some(address), Some(value), None) = (parts.next(), parts.next(), parts.next()) {
        return Some(CheatPatch {
            address: parse_hex_u32(address)?,
            value: parse_hex_u8(value)?,
        });
    }

    if segment.chars().all(|ch| ch.is_ascii_hexdigit()) && segment.len() == 8 {
        return Some(CheatPatch {
            address: u32::from_str_radix(&segment[..6], 16).ok()?,
            value: u8::from_str_radix(&segment[6..], 16).ok()?,
        });
    }

    None
}

fn parse_hex_u32(value: &str) -> Option<u32> {
    let value = value.trim().trim_start_matches('$');
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if value.len() != 6 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(value, 16).ok()
}

fn parse_hex_u8(value: &str) -> Option<u8> {
    let value = value.trim().trim_start_matches('$');
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if value.len() != 2 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u8::from_str_radix(value, 16).ok()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use starbyte_core::{input::ControllerState, manifest::AssetConfig};
    use tempfile::tempdir;

    use super::{CheatEntry, FrontendSession, parse_cheat_patches};

    fn synthetic_rom_bytes() -> Vec<u8> {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE FRONTEND    ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0xFF;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0x00;
        rom[base + 0x1F] = 0x00;
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
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

    #[test]
    fn quick_save_roundtrip_and_rom_reload_invalidate_slot() {
        let temp_dir = tempdir().unwrap();
        let rom_path = temp_dir.path().join("quick-save-test.sfc");
        fs::write(&rom_path, synthetic_rom_bytes()).unwrap();

        let mut session = FrontendSession::new(Default::default()).unwrap();
        assert!(session.quick_save().is_err());
        assert!(session.quick_load().is_err());
        session.load_rom(&rom_path).unwrap();
        session.run_frames(2).unwrap();
        let saved_frame = session.snapshot().frame;
        session.quick_save().unwrap();
        assert!(session.has_quick_save());

        session.run_frames(2).unwrap();
        assert!(session.snapshot().frame > saved_frame);
        session.quick_load().unwrap();
        assert_eq!(session.snapshot().frame, saved_frame);

        session.load_rom(&rom_path).unwrap();
        assert!(!session.has_quick_save());
        assert!(session.quick_load().is_err());
    }

    #[test]
    fn persistent_save_slots_survive_new_sessions_and_are_game_scoped() {
        let temp_dir = tempdir().unwrap();
        let rom_path = temp_dir.path().join("slot-test.sfc");
        let state_dir = temp_dir.path().join("states");
        let base_rom = synthetic_rom_bytes();
        fs::write(&rom_path, &base_rom).unwrap();
        let assets = AssetConfig {
            state_dir: Some(state_dir.clone()),
            ..AssetConfig::default()
        };
        let mut session = FrontendSession::new(assets.clone()).unwrap();
        assert!(!session.has_state_slot(1));
        assert!(session.save_state_slot(0).is_err());
        assert!(session.load_state_slot(1).is_err());
        session.load_rom(&rom_path).unwrap();
        session.run_frames(2).unwrap();
        let saved_frame = session.snapshot().frame;
        let slot_path = session.save_state_slot(1).unwrap();
        assert!(slot_path.starts_with(&state_dir));
        assert!(slot_path.is_file());
        assert!(session.has_state_slot(1));
        session.run_frames(2).unwrap();
        session.load_state_slot(1).unwrap();
        assert_eq!(session.snapshot().frame, saved_frame);

        let mut reopened = FrontendSession::new(assets.clone()).unwrap();
        reopened.load_rom(&rom_path).unwrap();
        assert!(reopened.has_state_slot(1));
        reopened.load_state_slot(1).unwrap();
        assert_eq!(reopened.snapshot().frame, saved_frame);

        let other_path = temp_dir.path().join("same-title-different-game.sfc");
        let mut modified_rom = base_rom;
        modified_rom[0x2000] ^= 0xFF; // Same title/header, different bytes.
        fs::write(&other_path, modified_rom).unwrap();
        reopened.load_rom(&other_path).unwrap();
        assert!(!reopened.has_state_slot(1));
        assert!(reopened.load_state_slot(1).is_err());
        assert_ne!(reopened.state_slot_path(1).unwrap(), slot_path);
    }

    #[test]
    fn cheat_parser_supports_raw_ram_patch_formats() {
        assert_eq!(parse_cheat_patches("7E1A2B09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B:09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B 09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B09+C2B7-6D07").len(), 1);
    }

    #[test]
    fn session_applies_enabled_raw_cheats_to_live_emulator() {
        let temp_dir = tempdir().unwrap();
        let rom_path = temp_dir.path().join("frontend-cheat-test.sfc");
        fs::write(&rom_path, synthetic_rom_bytes()).unwrap();

        let mut session = FrontendSession::new(Default::default()).unwrap();
        session.load_rom(&rom_path).unwrap();
        session.set_active_cheats(&[CheatEntry {
            id: "test-cheat".to_owned(),
            game_id: "test-game".to_owned(),
            name: "Max value".to_owned(),
            code: "7E0010:AA".to_owned(),
            source: "test".to_owned(),
            kind: "Action Replay".to_owned(),
            enabled: true,
        }]);

        session.run_frame().unwrap();

        assert_eq!(session.host_read_u8(0x7E0010), 0xAA);
    }
}
