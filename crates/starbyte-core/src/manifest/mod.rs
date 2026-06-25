//! Runtime configuration and asset path manifest types.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::Result;

/// Host asset locations resolved by the CLI or future frontends.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetConfig {
    /// Optional SPC700 IPL ROM location.
    pub spc700_ipl: Option<PathBuf>,
    /// Optional base directory for saves.
    pub save_dir: Option<PathBuf>,
    /// Optional base directory for save states.
    pub state_dir: Option<PathBuf>,
    /// Optional base directory for cached metadata, covers, and cheats.
    pub cache_dir: Option<PathBuf>,
    /// Optional runtime configuration path.
    pub config_path: Option<PathBuf>,
}

impl AssetConfig {
    /// Resolve the effective cache root for frontend/library data.
    #[must_use]
    pub fn cache_root(&self) -> PathBuf {
        self.cache_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(".cache").join("starbyte"))
    }

    /// Resolve the effective configuration path for persisted GUI/runtime settings.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.config_path
            .clone()
            .unwrap_or_else(|| self.cache_root().join("config.toml"))
    }
}

/// Library presentation mode shared by persistent config and host shells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryViewMode {
    /// Compact row-oriented presentation.
    List,
    /// Cover-forward card presentation.
    Grid,
    /// Metadata-heavy expanded presentation.
    Detailed,
}

impl Default for LibraryViewMode {
    fn default() -> Self {
        Self::Grid
    }
}

/// Persistent audio options for frontend shells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    /// Whether audio playback should be enabled.
    pub enabled: bool,
    /// Output volume as a normalized scalar.
    pub volume: f32,
    /// Requested output sample rate for future host backends.
    pub sample_rate_hz: u32,
    /// Whether audio should be muted on startup.
    pub mute_on_startup: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 1.0,
            sample_rate_hz: 48_000,
            mute_on_startup: false,
        }
    }
}

/// Persistent video options for frontend shells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    /// Whether the frontend should start in fullscreen.
    pub fullscreen: bool,
    /// Integer scale preference for pixel output.
    pub integer_scale: bool,
    /// Preferred window scale multiplier.
    pub scale: u32,
    /// Whether vsync should be requested.
    pub vsync: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            fullscreen: false,
            integer_scale: true,
            scale: 3,
            vsync: true,
        }
    }
}

/// Persistent input options for frontend shells.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InputSettings {
    /// Optional keyboard/controller bindings keyed by frontend-facing action name.
    pub bindings: BTreeMap<String, String>,
}

/// Persistent cheat-management settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheatSettings {
    /// Enabled cheat identifiers grouped by stable game id.
    pub enabled_by_game: BTreeMap<String, Vec<String>>,
    /// Whether cheats should be visible in the library by default.
    pub show_cheat_badges: bool,
}

/// Persistent library-management settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Directories searched recursively for ROM images.
    pub rom_dirs: Vec<PathBuf>,
    /// Current library presentation mode.
    pub active_view: LibraryViewMode,
    /// Whether to filter the library down to installed entries only.
    pub show_installed_only: bool,
    /// Optional cache root override for library data.
    pub cache_dir: Option<PathBuf>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            rom_dirs: Vec::new(),
            active_view: LibraryViewMode::default(),
            show_installed_only: false,
            cache_dir: None,
        }
    }
}

/// Remote provider settings and refresh bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Whether network-backed metadata refresh is enabled.
    pub enable_network: bool,
    /// Metadata index endpoint.
    pub metadata_index_url: String,
    /// Cover-art tree endpoint.
    pub cover_index_url: String,
    /// Cheat index endpoint.
    pub cheat_index_url: String,
    /// Last successful metadata refresh timestamp.
    pub last_metadata_refresh_unix: Option<u64>,
    /// Last successful cover refresh timestamp.
    pub last_cover_refresh_unix: Option<u64>,
    /// Last successful cheat refresh timestamp.
    pub last_cheat_refresh_unix: Option<u64>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enable_network: true,
            metadata_index_url: "https://api.github.com/repos/libretro-thumbnails/Nintendo_-_Super_Nintendo_Entertainment_System/git/trees/master?recursive=1".to_owned(),
            cover_index_url: "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Super_Nintendo_Entertainment_System/master".to_owned(),
            cheat_index_url: "https://api.github.com/repos/libretro/libretro-database/git/trees/master?recursive=1".to_owned(),
            last_metadata_refresh_unix: None,
            last_cover_refresh_unix: None,
            last_cheat_refresh_unix: None,
        }
    }
}

/// Advanced frontend/cache settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedSettings {
    /// Whether missing library entries should be shown.
    pub show_missing_games: bool,
    /// Whether provider refresh actions should run automatically on startup.
    pub refresh_on_startup: bool,
    /// Provider-specific configuration and refresh bookkeeping.
    pub providers: ProviderSettings,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            show_missing_games: true,
            refresh_on_startup: false,
            providers: ProviderSettings::default(),
        }
    }
}

/// User-tunable runtime options that should survive frontend changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Logging filter used by `tracing_subscriber`.
    pub log_filter: String,
    /// Whether the frontend should start in dark mode.
    pub prefer_dark_mode: bool,
    /// Frontend/library-facing audio options.
    pub audio: AudioSettings,
    /// Frontend/library-facing video options.
    pub video: VideoSettings,
    /// Frontend/library-facing input options.
    pub input: InputSettings,
    /// Frontend/library-facing cheat options.
    pub cheats: CheatSettings,
    /// Frontend/library-facing library options.
    pub library: LibrarySettings,
    /// Advanced/cache/provider options.
    pub advanced: AdvancedSettings,
}

impl RuntimeConfig {
    /// Return the default configuration path used by CLI and GUI shells.
    #[must_use]
    pub fn default_path() -> PathBuf {
        PathBuf::from(".cache").join("starbyte").join("config.toml")
    }

    /// Load a config file if it exists, otherwise return defaults.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).map_err(|source| crate::Error::io(path, source))?;
        Ok(toml::from_str(&text)?)
    }

    /// Serialize this config to the provided path, creating parent directories first.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| crate::Error::io(parent, source))?;
        }
        fs::write(path, toml::to_string_pretty(self)?)
            .map_err(|source| crate::Error::io(path, source))?;
        Ok(())
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            log_filter: "info,starbyte_core=debug,starbyte_cli=debug".to_owned(),
            prefer_dark_mode: true,
            audio: AudioSettings::default(),
            video: VideoSettings::default(),
            input: InputSettings::default(),
            cheats: CheatSettings::default(),
            library: LibrarySettings::default(),
            advanced: AdvancedSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{AssetConfig, LibraryViewMode, RuntimeConfig};

    #[test]
    fn asset_config_resolves_default_paths() {
        let config = AssetConfig::default();
        assert_eq!(config.cache_root(), std::path::PathBuf::from(".cache").join("starbyte"));
        assert_eq!(config.config_path(), std::path::PathBuf::from(".cache").join("starbyte").join("config.toml"));
    }

    #[test]
    fn runtime_config_roundtrips_toml() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("starbyte.toml");
        let mut config = RuntimeConfig::default();
        config.library.active_view = LibraryViewMode::Detailed;
        config.library.rom_dirs.push(temp_dir.path().join("roms"));
        config.cheats
            .enabled_by_game
            .insert("abc123".to_owned(), vec!["infinite-lives".to_owned()]);
        config.save_to_path(&path).unwrap();

        let loaded = RuntimeConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.library.active_view, LibraryViewMode::Detailed);
        assert_eq!(loaded.library.rom_dirs.len(), 1);
        assert_eq!(
            loaded
                .cheats
                .enabled_by_game
                .get("abc123")
                .cloned()
                .unwrap_or_default(),
            vec!["infinite-lives".to_owned()]
        );
    }
}
