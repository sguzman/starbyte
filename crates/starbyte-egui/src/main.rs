//! Native `egui` frontend bootstrap for Starbyte.

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
#[command(name = "starbyte-egui", about = "Bootstrap egui frontend for Starbyte")]
struct Args {
    /// Optional ROM to load at startup.
    #[arg(long)]
    rom: Option<PathBuf>,

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
            .with_min_inner_size([1100.0, 720.0])
            .with_title("Starbyte"),
        ..Default::default()
    };

    let rom = args.rom.clone();
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
        return RuntimeConfig::load_or_default(&config_path)
            .map_err(anyhow::Error::from);
    }

    let legacy_path = assets.legacy_config_path();
    if legacy_path.exists() {
        return RuntimeConfig::load_or_default(&legacy_path)
            .map_err(anyhow::Error::from);
    }

    Ok(RuntimeConfig::default())
}
