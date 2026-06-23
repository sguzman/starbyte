//! Native `egui` frontend bootstrap for Starbyte.

mod app;
mod session;

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

    /// Optional SPC700 IPL ROM path.
    #[arg(long)]
    spc700_ipl: Option<PathBuf>,

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
    };
    let prefer_dark_mode = !args.day_mode;
    let rom = args.rom.clone();

    eframe::run_native(
        "Starbyte",
        options,
        Box::new(move |cc| {
            let app = StarbyteApp::new(cc, assets.clone(), rom.clone(), prefer_dark_mode)
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
