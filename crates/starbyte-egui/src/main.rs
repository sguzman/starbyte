//! Native `egui` frontend bootstrap for Starbyte.

mod app;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use tracing_subscriber::{EnvFilter, fmt};

use crate::app::StarbyteApp;

use starbyte_core::manifest::AssetConfig;

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

    /// Start in light mode instead of night mode.
    #[arg(long)]
    day_mode: bool,
}

fn main() -> Result<()> {
    install_tracing()?;
    let args = Args::parse();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Starbyte"),
        ..Default::default()
    };

    let assets = AssetConfig {
        spc700_ipl: args.spc700_ipl.clone(),
        save_dir: None,
        state_dir: None,
        cache_dir: args.cache_dir.clone(),
        config_path: args.config.clone(),
    };
    let prefer_dark_mode = !args.day_mode;
    let rom = args.rom.clone();
    let rom_dirs = args.rom_dirs.clone();

    eframe::run_native(
        "Starbyte",
        options,
        Box::new(move |cc| {
            let app = StarbyteApp::new(
                cc,
                assets.clone(),
                rom.clone(),
                rom_dirs.clone(),
                prefer_dark_mode,
            )
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("failed to launch egui frontend: {error}"))
}

fn install_tracing() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))
}
