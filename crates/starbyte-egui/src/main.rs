//! Native Wayland-first `egui` desktop frontend for Starbyte.

mod app;
mod audio;
mod blackout;
mod logging;
mod pitch;
#[allow(dead_code)]
mod preview_gamepads;
// Worker-clock foundation; GUI session handoff is a separate gated milestone.
#[allow(dead_code)]
mod scheduler;
// Session/audio/save ownership bridge; keep current frontend on the proven
// clock until the UI handoff is complete.
#[allow(dead_code)]
mod scheduler_video;
#[allow(dead_code)]
mod session_engine;
mod threaded_preview;
mod video_upload;
mod worker;

use std::{env, path::PathBuf};

use anyhow::Result;
use clap::Parser;
use eframe::egui;
use tracing::info;

use crate::{app::StarbyteApp, logging::install_tracing};

use starbyte_core::manifest::{AssetConfig, RuntimeConfig, UiTheme};

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

    /// Play a short, locally synthesized 440 Hz tone through the default
    /// output device and exit; requires no ROM or firmware.
    #[arg(long, conflicts_with_all = ["rom", "demo"])]
    speaker_test: bool,

    /// EXPERIMENTAL: execute the uploaded SPC700 driver for this one startup
    /// ROM. May stall; existing SRAM and durable save slots are isolated.
    #[arg(long, requires = "rom", conflicts_with_all = ["demo", "speaker_test"])]
    experimental_sound_driver: bool,

    /// CI/diagnostic only: close the native GUI cleanly once N emulated
    /// frames have completed. Requires a startup ROM or built-in demo.
    #[arg(long, conflicts_with = "speaker_test", value_parser = clap::value_parser!(u64).range(1..))]
    exit_after_frames: Option<u64>,

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

    /// Headless developer smoke: prove that the independent SNES clock runs
    /// without Wayland repaint callbacks and obeys background pause policy.
    #[arg(long, requires = "demo", conflicts_with = "exit_after_frames")]
    scheduler_smoke: bool,

    /// Opt-in native preview of the worker-owned emulation engine.
    /// Does not replace the normal, fully featured library UI yet.
    #[arg(long, conflicts_with = "scheduler_smoke")]
    threaded_preview: bool,

    /// Threaded preview only: keep running without compositor keyboard focus.
    /// Useful for headless Weston acceptance; does not change saved settings.
    #[arg(long, requires = "threaded_preview")]
    preview_background: bool,

    /// CI only: close the empty threaded library after indexing at least
    /// one installed game. Verifies the real native Wayland library path.
    #[arg(
        long,
        requires = "threaded_preview",
        conflicts_with_all = ["rom", "demo", "exit_after_frames", "speaker_test"]
    )]
    exit_after_library_scan: bool,

    /// CI only: index the local library, launch its first installed game
    /// through the emulation worker, render 30 frames and exit.
    #[arg(
        long,
        requires = "threaded_preview",
        conflicts_with_all = ["rom", "demo", "exit_after_frames", "exit_after_library_scan", "speaker_test"]
    )]
    exit_after_library_play: bool,

    /// CI: stop servicing egui for two seconds while the worker runs.
    #[arg(
        long,
        requires_all = ["demo", "threaded_preview", "preview_background"],
        conflicts_with_all = ["exit_after_frames", "exit_after_library_scan", "exit_after_library_play"]
    )]
    exit_after_repaint_gap: bool,

    /// Internal integration preview: preserve the complete original
    /// library window but use the independent worker for SNES execution.
    /// Deliberately hidden until all native controls are validated.
    #[arg(long, hide = true, conflicts_with_all = ["threaded_preview", "scheduler_smoke", "speaker_test"])]
    native_worker_preview: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    validate_frame_exit_args(&args)?;
    if args.speaker_test {
        crate::audio::AudioOutput::test_speakers()?;
        println!("Speaker probe finished (440 Hz stereo test tone).");
        return Ok(());
    }
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
        config.ui.theme = if prefer_dark_mode {
            UiTheme::Dark
        } else {
            UiTheme::Light
        };
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
    // The existing library receives these directories through StarbyteApp,
    // but the independent preview needs them before its own worker scan.
    for path in &rom_dirs {
        if !config.library.rom_dirs.contains(path) {
            config.library.rom_dirs.push(path.clone());
        }
    }
    if args.scheduler_smoke {
        return run_scheduler_smoke(assets, rom.expect("--demo generates a built-in ROM"));
    }
    if args.threaded_preview {
        return eframe::run_native(
            "Starbyte (threaded preview)",
            options,
            Box::new(move |cc| {
                // Honor the same persisted System / Light / Dark choice as the
                // established library, even in the opt-in worker preview.
                cc.egui_ctx.set_theme(match config.ui.theme {
                    UiTheme::System => egui::ThemePreference::System,
                    UiTheme::Light => egui::ThemePreference::Light,
                    UiTheme::Dark => egui::ThemePreference::Dark,
                });
                let mut app = crate::threaded_preview::ThreadedPreviewApp::new(
                    assets.clone(),
                    &config,
                    rom.clone(),
                    args.exit_after_frames
                        .or(args.exit_after_library_play.then_some(30)),
                    args.preview_background,
                    args.exit_after_library_scan,
                    args.exit_after_library_play,
                )
                .map_err(|error| std::io::Error::other(error.to_string()))?;
                if args.exit_after_repaint_gap {
                    app.arm_repaint_gap_smoke();
                }
                Ok(Box::new(app))
            }),
        )
        .map_err(|error| anyhow::anyhow!("threaded preview failed: {error}"));
    }

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
                args.experimental_sound_driver,
                prefer_dark_mode_override,
                args.exit_after_frames,
                args.native_worker_preview,
                log_lines.clone(),
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("failed to launch egui frontend: {error}"))
}

/// Drive an actual ROM through the worker without constructing any window.
/// Focus and pause acknowledgements travel on the same serialized queue as
/// frames; a graphics compositor cannot influence this probe.
fn run_scheduler_smoke(assets: AssetConfig, rom: PathBuf) -> Result<()> {
    use crate::session_engine::OwnedSession;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let mut engine = OwnedSession::create(assets)?;
    engine.load_rom(&rom, starbyte_frontend::SoundSessionMode::Persistent)?;
    engine.set_audio(false, true, 0.0);

    let worker = engine.spawn()?;
    worker.set_pause_when_unfocused(false);
    worker.set_focused(false);
    worker.set_playing(true);

    let started = Instant::now();
    while worker.completed_frames() < 5 && started.elapsed() < Duration::from_secs(6) {
        thread::sleep(Duration::from_millis(15));
    }
    let running = worker.completed_frames();
    anyhow::ensure!(
        running >= 5,
        "independent worker failed to produce 5 background frames: {running}"
    );

    worker.set_pause_when_unfocused(true);
    worker.invoke(|_| Ok(())).map_err(anyhow::Error::msg)?;
    let stopped_at = worker.completed_frames();
    thread::sleep(Duration::from_millis(120));
    anyhow::ensure!(
        worker.completed_frames() == stopped_at,
        "background pause did not stop emulation after command acknowledgement"
    );

    worker.set_focused(true);
    worker.invoke(|_| Ok(())).map_err(anyhow::Error::msg)?;
    let resumed_at = worker.completed_frames();
    let resumed = Instant::now();
    while worker.completed_frames() <= resumed_at && resumed.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(15));
    }
    anyhow::ensure!(
        worker.completed_frames() > resumed_at,
        "worker did not resume after window focus returned"
    );

    // A separate saved-state round trip checks that commands serialize
    // against the independent clock. This uses only a memory quick slot in
    // the synthetic demo ROM; it never touches a real game's disk slots.
    worker.set_playing(false);
    worker.invoke(|_| Ok(())).map_err(anyhow::Error::msg)?;
    let saved = worker.read_status().map_err(anyhow::Error::msg)?.frame;
    worker.quick_save_game().map_err(anyhow::Error::msg)?;
    worker.set_playing(true);
    let replay_start = worker.completed_frames();
    let deadline = Instant::now() + Duration::from_secs(3);
    while worker.completed_frames() <= replay_start + 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(15));
    }
    anyhow::ensure!(
        worker.completed_frames() > replay_start + 2,
        "worker did not advance after memory quick-save"
    );
    worker.set_playing(false);
    worker.invoke(|_| Ok(())).map_err(anyhow::Error::msg)?;
    let restored = worker.quick_load_with_frame().map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        restored.status.frame == saved,
        "quick-load restored frame {} instead of saved frame {saved}",
        restored.status.frame
    );
    // A bad replacement ROM must not eject the currently loaded game or
    // alter its save namespace. Check both failure acknowledgment and state.
    let before_failed_load = worker.read_status().map_err(anyhow::Error::msg)?;
    let invalid_rom = rom.with_extension("intentionally-missing-for-smoke.sfc");
    anyhow::ensure!(
        !invalid_rom.exists(),
        "scheduler test fixture unexpectedly exists: {}",
        invalid_rom.display()
    );
    let failed_load = worker
        .queue_load_game_with_frame(invalid_rom, starbyte_frontend::SoundSessionMode::Persistent)
        .map_err(anyhow::Error::msg)?;
    let result = failed_load
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| anyhow::anyhow!("missing ROM load response: {error}"))?;
    anyhow::ensure!(result.is_err(), "invalid cartridge unexpectedly loaded");
    let after_failed_load = worker.read_status().map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        after_failed_load.has_rom
            && after_failed_load.frame == before_failed_load.frame
            && after_failed_load.rom_path == before_failed_load.rom_path,
        "failed ROM load changed the active cartridge or its frame"
    );

    let rgba = restored
        .rgba
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("restored still image was not published"))?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let screenshot = std::env::temp_dir().join(format!(
        "starbyte-scheduler-smoke-{}-{timestamp}.png",
        std::process::id()
    ));
    crate::app::write_png_screenshot(
        &screenshot,
        rgba,
        restored.status.framebuffer_width,
        restored.status.framebuffer_height,
    )?;
    let decoded = image::open(&screenshot)?;
    anyhow::ensure!(
        decoded.width() == restored.status.framebuffer_width
            && decoded.height() == restored.status.framebuffer_height,
        "worker screenshot dimensions did not survive PNG round-trip"
    );
    std::fs::remove_file(&screenshot)?;

    let engine = worker
        .shutdown()
        .map_err(|_| anyhow::anyhow!("emulation scheduler worker panicked"))?;
    anyhow::ensure!(engine.snapshot().has_rom, "worker lost ROM ownership");
    println!(
        concat!(
            "STARBYTE_SCHEDULER_SMOKE_OK background_frames={} ",
            "paused_at={} total_frames={} restored_frame={}"
        ),
        running,
        stopped_at,
        engine.snapshot().frame,
        restored.status.frame
    );
    Ok(())
}

fn validate_frame_exit_args(args: &Args) -> Result<()> {
    anyhow::ensure!(
        args.exit_after_frames.is_none() || args.demo || args.rom.is_some(),
        "--exit-after-frames requires --demo or --rom"
    );
    Ok(())
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
mod speaker_cli_tests {
    use clap::Parser;

    use super::{Args, validate_frame_exit_args};

    #[test]
    fn no_rom_speaker_probe_requires_explicit_opt_in() {
        let probe = Args::try_parse_from(["starbyte-egui", "--speaker-test"]).unwrap();
        assert!(probe.speaker_test);
        assert!(probe.rom.is_none());
        assert!(!probe.demo);

        let normal = Args::try_parse_from(["starbyte-egui"]).unwrap();
        assert!(!normal.speaker_test);
        assert!(!normal.native_worker_preview);
        let integration = Args::try_parse_from([
            "starbyte-egui", "--demo", "--native-worker-preview",
            "--exit-after-frames", "30"
        ]).unwrap();
        assert!(validate_frame_exit_args(&integration).is_ok());
    }

    #[test]
    fn experimental_sound_driver_requires_an_explicit_rom_and_never_defaults_on() {
        let normal = Args::try_parse_from(["starbyte-egui", "--rom", "fixture.sfc"]).unwrap();
        assert!(!normal.experimental_sound_driver);
        let probe = Args::try_parse_from([
            "starbyte-egui",
            "--rom",
            "fixture.sfc",
            "--experimental-sound-driver",
        ])
        .unwrap();
        assert!(probe.experimental_sound_driver);
        assert!(Args::try_parse_from(["starbyte-egui", "--experimental-sound-driver",]).is_err());
        assert!(
            Args::try_parse_from(["starbyte-egui", "--demo", "--experimental-sound-driver",])
                .is_err()
        );
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--speaker-test",
                "--experimental-sound-driver",
            ])
            .is_err()
        );
    }

    #[test]
    fn bounded_native_smoke_requires_a_startup_game_or_demo() {
        let normal = Args::try_parse_from(["starbyte-egui"]).unwrap();
        assert_eq!(normal.exit_after_frames, None);
        assert!(validate_frame_exit_args(&normal).is_ok());
        let missing = Args::try_parse_from(["starbyte-egui", "--exit-after-frames", "30"]).unwrap();
        assert!(validate_frame_exit_args(&missing).is_err());
        for flags in [
            vec!["starbyte-egui", "--demo", "--exit-after-frames", "30"],
            vec![
                "starbyte-egui",
                "--rom",
                "fixture.sfc",
                "--exit-after-frames",
                "500",
            ],
            vec![
                "starbyte-egui",
                "--rom",
                "fixture.sfc",
                "--experimental-sound-driver",
                "--exit-after-frames",
                "500",
            ],
        ] {
            let args = Args::try_parse_from(flags).unwrap();
            assert!(validate_frame_exit_args(&args).is_ok());
            assert!(args.exit_after_frames.unwrap() > 0);
        }
        assert!(
            Args::try_parse_from(["starbyte-egui", "--demo", "--exit-after-frames", "0"]).is_err()
        );
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--speaker-test",
                "--exit-after-frames",
                "5"
            ])
            .is_err()
        );
    }

    #[test]
    fn threaded_preview_can_open_a_library_without_a_startup_game() {
        let missing = Args::try_parse_from(["starbyte-egui", "--threaded-preview"]).unwrap();
        assert!(validate_frame_exit_args(&missing).is_ok());
        let bounded = Args::try_parse_from([
            "starbyte-egui",
            "--threaded-preview",
            "--exit-after-library-scan",
        ])
        .unwrap();
        assert!(validate_frame_exit_args(&bounded).is_ok());
        assert!(bounded.exit_after_library_scan);
        assert!(Args::try_parse_from(["starbyte-egui", "--exit-after-library-scan"]).is_err());
        let launch = Args::try_parse_from([
            "starbyte-egui",
            "--threaded-preview",
            "--exit-after-library-play",
        ])
        .unwrap();
        assert!(validate_frame_exit_args(&launch).is_ok());
        assert!(launch.exit_after_library_play);
        assert!(Args::try_parse_from(["starbyte-egui", "--exit-after-library-play"]).is_err());
        let without_redraws = Args::try_parse_from([
            "starbyte-egui",
            "--threaded-preview",
            "--preview-background",
            "--demo",
            "--exit-after-repaint-gap",
        ])
        .unwrap();
        assert!(validate_frame_exit_args(&without_redraws).is_ok());
        assert!(without_redraws.exit_after_repaint_gap);
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--threaded-preview",
                "--demo",
                "--exit-after-repaint-gap"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--threaded-preview",
                "--demo",
                "--exit-after-library-play"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--threaded-preview",
                "--demo",
                "--exit-after-library-scan",
            ])
            .is_err()
        );
        let demo = Args::try_parse_from([
            "starbyte-egui",
            "--demo",
            "--threaded-preview",
            "--exit-after-frames",
            "30",
        ])
        .unwrap();
        assert!(validate_frame_exit_args(&demo).is_ok());
        assert!(
            Args::try_parse_from([
                "starbyte-egui",
                "--demo",
                "--threaded-preview",
                "--scheduler-smoke",
            ])
            .is_err()
        );
    }

    #[test]
    fn speaker_probe_cannot_be_combined_with_game_or_demo_mode() {
        assert!(Args::try_parse_from(["starbyte-egui", "--speaker-test", "--demo"]).is_err());
        assert!(
            Args::try_parse_from(["starbyte-egui", "--speaker-test", "--rom", "game.sfc"]).is_err()
        );
    }
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
