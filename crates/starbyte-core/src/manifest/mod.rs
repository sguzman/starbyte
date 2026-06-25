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
            .unwrap_or_else(|| PathBuf::from(".config").join("starbyte").join("config.toml"))
    }

    /// Resolve the legacy configuration path used before config relocation.
    #[must_use]
    pub fn legacy_config_path(&self) -> PathBuf {
        self.cache_root().join("config.toml")
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDeviceMode {
    /// Keyboard bindings drive the controller state.
    Keyboard,
    /// A connected gamepad drives the controller state.
    Gamepad,
}

impl Default for InputDeviceMode {
    fn default() -> Self {
        Self::Keyboard
    }
}

/// Persistent input options for frontend shells.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSettings {
    /// Active input backend.
    #[serde(default)]
    pub active_device: InputDeviceMode,
    /// Keyboard bindings keyed by frontend-facing action name.
    #[serde(default = "default_keyboard_bindings")]
    pub keyboard_bindings: BTreeMap<String, String>,
    /// Gamepad bindings keyed by frontend-facing action name.
    #[serde(default = "default_gamepad_bindings")]
    pub gamepad_bindings: BTreeMap<String, String>,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            active_device: InputDeviceMode::Keyboard,
            keyboard_bindings: default_keyboard_bindings(),
            gamepad_bindings: default_gamepad_bindings(),
        }
    }
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

/// Runtime shell mode used to tune logging and diagnostics defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    /// Development mode with verbose local diagnostics.
    Dev,
    /// Production mode with quieter local side effects.
    Prod,
}

impl Default for AppMode {
    fn default() -> Self {
        Self::Dev
    }
}

/// Persistent GUI layout and logging panel settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    /// Whether the left settings/navigation column is visible.
    pub show_left_panel: bool,
    /// Whether the right session column is visible.
    pub show_right_panel: bool,
    /// Whether the details/cover column is visible.
    pub show_details_panel: bool,
    /// Whether the bottom log panel is visible.
    pub show_log_panel: bool,
    /// Whether the log view should auto-scroll as new lines arrive.
    pub log_auto_scroll: bool,
    /// Desired width for the left settings/navigation column.
    pub left_panel_width: f32,
    /// Desired width for the library browser column.
    pub library_panel_width: f32,
    /// Desired width for the details/cover column.
    pub details_panel_width: f32,
    /// Desired width for the right session column.
    pub right_panel_width: f32,
    /// Desired height for the bottom log panel.
    pub log_panel_height: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_left_panel: true,
            show_right_panel: true,
            show_details_panel: true,
            show_log_panel: true,
            log_auto_scroll: true,
            left_panel_width: 280.0,
            library_panel_width: 420.0,
            details_panel_width: 340.0,
            right_panel_width: 320.0,
            log_panel_height: 180.0,
        }
    }
}

/// User-tunable runtime options that should survive frontend changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Runtime operating mode.
    #[serde(default)]
    pub mode: AppMode,
    /// Logging filter used by `tracing_subscriber`.
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
    /// Whether the frontend should start in dark mode.
    #[serde(default = "default_prefer_dark_mode")]
    pub prefer_dark_mode: bool,
    /// Frontend/library-facing audio options.
    #[serde(default)]
    pub audio: AudioSettings,
    /// Frontend/library-facing video options.
    #[serde(default)]
    pub video: VideoSettings,
    /// Frontend/library-facing input options.
    #[serde(default)]
    pub input: InputSettings,
    /// Frontend/library-facing cheat options.
    #[serde(default)]
    pub cheats: CheatSettings,
    /// Frontend/library-facing library options.
    #[serde(default)]
    pub library: LibrarySettings,
    /// Advanced/cache/provider options.
    #[serde(default)]
    pub advanced: AdvancedSettings,
    /// Frontend shell layout and logging pane preferences.
    #[serde(default)]
    pub ui: UiSettings,
}

impl RuntimeConfig {
    /// Return the default configuration path used by CLI and GUI shells.
    #[must_use]
    pub fn default_path() -> PathBuf {
        PathBuf::from(".config").join("starbyte").join("config.toml")
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
            mode: AppMode::default(),
            log_filter: default_log_filter(),
            prefer_dark_mode: default_prefer_dark_mode(),
            audio: AudioSettings::default(),
            video: VideoSettings::default(),
            input: InputSettings::default(),
            cheats: CheatSettings::default(),
            library: LibrarySettings::default(),
            advanced: AdvancedSettings::default(),
            ui: UiSettings::default(),
        }
    }
}

fn default_log_filter() -> String {
    "info,starbyte_frontend=debug,starbyte_egui=debug".to_owned()
}

const fn default_prefer_dark_mode() -> bool {
    true
}

fn default_keyboard_bindings() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("up".to_owned(), "ArrowUp".to_owned()),
        ("down".to_owned(), "ArrowDown".to_owned()),
        ("left".to_owned(), "ArrowLeft".to_owned()),
        ("right".to_owned(), "ArrowRight".to_owned()),
        ("start".to_owned(), "Enter".to_owned()),
        ("select".to_owned(), "Space".to_owned()),
        ("a".to_owned(), "X".to_owned()),
        ("b".to_owned(), "Z".to_owned()),
        ("x".to_owned(), "S".to_owned()),
        ("y".to_owned(), "A".to_owned()),
        ("l".to_owned(), "Q".to_owned()),
        ("r".to_owned(), "W".to_owned()),
    ])
}

fn default_gamepad_bindings() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("up".to_owned(), "DPadUp".to_owned()),
        ("down".to_owned(), "DPadDown".to_owned()),
        ("left".to_owned(), "DPadLeft".to_owned()),
        ("right".to_owned(), "DPadRight".to_owned()),
        ("start".to_owned(), "Start".to_owned()),
        ("select".to_owned(), "Select".to_owned()),
        ("a".to_owned(), "East".to_owned()),
        ("b".to_owned(), "South".to_owned()),
        ("x".to_owned(), "North".to_owned()),
        ("y".to_owned(), "West".to_owned()),
        ("l".to_owned(), "LeftTrigger".to_owned()),
        ("r".to_owned(), "RightTrigger".to_owned()),
    ])
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{AppMode, AssetConfig, LibraryViewMode, RuntimeConfig};

    #[test]
    fn asset_config_resolves_default_paths() {
        let config = AssetConfig::default();
        assert_eq!(config.cache_root(), std::path::PathBuf::from(".cache").join("starbyte"));
        assert_eq!(config.config_path(), std::path::PathBuf::from(".config").join("starbyte").join("config.toml"));
        assert_eq!(config.legacy_config_path(), std::path::PathBuf::from(".cache").join("starbyte").join("config.toml"));
    }

    #[test]
    fn runtime_config_roundtrips_toml() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("starbyte.toml");
        let mut config = RuntimeConfig::default();
        config.library.active_view = LibraryViewMode::Detailed;
        config.library.rom_dirs.push(temp_dir.path().join("roms"));
        config.mode = AppMode::Prod;
        config.ui.show_log_panel = false;
        config.ui.details_panel_width = 512.0;
        config.cheats
            .enabled_by_game
            .insert("abc123".to_owned(), vec!["infinite-lives".to_owned()]);
        config.save_to_path(&path).unwrap();

        let loaded = RuntimeConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.library.active_view, LibraryViewMode::Detailed);
        assert_eq!(loaded.library.rom_dirs.len(), 1);
        assert_eq!(loaded.mode, AppMode::Prod);
        assert!(!loaded.ui.show_log_panel);
        assert_eq!(loaded.ui.details_panel_width, 512.0);
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

    #[test]
    fn legacy_config_without_mode_loads_with_defaults() {
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("legacy.toml");
        std::fs::write(
            &path,
            r#"
log_filter = "info"
prefer_dark_mode = true

[library]
rom_dirs = []
active_view = "grid"
show_installed_only = false
"#,
        )
        .unwrap();

        let loaded = RuntimeConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.mode, AppMode::Dev);
        assert!(loaded.ui.show_log_panel);
    }
}
