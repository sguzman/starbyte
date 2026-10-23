//! Frontend-agnostic session orchestration and library services shared by native shells.

mod library;

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};

use crate::library::decode_single_game_genie;

use starbyte_core::{
    Emulator, EmulatorBuilder, cartridge::Cartridge, emulator::SaveState, input::ControllerState,
    manifest::AssetConfig, system::RomReadPatch,
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

/// How a frontend runs the game sound driver and handles user saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundSessionMode {
    /// Existing compatibility runtime: stable saves, no commercial music.
    Compatibility,
    /// Experimental SPC700 execution with a read-only copy of battery SRAM.
    Isolated,
    /// Opt-in SPC700 execution with persistent battery SRAM and isolated sound slots.
    Persistent,
}

impl SoundSessionMode {
    fn driver_enabled(self) -> bool {
        self != Self::Compatibility
    }

    fn saves_isolated(self) -> bool {
        self == Self::Isolated
    }
}

/// Owned RGBA screenshot pixels captured at one point in emulated time.
/// Frontends can encode this away from the machine thread without borrowing
/// the live emulator framebuffer.
#[derive(Debug, Clone)]
pub struct SessionScreenshot {
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Read-only joypad and cheat-interception counters published to user
/// interfaces; never requires borrowing the mutable SNES bus or the
/// emulation worker's machine from the GUI.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionInputDiagnostics {
    pub host_bits: u16,
    pub latched_bits: u16,
    pub reading: bool,
    pub rom_cheat_read_hits: u64,
}

/// Small, frontend-neutral menu/control state. It avoids exposing a
/// borrowed Emulator to a renderer and can be published by a single-owner
/// emulation worker without taking a UI-thread mutex.
#[derive(Debug, Clone)]
pub struct SessionControlsSnapshot {
    /// Current cartridge and emulation counters.
    pub session: SessionSnapshot,
    /// Whether the current session contains an in-memory quick save.
    pub has_quick_save: bool,
    /// Availability of the three durable state slots for this ROM namespace.
    pub state_slots: [bool; 3],
    /// Distinguishes compatibility, isolated and sound-persistent save paths.
    pub sound_mode: SoundSessionMode,
    /// Number of intercepted ROM read patches, for diagnostic menus.
    pub rom_cheat_read_hits: u64,
}

/// Frontend-agnostic emulator session used by UI shells.
#[derive(Debug)]
pub struct FrontendSession {
    emulator: Emulator,
    rom_path: Option<PathBuf>,
    cartridge_identity: Option<String>,
    active_cheat_patches: Vec<CheatPatch>,
    active_rom_patches: Vec<RomReadPatch>,
    quick_state: Option<SaveState>,
    /// Last SRAM image successfully read from or written to disk.
    last_saved_ram: Option<Vec<u8>>,
    /// Sound execution is separate from whether disk saves are writable.
    sound_mode: SoundSessionMode,
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
            active_rom_patches: Vec::new(),
            quick_state: None,
            last_saved_ram: None,
            sound_mode: SoundSessionMode::Compatibility,
        })
    }

    /// Load a ROM from disk and reset the emulator around it.
    pub fn load_rom<P>(&mut self, rom_path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        self.load_rom_inner(rom_path.as_ref(), SoundSessionMode::Compatibility)
    }

    /// Explicitly opt a single loaded ROM into host-clocked SPC700 sound
    /// execution for desktop compatibility diagnostics. Existing SRAM is
    /// loaded read-only into a private emulated copy, while durable slots
    /// remain inaccessible and neither save format is written by the probe.
    pub fn load_rom_with_isolated_audio_probe<P>(&mut self, rom_path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        self.load_rom_inner(rom_path.as_ref(), SoundSessionMode::Isolated)
    }

    /// Opt into commercial sound-driver emulation without disabling battery
    /// save persistence. Sound-enabled disk slots use a distinct namespace
    /// so they can never replace legacy compatibility-mode snapshots.
    ///
    /// This remains an explicit preview: never silently enable it for
    /// an existing player's installation before native acceptance.
    pub fn load_rom_with_persistent_audio<P>(&mut self, rom_path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        self.load_rom_inner(rom_path.as_ref(), SoundSessionMode::Persistent)
    }

    /// Current independent sound/save policy for the loaded cartridge.
    #[must_use]
    pub const fn sound_session_mode(&self) -> SoundSessionMode {
        self.sound_mode
    }

    fn load_rom_inner(&mut self, rom_path: &Path, sound_mode: SoundSessionMode) -> Result<()> {
        let path = rom_path.to_path_buf();
        let cartridge = Cartridge::load(&path)
            .with_context(|| format!("failed to load ROM at {}", path.display()))?;
        // Hash once at load time, not every egui frame while a save-slot menu
        // is visible. Slot identity follows ROM contents, not its filename.
        let mut hasher = Sha1::new();
        hasher.update(cartridge.rom());
        let identity = format!("{:x}", hasher.finalize());
        // Read the new game's save before mutating the active session. A
        // corrupt existing file must not eject the currently loaded game.
        let expected_save_len = cartridge.header().ram_size_bytes();
        let new_save_path = self
            .emulator
            .assets()
            .save_root()
            .join(format!("{identity}.srm"));
        // Flush the previous game before reading the selected cartridge's
        // SRAM. In particular, switching the *same* running game into the
        // experimental sound path must use the latest persisted progress,
        // not a disk image from before the last in-memory save.
        self.flush_save_ram()?;
        let stored_save = if expected_save_len > 0 && new_save_path.exists() {
            let data = std::fs::read(&new_save_path)
                .with_context(|| format!("failed reading SRAM {}", new_save_path.display()))?;
            anyhow::ensure!(
                data.len() == expected_save_len,
                "invalid SRAM length for {}: expected {expected_save_len}, got {}",
                new_save_path.display(),
                data.len()
            );
            Some(data)
        } else {
            None
        };

        // Isolated audio reuses a private copy of SRAM only. It never writes
        // the user's .srm file or reads/writes durable save-state slots.
        self.emulator.load_rom(cartridge);
        if sound_mode.driver_enabled() {
            self.emulator
                .enable_isolated_apu_driver_probe()
                .context("could not initialize opt-in SPC700 sound-driver execution")?;
        }
        self.sound_mode = sound_mode;
        self.cartridge_identity = Some(identity);
        self.quick_state = None;
        self.rom_path = Some(path);
        // Never replay the previous game's cheat patches into the new game.
        self.active_cheat_patches.clear();
        self.active_rom_patches.clear();
        if let Some(ref bytes) = stored_save {
            self.emulator
                .load_save_ram(bytes)
                .context("could not restore SRAM")?;
        }
        self.last_saved_ram = if sound_mode.saves_isolated() {
            None
        } else {
            self.emulator.save_ram()
        };
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

    /// Execute a guarded frame in a synchronous graphical frontend.
    pub fn run_frame_with_timeout(&mut self, timeout: Duration) -> Result<()> {
        self.apply_active_cheats();
        self.emulator
            .run_until_frame_with_timeout(timeout)
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
        self.quick_state = Some(self.emulator.capture_state());
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
            .as_ref()
            .context("No quick save in this session")?
            .clone();
        self.emulator
            .restore_state(state)
            .context("quick load failed")?;
        self.emulator.refresh_framebuffer();
        self.apply_active_cheats();
        Ok(())
    }

    /// Return the current cartridge's on-disk SRAM location, if present.
    pub fn save_ram_path(&self) -> Option<PathBuf> {
        if self.sound_mode.saves_isolated() {
            return None;
        }
        let identity = self.cartridge_identity.as_deref()?;
        let ram = self.emulator.save_ram()?;
        if ram.is_empty() {
            return None;
        }
        Some(
            self.emulator
                .assets()
                .save_root()
                .join(format!("{identity}.srm")),
        )
    }

    /// Flush changed battery-backed RAM to the disk using atomic replacement.
    /// Avoid rewriting unchanged SRAM, including during ordinary test teardown.
    pub fn flush_save_ram(&mut self) -> Result<()> {
        let Some(path) = self.save_ram_path() else {
            return Ok(());
        };
        let ram = self.emulator.save_ram().context("No SRAM in loaded game")?;
        if self.last_saved_ram.as_deref() == Some(ram.as_slice()) {
            return Ok(());
        }
        write_atomic(&path, &ram)?;
        self.last_saved_ram = Some(ram);
        Ok(())
    }

    /// Deterministic, game-scoped disk slot. Slot names never contain untrusted
    /// paths or game titles; SHA-1 is for local naming, not authentication.
    pub fn state_slot_path(&self, slot: u8) -> Result<PathBuf> {
        anyhow::ensure!(
            !self.sound_mode.saves_isolated(),
            "durable save slots are disabled in isolated sound-driver mode"
        );
        anyhow::ensure!((1..=3).contains(&slot), "save slot must be 1, 2, or 3");
        let identity = self
            .cartridge_identity
            .as_deref()
            .context("No ROM loaded for save slots")?;
        let filename = if self.sound_mode == SoundSessionMode::Persistent {
            format!("{identity}.sound.slot{slot}.state.json")
        } else {
            format!("{identity}.slot{slot}.state.json")
        };
        Ok(self.emulator.assets().state_root().join(filename))
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
        let state = self
            .emulator
            .save_state()
            .context("could not serialize state")?;
        write_atomic(&path, state.as_bytes())?;
        Ok(path)
    }

    /// Restore a durable slot for the currently loaded cartridge.
    ///
    /// The core checks full ROM bytes before any state mutation.
    pub fn load_state_slot(&mut self, slot: u8) -> Result<()> {
        let path = self.state_slot_path(slot)?;
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read save state {}", path.display()))?;
        let state: SaveState = serde_json::from_str(&data).context("failed to parse disk state")?;
        let saved_sound_enabled = state.apu.isolated_host_clocked_driver_enabled();
        let current_sound_enabled = self.sound_mode.driver_enabled();
        anyhow::ensure!(
            saved_sound_enabled == current_sound_enabled,
            "disk slot sound-mode mismatch: saved sound driver {}, active sound driver {}; switch to the corresponding game mode",
            saved_sound_enabled,
            current_sound_enabled
        );
        self.emulator
            .restore_state(state)
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
        self.active_cheat_patches.clear();
        self.active_rom_patches.clear();
        for cheat in cheats
            .iter()
            .filter(|cheat| cheat.enabled && cheat.can_execute())
        {
            if cheat.source.starts_with("Cheatarium /") && cheat.kind == "game-genie" {
                if let Some(patch) = decode_single_game_genie(&cheat.code) {
                    self.active_rom_patches.push(patch);
                }
            } else {
                self.active_cheat_patches
                    .extend(parse_cheat_patches(&cheat.code));
            }
        }
        self.apply_active_cheats();
        self.active_cheat_patches.len() + self.active_rom_patches.len()
    }

    /// Disable all runtime-applied cheats for the current session.
    pub fn clear_active_cheats(&mut self) {
        self.active_cheat_patches.clear();
        self.active_rom_patches.clear();
        self.emulator.set_rom_read_patches(&[]);
    }

    /// Copy the SNES joypad hardware readout for an input diagnostics
    /// overlay. Call from the machine owner; the renderer consumes only the
    /// resulting primitive fields.
    #[must_use]
    pub fn input_diagnostics(&self) -> SessionInputDiagnostics {
        let (host_bits, latched_bits, reading) = self.emulator.joypad_status();
        SessionInputDiagnostics {
            host_bits,
            latched_bits,
            reading,
            rom_cheat_read_hits: self.rom_cheat_read_hits(),
        }
    }

    /// Number of ROM reads observed using the current Game Genie patches.
    ///
    /// A positive count confirms runtime interception, not the cheat's effect.
    #[must_use]
    pub fn rom_cheat_read_hits(&self) -> u64 {
        self.emulator.rom_read_patch_hits()
    }

    /// Capture all lightweight menu facts under one emulator owner.
    /// Durable-slot existence is intentionally queried on demand, not at
    /// 60 Hz while generating audio/video frames.
    #[must_use]
    pub fn controls_snapshot(&self) -> SessionControlsSnapshot {
        SessionControlsSnapshot {
            session: self.snapshot(),
            has_quick_save: self.has_quick_save(),
            state_slots: [
                self.has_state_slot(1),
                self.has_state_slot(2),
                self.has_state_slot(3),
            ],
            sound_mode: self.sound_session_mode(),
            rom_cheat_read_hits: self.rom_cheat_read_hits(),
        }
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

    /// Copy a still image only when explicitly requested by the user,
    /// never as part of ordinary menu redraws.
    pub fn capture_screenshot(&self) -> Option<SessionScreenshot> {
        let status = self.snapshot();
        if !status.has_rom {
            return None;
        }
        Some(SessionScreenshot {
            frame: status.frame,
            width: status.framebuffer_width,
            height: status.framebuffer_height,
            rgba: self.framebuffer_rgba().to_vec(),
        })
    }

    /// Generated stereo PCM for the most recently emulated SNES frame.
    /// The slice is borrowed only within its owning thread. A future
    /// worker-powered UI must consume published counts or a bounded PCM
    /// stream, never borrow this directly across threads.
    #[must_use]
    pub fn frame_audio_samples(&self) -> &[i16] {
        &self.emulator.audio_samples().samples
    }

    /// Compact flag used by the status bar. The renderer must not inspect
    /// the emulator's experimental audio driver directly.
    #[must_use]
    pub fn isolated_audio_driver_enabled(&self) -> bool {
        self.emulator.isolated_apu_driver_probe_enabled()
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
        // Guest save-state restores replace the bus. Reapply host-managed ROM
        // substitutions after restore and on subsequent gameplay frames.
        self.emulator.set_rom_read_patches(&self.active_rom_patches);
        for patch in self.active_cheat_patches.iter().copied() {
            self.emulator.host_write_u8(patch.address, patch.value);
        }
    }
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("save path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create save directory {}", parent.display()))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).context("failed to create temporary save file")?;
    temp.write_all(content)
        .context("failed to write temporary save")?;
    temp.as_file()
        .sync_all()
        .context("failed to sync temporary save")?;
    temp.persist(path)
        .with_context(|| format!("failed to commit save to {}", path.display()))?;
    Ok(())
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
    fn frontend_controller_state_reaches_emulated_snes_auto_joypad_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("joypad-integration.sfc");
        let mut rom = synthetic_rom_bytes();
        // LDA #$01 / STA $4200 / BRA * : enable SNES auto joypad polling.
        rom[..7].copy_from_slice(&[0xA9, 0x01, 0x8D, 0x00, 0x42, 0x80, 0xFE]);
        fs::write(&path, rom).unwrap();

        let mut session = FrontendSession::new(Default::default()).unwrap();
        session.load_rom(&path).unwrap();
        let pressed = ControllerState {
            start: true,
            right: true,
            ..ControllerState::default()
        };
        session.set_controller1(pressed);
        session.run_frames(3).unwrap();
        let (host, latched, busy) = session.emulator().joypad_status();
        assert_eq!(host, pressed.to_bits());
        assert_eq!(latched, pressed.to_bits());
        assert!(!busy);
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
    fn battery_backed_sram_is_saved_on_flush_and_game_switch() {
        let dir = tempdir().unwrap();
        let rom = dir.path().join("first.sfc");
        let second_rom = dir.path().join("other.sfc");
        let save_dir = dir.path().join("battery");
        let original = synthetic_rom_bytes();
        fs::write(&rom, &original).unwrap();
        let mut changed = original.clone();
        changed[0x2100] = 0xA9; // Same title, different content.
        fs::write(&second_rom, changed).unwrap();
        let assets = AssetConfig {
            save_dir: Some(save_dir.clone()),
            ..AssetConfig::default()
        };

        let mut session = FrontendSession::new(assets.clone()).unwrap();
        session.load_rom(&rom).unwrap();
        session.set_active_cheats(&[CheatEntry {
            id: "sram-cheat".to_owned(),
            game_id: "first".to_owned(),
            name: "Store test byte".to_owned(),
            code: "700000:5A".to_owned(),
            source: "test".to_owned(),
            kind: "Action Replay".to_owned(),
            enabled: true,
        }]);
        session.run_frame().unwrap();
        session.clear_active_cheats();
        assert_eq!(session.host_read_u8(0x700000), 0x5A);
        let path = session.save_ram_path().unwrap();
        assert!(path.starts_with(&save_dir));
        session.flush_save_ram().unwrap();
        assert_eq!(fs::read(&path).unwrap()[0], 0x5A);

        // Game switching must not erase the old SRAM or import old cheats.
        session.load_rom(&second_rom).unwrap();
        assert_ne!(session.save_ram_path(), Some(path.clone()));
        session.load_rom(&rom).unwrap();
        assert_eq!(session.host_read_u8(0x700000), 0x5A);

        let mut reopened = FrontendSession::new(assets.clone()).unwrap();
        reopened.load_rom(&rom).unwrap();
        assert_eq!(reopened.host_read_u8(0x700000), 0x5A);

        // A corrupt save must not eject the already loaded game.
        // The invalid data is written to the content-scoped identity path.
        let mut probe = FrontendSession::new(assets).unwrap();
        probe.load_rom(&second_rom).unwrap();
        let second_path = probe.save_ram_path().unwrap();
        fs::write(&second_path, [0xFF]).unwrap();
        assert!(reopened.load_rom(&second_rom).is_err());
        assert_eq!(reopened.loaded_rom_path(), Some(rom.as_path()));
    }

    #[test]
    fn experimental_desktop_probe_executes_synthetically_uploaded_spc700_driver() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("smp-driver-probe.sfc");
        let mut rom = synthetic_rom_bytes();
        let driver = [0xe8, 0x2a, 0xc4, 0xf4, 0x2f, 0xfe];
        fn append_port_write(code: &mut Vec<u8>, port: u8, value: u8) {
            code.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
        }

        // Original 65816 program; no commercial ROM bytes. The host uploads
        // a small sound program and then parks in a bounded BRA loop.
        let mut code = Vec::new();
        append_port_write(&mut code, 1, 1);
        append_port_write(&mut code, 2, 0);
        append_port_write(&mut code, 3, 4);
        append_port_write(&mut code, 0, 0xcc);
        for (index, byte) in driver.into_iter().enumerate() {
            append_port_write(&mut code, 1, byte);
            append_port_write(&mut code, 0, index as u8);
        }
        append_port_write(&mut code, 1, 0);
        append_port_write(&mut code, 2, 0);
        append_port_write(&mut code, 3, 4);
        append_port_write(&mut code, 0, 7);
        code.extend_from_slice(&[0x80, 0xfe]);
        rom[..code.len()].copy_from_slice(&code);
        fs::write(&rom_path, rom).unwrap();

        let mut session = FrontendSession::new(AssetConfig::default()).unwrap();
        session
            .load_rom_with_isolated_audio_probe(&rom_path)
            .unwrap();
        session.run_frame().unwrap();

        let upload = session.emulator().isolated_apu_upload().unwrap();
        assert_eq!(upload.bytes_written(), 6);
        assert_eq!(upload.entrypoint(), Some(0x0400));
        assert_eq!(session.emulator().apu_to_cpu_ports()[0], 0x2a);
        let status = session.emulator().isolated_apu_spc700_status().unwrap();
        assert!(status.instructions > 2);
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert!(session.save_ram_path().is_none());

        session.load_rom(&rom_path).unwrap();
        assert!(!session.emulator().isolated_apu_driver_probe_enabled());
        assert!(session.emulator().isolated_apu_upload().is_none());
    }

    #[test]
    fn experimental_frontend_probe_generates_real_dsp_pcm_from_original_driver() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("original-brr-audio.sfc");
        let mut driver = Vec::new();
        for (register, value) in [
            (0x5d, 0x02), // Sample directory base.
            (0x00, 0x7f), // Voice 0 left.
            (0x01, 0x7f), // Voice 0 right.
            (0x03, 0x10), // Sample pitch.
            (0x07, 0x7f), // Gain.
            (0x0c, 0x7f), // Master volume left.
            (0x1c, 0x7f), // Master volume right.
            (0x4c, 0x01), // Key on voice 0.
        ] {
            driver.extend_from_slice(&[0x8f, register, 0xf2, 0x8f, value, 0xf3]);
        }
        driver.extend_from_slice(&[0x2f, 0xfe]); // Stable sound CPU BRA loop.

        fn port_write(code: &mut Vec<u8>, port: u8, value: u8) {
            code.extend_from_slice(&[0xa9, value, 0x8d, 0x40 + port, 0x21]);
        }
        let mut cpu = Vec::new();
        // Original synthetic IPL upload: BRR directory at $0200.
        port_write(&mut cpu, 1, 1);
        port_write(&mut cpu, 2, 0);
        port_write(&mut cpu, 3, 2);
        port_write(&mut cpu, 0, 0xcc);
        for (index, byte) in [0, 3, 0, 3].into_iter().enumerate() {
            port_write(&mut cpu, 1, byte);
            port_write(&mut cpu, 0, index as u8);
        }
        // Looping, copyright-free BRR waveform at $0300.
        port_write(&mut cpu, 1, 1);
        port_write(&mut cpu, 2, 0);
        port_write(&mut cpu, 3, 3);
        port_write(&mut cpu, 0, 5);
        for (index, byte) in [0xc3, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77, 0x77]
            .into_iter()
            .enumerate()
        {
            port_write(&mut cpu, 1, byte);
            port_write(&mut cpu, 0, index as u8);
        }
        // Upload MMIO-programming sound driver at $0400.
        port_write(&mut cpu, 1, 1);
        port_write(&mut cpu, 2, 0);
        port_write(&mut cpu, 3, 4);
        port_write(&mut cpu, 0, 10);
        for (index, byte) in driver.iter().copied().enumerate() {
            port_write(&mut cpu, 1, byte);
            port_write(&mut cpu, 0, index as u8);
        }
        port_write(&mut cpu, 1, 0);
        port_write(&mut cpu, 2, 0);
        port_write(&mut cpu, 3, 4);
        port_write(&mut cpu, 0, driver.len() as u8 + 1);
        cpu.extend_from_slice(&[0x80, 0xfe]); // Stable 65816 BRA loop.

        let mut rom = synthetic_rom_bytes();
        rom[..cpu.len()].copy_from_slice(&cpu);
        fs::write(&rom_path, rom).unwrap();

        let mut session = FrontendSession::new(AssetConfig {
            save_dir: Some(dir.path().join("saves")),
            state_dir: Some(dir.path().join("states")),
            ..AssetConfig::default()
        })
        .unwrap();
        session
            .load_rom_with_isolated_audio_probe(&rom_path)
            .unwrap();
        session.run_frame().unwrap();
        assert_eq!(
            session
                .emulator()
                .isolated_apu_upload()
                .unwrap()
                .entrypoint(),
            Some(0x0400)
        );
        assert_eq!(session.emulator().isolated_apu_dsp_register(0x4c), Some(1));
        assert!(
            session
                .emulator()
                .audio_samples()
                .samples
                .iter()
                .any(|&sample| sample > 1_000),
            "explicitly uploaded original BRR driver must reach frontend PCM"
        );
        assert!(
            session
                .emulator()
                .audio_samples()
                .samples
                .chunks_exact(2)
                .all(|pair| pair[0] == pair[1])
        );

        // Reload the same cartridge ordinarily: no implicit upload or audio.
        session.load_rom(&rom_path).unwrap();
        session.run_frame().unwrap();
        assert!(session.emulator().isolated_apu_upload().is_none());
        assert!(
            session
                .emulator()
                .audio_samples()
                .samples
                .iter()
                .all(|&sample| sample == 0)
        );

        // Identical guest APU execution must work in the sound-with-saves
        // preview, not just in the previous read-only diagnostic mode.
        session.load_rom_with_persistent_audio(&rom_path).unwrap();
        session.run_frame().unwrap();
        assert_eq!(
            session.sound_session_mode(),
            super::SoundSessionMode::Persistent
        );
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert!(
            session
                .emulator()
                .audio_samples()
                .samples
                .iter()
                .any(|&sample| sample > 1_000),
            "sound-with-saves preview must synthesize the uploaded BRR driver"
        );
        assert!(session.save_ram_path().is_some());

        // A sound-mode disk state must restore not just SPC RAM and flags,
        // but fractional SPC/DSP phase and the future BRR output sample-for-
        // sample. This uses only an original synthetic driver (no game ROM).
        let sound_slot = session.save_state_slot(1).unwrap();
        assert!(sound_slot.is_file());
        let saved_frame = session.snapshot().frame;
        session.run_frame().unwrap();
        let expected_first = session.emulator().audio_samples().samples.clone();
        session.run_frame().unwrap();
        let expected_second = session.emulator().audio_samples().samples.clone();
        assert!(
            expected_first.iter().any(|&sample| sample != 0)
                || expected_second.iter().any(|&sample| sample != 0),
            "sound-mode replay fixture must actually emit audio"
        );
        session.load_state_slot(1).unwrap();
        assert_eq!(session.snapshot().frame, saved_frame);
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        session.run_frame().unwrap();
        assert_eq!(
            session.emulator().audio_samples().samples,
            expected_first,
            "first restored frame must reproduce the saved sound phase"
        );
        session.run_frame().unwrap();
        assert_eq!(
            session.emulator().audio_samples().samples,
            expected_second,
            "second restored frame must continue the same sound timeline"
        );
    }

    #[test]
    fn isolated_audio_probe_loads_progress_read_only_and_never_overwrites_saves() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("sound-probe.sfc");
        fs::write(&rom_path, synthetic_rom_bytes()).unwrap();
        let assets = AssetConfig {
            save_dir: Some(dir.path().join("save")),
            state_dir: Some(dir.path().join("states")),
            ..AssetConfig::default()
        };
        let mut session = FrontendSession::new(assets).unwrap();
        session.load_rom(&rom_path).unwrap();
        let save_path = session.save_ram_path().unwrap();
        let initial_sram = session.emulator().save_ram().unwrap();
        assert!(!initial_sram.is_empty());
        let mut existing_sram = initial_sram;
        existing_sram[0] = 0x5a;
        std::fs::create_dir_all(save_path.parent().unwrap()).unwrap();
        fs::write(&save_path, &existing_sram).unwrap();
        session.save_state_slot(1).unwrap();
        let existing_slot = session.state_slot_path(1).unwrap();
        let original_slot = fs::read(&existing_slot).unwrap();

        session
            .load_rom_with_isolated_audio_probe(&rom_path)
            .unwrap();
        assert!(session.emulator().isolated_apu_upload().is_some());
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x5a);
        assert!(session.save_ram_path().is_none());
        assert!(!session.has_state_slot(1));
        assert!(session.save_state_slot(1).is_err());
        assert!(session.load_state_slot(1).is_err());
        // Probe RAM is writable *inside emulation*, never as a durable save.
        let mut private_ram = session.emulator().save_ram().unwrap();
        private_ram[0] = 0xc3;
        session.emulator.load_save_ram(&private_ram).unwrap();
        session.flush_save_ram().unwrap();
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0xc3);
        assert_eq!(fs::read(&save_path).unwrap(), existing_sram);
        assert_eq!(fs::read(&existing_slot).unwrap(), original_slot);

        session.load_rom(&rom_path).unwrap();
        assert!(!session.emulator().isolated_apu_driver_probe_enabled());
        assert!(session.emulator().isolated_apu_upload().is_none());
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x5a);
        assert_eq!(session.save_ram_path(), Some(save_path));
        assert!(session.has_state_slot(1));
    }

    #[test]
    fn persistent_sound_mode_saves_sram_without_overwriting_legacy_state_slots() {
        use super::SoundSessionMode;

        let dir = tempdir().unwrap();
        let rom = dir.path().join("sound-persistent.sfc");
        fs::write(&rom, synthetic_rom_bytes()).unwrap();
        let assets = AssetConfig {
            save_dir: Some(dir.path().join("save")),
            state_dir: Some(dir.path().join("states")),
            ..AssetConfig::default()
        };
        let mut session = FrontendSession::new(assets).unwrap();
        session.load_rom(&rom).unwrap();
        assert_eq!(
            session.sound_session_mode(),
            SoundSessionMode::Compatibility
        );
        let original_sram_path = session.save_ram_path().unwrap();
        let mut ram = session.emulator().save_ram().unwrap();
        ram[0] = 0x42;
        session.emulator.load_save_ram(&ram).unwrap();
        session.flush_save_ram().unwrap();
        let legacy_slot = session.save_state_slot(1).unwrap();
        let legacy_bytes = fs::read(&legacy_slot).unwrap();

        session.load_rom_with_persistent_audio(&rom).unwrap();
        assert_eq!(session.sound_session_mode(), SoundSessionMode::Persistent);
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x42);
        assert_eq!(session.save_ram_path(), Some(original_sram_path.clone()));
        assert!(!session.has_state_slot(1));
        let sound_slot = session.save_state_slot(1).unwrap();
        assert_ne!(legacy_slot, sound_slot);
        assert!(sound_slot.is_file());
        assert_eq!(fs::read(&legacy_slot).unwrap(), legacy_bytes);

        ram[0] = 0x99;
        session.emulator.load_save_ram(&ram).unwrap();
        session.flush_save_ram().unwrap();
        assert_eq!(fs::read(&original_sram_path).unwrap()[0], 0x99);
        session.quick_save().unwrap();
        session.quick_load().unwrap();
        session.load_state_slot(1).unwrap();
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x42);
        // Flush restored RAM as usual; loading sound-mode state does not
        // rewrite legacy files or redirect SRAM into a private temp copy.
        session.flush_save_ram().unwrap();
        assert_eq!(fs::read(&original_sram_path).unwrap()[0], 0x42);
        assert_eq!(fs::read(&legacy_slot).unwrap(), legacy_bytes);

        session.load_rom(&rom).unwrap();
        assert_eq!(
            session.sound_session_mode(),
            SoundSessionMode::Compatibility
        );
        assert!(!session.emulator().isolated_apu_driver_probe_enabled());
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x42);
        assert_eq!(session.state_slot_path(1).unwrap(), legacy_slot);
        session.load_state_slot(1).unwrap();
        assert!(!session.emulator().isolated_apu_driver_probe_enabled());
    }

    #[test]
    fn importing_a_silent_disk_slot_into_sound_mode_is_rejected_before_mutation() {
        let dir = tempdir().unwrap();
        let rom = dir.path().join("audio-slot-validation.sfc");
        fs::write(&rom, synthetic_rom_bytes()).unwrap();
        let assets = AssetConfig {
            state_dir: Some(dir.path().join("states")),
            save_dir: Some(dir.path().join("saves")),
            ..Default::default()
        };
        let mut session = FrontendSession::new(assets).unwrap();
        session.load_rom(&rom).unwrap();
        let silent_slot = session.save_state_slot(2).unwrap();
        session.load_rom_with_persistent_audio(&rom).unwrap();
        let audio_slot = session.state_slot_path(2).unwrap();
        assert_ne!(audio_slot, silent_slot);
        fs::copy(&silent_slot, &audio_slot).unwrap();
        assert!(session.load_state_slot(2).is_err());
        assert!(session.emulator().isolated_apu_driver_probe_enabled());
        assert_eq!(
            session.sound_session_mode(),
            super::SoundSessionMode::Persistent
        );
        // Rejection must not remove the misplaced file or mutate the original.
        assert!(audio_slot.is_file());
        assert!(silent_slot.is_file());
        assert_eq!(
            fs::read(&audio_slot).unwrap(),
            fs::read(&silent_slot).unwrap()
        );
    }

    #[test]
    fn changing_from_persistent_sound_to_isolated_never_writes_probe_ram() {
        let dir = tempdir().unwrap();
        let rom = dir.path().join("sound-isolated.sfc");
        fs::write(&rom, synthetic_rom_bytes()).unwrap();
        let assets = AssetConfig {
            save_dir: Some(dir.path().join("save")),
            state_dir: Some(dir.path().join("states")),
            ..Default::default()
        };
        let mut session = FrontendSession::new(assets).unwrap();
        session.load_rom_with_persistent_audio(&rom).unwrap();
        let save = session.save_ram_path().unwrap();
        let mut ram = session.emulator().save_ram().unwrap();
        ram[0] = 0x5a;
        session.emulator.load_save_ram(&ram).unwrap();
        // Entering a read-only diagnostic first commits pending persistent
        // progress, then imports it privately.
        session.load_rom_with_isolated_audio_probe(&rom).unwrap();
        assert_eq!(fs::read(&save).unwrap()[0], 0x5a);
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x5a);
        ram[0] = 0xbe;
        session.emulator.load_save_ram(&ram).unwrap();
        session.flush_save_ram().unwrap();
        assert_eq!(fs::read(&save).unwrap()[0], 0x5a);
        session.load_rom_with_persistent_audio(&rom).unwrap();
        assert_eq!(session.emulator().save_ram().unwrap()[0], 0x5a);
    }

    #[test]
    fn cheat_parser_supports_raw_ram_patch_formats() {
        assert_eq!(parse_cheat_patches("7E1A2B09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B:09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B 09").len(), 1);
        assert_eq!(parse_cheat_patches("7E1A2B09+C2B7-6D07").len(), 1);
    }

    #[test]
    fn cheatarium_game_genie_changes_rom_reads_only_while_enabled() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("genie-test.sfc");
        let mut bytes = synthetic_rom_bytes();
        // DDB4-6F07 decodes to $00:9E25 -> $00, a LoROM cartridge read.
        bytes[0x1E25] = 0x64;
        // LDA $9E25; STA $0010; BRA * -- this probes actual CPU bus reads,
        // not only host inspection of the patched cartridge address.
        bytes[..8].copy_from_slice(&[0xAD, 0x25, 0x9E, 0x8D, 0x10, 0x00, 0x80, 0xFE]);
        fs::write(&rom_path, bytes).unwrap();

        let mut session = FrontendSession::new(AssetConfig {
            state_dir: Some(dir.path().join("states")),
            save_dir: Some(dir.path().join("saves")),
            ..Default::default()
        })
        .unwrap();
        session.load_rom(&rom_path).unwrap();
        assert_eq!(session.host_read_u8(0x009E25), 0x64);
        let mut cheat = CheatEntry {
            id: "one-life".to_owned(),
            game_id: "snes-test".to_owned(),
            name: "Game Genie substitution".to_owned(),
            code: "DDB4-6F07".to_owned(),
            source: "Cheatarium / test".to_owned(),
            kind: "game-genie".to_owned(),
            enabled: true,
        };
        assert_eq!(session.set_active_cheats(&[cheat.clone()]), 1);
        assert_eq!(session.rom_cheat_read_hits(), 0);
        assert_eq!(session.host_read_u8(0x009E25), 0x00);
        assert_eq!(session.rom_cheat_read_hits(), 1);
        assert_eq!(session.host_read_u8(0x809E25), 0x64);
        assert_eq!(session.emulator().cartridge().unwrap().rom()[0x1E25], 0x64);
        session.run_frame().unwrap();
        assert!(session.rom_cheat_read_hits() > 1);
        assert_eq!(session.host_read_u8(0x7E0010), 0x00);

        // Both restore paths keep the host's chosen cheats, not the saved game's.
        session.quick_save().unwrap();
        session.quick_load().unwrap();
        assert_eq!(session.host_read_u8(0x009E25), 0x00);
        let path = session.save_state_slot(1).unwrap();
        session.load_state_slot(1).unwrap();
        assert_eq!(session.host_read_u8(0x009E25), 0x00);
        assert!(path.is_file());

        cheat.enabled = false;
        assert_eq!(session.set_active_cheats(&[cheat]), 0);
        assert_eq!(session.rom_cheat_read_hits(), 0);
        assert_eq!(session.host_read_u8(0x009E25), 0x64);
        assert_eq!(session.rom_cheat_read_hits(), 0);
        // A clean unpatched replay must execute the same CPU program but
        // store the actual cartridge byte, not the former Game Genie value.
        session.load_rom(&rom_path).unwrap();
        session.run_frame().unwrap();
        assert_eq!(session.host_read_u8(0x7E0010), 0x64);
    }

    #[test]
    fn cheatarium_rejects_unresolved_compound_genie_and_stale_cheats() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("genie-reject-test.sfc");
        fs::write(&rom_path, synthetic_rom_bytes()).unwrap();
        let mut session = FrontendSession::new(Default::default()).unwrap();
        session.load_rom(&rom_path).unwrap();
        let unreviewed = CheatEntry {
            id: "compound".to_owned(),
            game_id: "test".to_owned(),
            name: "Unknown join".to_owned(),
            code: "DDB4-6F07+D6B4-6F07".to_owned(),
            source: "Cheatarium / test".to_owned(),
            kind: "unverified".to_owned(),
            enabled: true,
        };
        assert_eq!(session.set_active_cheats(&[unreviewed]), 0);
        session.clear_active_cheats();
        assert_eq!(session.host_read_u8(0x009E25), 0x00);
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

#[cfg(test)]
mod session_controls_snapshot_tests {
    use super::{FrontendSession, SoundSessionMode};
    use starbyte_core::manifest::AssetConfig;

    #[test]
    fn empty_session_has_no_saves_or_loaded_cartridge() {
        let session = FrontendSession::new(AssetConfig::default()).unwrap();
        let summary = session.controls_snapshot();
        assert!(!summary.session.has_rom);
        assert!(!summary.has_quick_save);
        assert_eq!(summary.state_slots, [false; 3]);
        assert_eq!(summary.sound_mode, SoundSessionMode::Compatibility);
        assert_eq!(summary.rom_cheat_read_hits, 0);
        let input = session.input_diagnostics();
        assert_eq!(input.host_bits, 0);
        assert_eq!(input.latched_bits, 0);
        assert!(!input.reading);
        assert_eq!(input.rom_cheat_read_hits, 0);
        assert!(session.capture_screenshot().is_none());
    }
}
