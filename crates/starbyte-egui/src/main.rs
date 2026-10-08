//! Native Wayland-first `egui` desktop frontend for Starbyte.

mod app;
mod logging;
mod worker;

use std::{env, path::PathBuf};

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use tracing::info;

use crate::{app::StarbyteApp, logging::install_tracing};

use starbyte_core::manifest::{AssetConfig, RuntimeConfig};

#[derive(Debug, Parser)]
#[command(
    name = "starbyte-egui",
    about = "Rust-native SNES library and desktop frontend"
)]
struct Args {
    /// Optional ROM to load at startup.
    #[arg(long, conflicts_with = "demo")]
    rom: Option<PathBuf>,

    /// Start a built-in copyright-free graphics demo without a game ROM.
    #[arg(long)]
    demo: bool,

    /// Optional library ROM directory to add on startup. May be provided multiple times.
    #[arg(long = "rom-dir")]
    rom_dirs: Vec<PathBuf>,

    /// Optional SPC700 IPL ROM path.
    #[arg(long)]
    spc700_ipl: Option<PathBuf>,

    /// Optional cache root for metadata, covers, cheats, and config.
    #[arg(long)]
    cache_dir: Option<PathBuf>,

    /// Optional runtime configuration file path.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Start in light mode instead of using the persisted theme preference.
    #[arg(long)]
    day_mode: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let assets = AssetConfig {
        spc700_ipl: args.spc700_ipl.clone(),
        save_dir: None,
        state_dir: None,
        cache_dir: args.cache_dir.clone(),
        config_path: args.config.clone(),
    };
    let config_path = assets.config_path();
    let mut config = load_runtime_config(&assets)?;
    // Adopt an existing, conventional per-user SNES directory as a removable
    // library source. Persisted by StarbyteApp alongside user-added folders.
    if let Some(home) = env::var_os("HOME") {
        discover_snes_library(&mut config, &PathBuf::from(home));
    }
    let cache_root = config
        .library
        .cache_dir
        .clone()
        .or_else(|| assets.cache_dir.clone())
        .unwrap_or_else(|| assets.cache_root());
    let filter = env::var("STARBYTE_LOG").unwrap_or_else(|_| config.log_filter.clone());
    let log_lines = install_tracing(&cache_root, &filter, config.mode)?;
    info!(
        config_path = %config_path.display(),
        cache_root = %cache_root.display(),
        mode = ?config.mode,
        filter = %filter,
        "starting starbyte egui"
    );

    let prefer_dark_mode_override = args.day_mode.then_some(false);
    if let Some(prefer_dark_mode) = prefer_dark_mode_override {
        config.prefer_dark_mode = prefer_dark_mode;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([520.0, 360.0])
            .with_title("Starbyte")
            .with_fullscreen(config.video.fullscreen),
        ..Default::default()
    };

    let rom = if args.demo {
        let demo_dir = cache_root.join("built-in");
        std::fs::create_dir_all(&demo_dir)?;
        let path = demo_dir.join("starbyte-display-demo.sfc");
        std::fs::write(&path, starbyte_core::testing::demo::demo_rom_bytes())?;
        Some(path)
    } else {
        args.rom.clone()
    };
    let rom_dirs = args.rom_dirs.clone();

    eframe::run_native(
        "Starbyte",
        options,
        Box::new(move |cc| {
            let app = StarbyteApp::new(
                cc,
                assets.clone(),
                config.clone(),
                rom.clone(),
                rom_dirs.clone(),
                prefer_dark_mode_override,
                log_lines.clone(),
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("failed to launch egui frontend: {error}"))
}

fn load_runtime_config(assets: &AssetConfig) -> Result<RuntimeConfig> {
    let config_path = assets.config_path();
    if assets.config_path.is_some() || config_path.exists() {
        return RuntimeConfig::load_or_default(&config_path).map_err(anyhow::Error::from);
    }

    if assets.config_path.is_none() {
        let worktree_path = assets.legacy_worktree_config_path();
        if worktree_path.exists() {
            return RuntimeConfig::load_or_default(&worktree_path).map_err(anyhow::Error::from);
        }
    }

    let legacy_path = assets.legacy_config_path();
    if legacy_path.exists() {
        return RuntimeConfig::load_or_default(&legacy_path).map_err(anyhow::Error::from);
    }

    Ok(RuntimeConfig::default())
}

fn discover_snes_library(config: &mut RuntimeConfig, home: &std::path::Path) -> bool {
    if config.library.home_snes_discovery_complete {
        return false;
    }
    let path = home.join("Games").join("Roms").join("SNES");
    if !path.is_dir() {
        return false;
    }
    config.library.home_snes_discovery_complete = true;
    if config.library.rom_dirs.contains(&path) {
        return false;
    }
    config.library.rom_dirs.push(path);
    true
}

#[cfg(test)]
mod library_discovery_tests {
    use super::{RuntimeConfig, discover_snes_library};

    #[test]
    fn existing_home_collection_is_added_once_and_persists_in_config() {
        let home = tempfile::tempdir().unwrap();
        let snes = home.path().join("Games/Roms/SNES");
        std::fs::create_dir_all(&snes).unwrap();
        let mut config = RuntimeConfig::default();
        config
            .library
            .rom_dirs
            .push(home.path().join("old-library"));
        assert!(discover_snes_library(&mut config, home.path()));
        assert!(!discover_snes_library(&mut config, home.path()));
        assert_eq!(config.library.rom_dirs.len(), 2);
        assert_eq!(config.library.rom_dirs[1], snes);
        let path = home.path().join("settings.toml");
        config.save_to_path(&path).unwrap();
        let loaded = RuntimeConfig::load_or_default(path).unwrap();
        assert_eq!(loaded.library.rom_dirs, config.library.rom_dirs);
        assert!(loaded.library.home_snes_discovery_complete);
        // Removing the discovered folder is an intentional user choice.
        config.library.rom_dirs.retain(|folder| folder != &snes);
        assert!(!discover_snes_library(&mut config, home.path()));
        assert!(!config.library.rom_dirs.contains(&snes));
    }

    #[test]
    fn nonexistent_home_collection_is_not_created() {
        let home = tempfile::tempdir().unwrap();
        let mut config = RuntimeConfig::default();
        assert!(!discover_snes_library(&mut config, home.path()));
        assert!(config.library.rom_dirs.is_empty());
        assert!(!home.path().join("Games").exists());
    }
}
