//! Starbyte CLI bootstrap entrypoint.

mod cheatarium;

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use starbyte_frontend::{LibraryFilter, LibraryService, LibraryTarget};
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt};

use starbyte_core::{
    EmulatorBuilder,
    cartridge::Cartridge,
    input::ControllerState,
    manifest::{AssetConfig, RuntimeConfig},
    testing,
};

#[derive(Debug, Parser)]
#[command(
    name = "starbyte",
    about = "Headless inspection, diagnostics and automation for the Starbyte SNES emulator"
)]
struct Cli {
    #[command(flatten)]
    logging: LoggingArgs,

    #[command(flatten)]
    assets: AssetArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Args)]
struct LoggingArgs {
    /// Additional log verbosity. May be provided multiple times.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Explicit tracing filter. Overrides STARBYTE_LOG and verbosity.
    #[arg(long, env = "STARBYTE_LOG", global = true)]
    log_filter: Option<String>,
}

#[derive(Debug, Args)]
struct AssetArgs {
    /// Path to a user-supplied SPC700 IPL ROM.
    #[arg(long, global = true)]
    spc700_ipl: Option<PathBuf>,

    /// Directory for battery-backed saves.
    #[arg(long, global = true)]
    save_dir: Option<PathBuf>,

    /// Directory for save-state files.
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,

    /// Directory for cached metadata, covers, and cheats.
    #[arg(long, global = true)]
    cache_dir: Option<PathBuf>,

    /// Runtime configuration file path.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect or validate external compliance corpora.
    Compliance(ComplianceArgs),
    /// Search a local Cheatarium SNES index without activating any codes.
    Cheatarium {
        /// Explicit path to a local Cheatarium snes.json.gz bundle.
        #[arg(long)]
        index: PathBuf,
        /// Candidate game title; not a verified cartridge match.
        #[arg(long)]
        title: String,
        /// Emit versioned machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Maximum candidate source records to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Inspect ROM metadata without running emulation.
    Inspect {
        /// Local ROM to inspect.
        rom: PathBuf,
        /// Emit a versioned JSON report instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Scan ROM libraries and manage cached metadata, covers, and cheats.
    Library(LibraryArgs),
    /// Run the bootstrap emulator for a fixed number of frames.
    Run(RunArgs),
    /// Emit a sample runtime configuration file to stdout.
    PrintConfig { format: ConfigFormat },
    /// Describe supported local automation commands as versioned JSON.
    Capabilities,
    /// Inspect local platform and user-directory configuration without loading a ROM.
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
struct LibraryArgs {
    #[command(subcommand)]
    command: LibraryCommand,
}

#[derive(Debug, Subcommand)]
enum LibraryCommand {
    /// Scan configured ROM directories and print the merged library snapshot.
    Scan(LibraryScanArgs),
    /// Refresh and cache library metadata.
    #[command(alias = "fetch-metadata")]
    RefreshMetadata(LibraryTargetArgs),
    /// Refresh and cache cover images.
    #[command(alias = "fetch-covers")]
    RefreshCovers(LibraryTargetArgs),
    /// Refresh and cache cheats.
    #[command(alias = "fetch-cheats")]
    RefreshCheats(LibraryTargetArgs),
    /// Refresh metadata, covers, and cheats together.
    #[command(alias = "fetch-all")]
    RefreshAll(LibraryTargetArgs),
    /// Placeholder hook for future ROM downloads.
    DownloadRom(LibraryTargetArgs),
}

#[derive(Debug, Args, Clone, Default)]
struct LibraryTargetArgs {
    /// Restrict refresh actions to installed entries only.
    #[arg(long)]
    installed_only: bool,

    /// Restrict refresh actions to one stable game id.
    #[arg(long)]
    game_id: Option<String>,

    /// Restrict refresh actions to entries with matching titles.
    #[arg(long)]
    title: Option<String>,

    /// Restrict refresh actions to one ROM path.
    #[arg(long)]
    rom: Option<PathBuf>,

    /// Override configured ROM directories for this command. May be provided multiple times.
    #[arg(long = "rom-dir")]
    rom_dirs: Vec<PathBuf>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args, Clone, Default)]
struct LibraryScanArgs {
    #[command(flatten)]
    target: LibraryTargetArgs,

    /// Free-text query applied to the merged library snapshot.
    #[arg(long)]
    query: Option<String>,
}

#[derive(Debug, Args)]
struct DoctorArgs {
    /// Emit machine-readable JSON diagnostics.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// ROM image to load.
    rom: PathBuf,

    /// Number of placeholder frames to execute before exiting.
    #[arg(long, default_value_t = 1)]
    frames: u32,

    /// Restore emulator state from this file before running frames.
    #[arg(long)]
    load_state: Option<PathBuf>,

    /// Serialize emulator state to this file before exit.
    #[arg(long)]
    save_state: Option<PathBuf>,

    /// Dump the final framebuffer to a PPM file.
    #[arg(long)]
    screenshot: Option<PathBuf>,

    /// Write a structured JSON run report for automation.
    #[arg(long)]
    report_json: Option<PathBuf>,

    /// Comma-separated controller-1 buttons to hold during the run.
    #[arg(long)]
    controller1: Option<String>,

    /// One-based frame events: "350:start;353:none;440:right,b". Buttons
    /// remain held until the next event; "none" releases all buttons.
    #[arg(long)]
    controller1_events: Option<String>,

    /// Write one flushed JSON object per attempted frame, including failures.
    #[arg(long)]
    frame_log: Option<PathBuf>,

    /// Optionally save bounded PPM snapshots of completed frames to this directory.
    #[arg(long)]
    frame_images_dir: Option<PathBuf>,

    /// Snapshot the first frame and then every Nth completed frame.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    frame_image_every: u32,

    /// Maximum frame snapshots per run to avoid excessive disk use.
    #[arg(long, default_value_t = 24, value_parser = clap::value_parser!(u32).range(1..))]
    max_frame_images: u32,

    /// Do not load or write cartridge SRAM; useful for reproducible diagnosis.
    #[arg(long)]
    no_save_ram: bool,

    /// One-based frame to record every CPU instruction and its bus events.
    #[arg(long, requires = "trace_out", value_parser = clap::value_parser!(u32).range(1..))]
    trace_frame: Option<u32>,

    /// Output JSONL path for the selected instruction-trace frame.
    #[arg(long, requires = "trace_frame")]
    trace_out: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ComplianceArgs {
    #[command(subcommand)]
    command: ComplianceCommand,
}

#[derive(Debug, Args)]
struct CommercialRecordArgs {
    /// Commercial ROM to record from.
    rom: PathBuf,

    /// Number of frames to execute before recording evidence.
    #[arg(long, default_value_t = 300)]
    frames: u32,

    /// Output path for the generated fixture JSON.
    #[arg(long)]
    fixture_out: PathBuf,

    /// Comma-separated controller-1 buttons to hold during recording.
    #[arg(long)]
    controller1: Option<String>,

    /// Optional instruction-trace output path.
    #[arg(long)]
    trace_out: Option<PathBuf>,

    /// Skip this many initial completed frames before instruction tracing.
    /// A 360-frame run with --trace-from-frame 359 traces only the final frame.
    #[arg(long, requires = "trace_out")]
    trace_from_frame: Option<u32>,
}

#[derive(Debug, Subcommand)]
enum ComplianceCommand {
    /// Count files and vectors in a compliance suite directory.
    Summary {
        #[arg(value_enum)]
        suite: ComplianceSuite,
        dir: PathBuf,
    },
    /// Parse a single opcode file and report whether the format is accepted.
    VerifyFormat {
        #[arg(value_enum)]
        suite: ComplianceSuite,
        dir: PathBuf,
        #[arg(long)]
        opcode: String,
        #[arg(long)]
        mode: Option<Cpu65816ModeArg>,
    },
    /// Execute one opcode file against the current in-tree core implementation.
    RunCurrent {
        #[arg(value_enum)]
        suite: ComplianceSuite,
        dir: PathBuf,
        #[arg(long)]
        opcode: String,
        #[arg(long)]
        mode: Option<Cpu65816ModeArg>,
        #[arg(long, default_value_t = 8)]
        max_failures: usize,
    },
    /// Count files and fixtures in a ROM-based regression suite directory.
    RomSummary { dir: PathBuf },
    /// Execute ROM-based regression fixtures against the current emulator.
    RomRunCurrent {
        dir: PathBuf,
        #[arg(long, default_value_t = 8)]
        max_failures: usize,
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
    },
    /// Count files and fixtures in a commercial-ROM suite directory.
    CommercialSummary { dir: PathBuf },
    /// Execute commercial-ROM fixtures against the current emulator.
    CommercialRunCurrent {
        dir: PathBuf,
        #[arg(long, default_value_t = 8)]
        max_failures: usize,
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
        #[arg(long)]
        trace_out: Option<PathBuf>,
    },
    /// Record a local commercial-ROM fixture from current emulator evidence.
    CommercialRecord(CommercialRecordArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ComplianceSuite {
    #[value(name = "cpu-65816", alias = "cpu65816")]
    Cpu65816,
    #[value(name = "spc700")]
    Spc700,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Cpu65816ModeArg {
    #[value(name = "emulation")]
    Emulation,
    #[value(name = "native")]
    Native,
}

impl From<Cpu65816ModeArg> for testing::cpu_65816::Mode {
    fn from(value: Cpu65816ModeArg) -> Self {
        match value {
            Cpu65816ModeArg::Emulation => Self::Emulation,
            Cpu65816ModeArg::Native => Self::Native,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ConfigFormat {
    Toml,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    install_tracing(&cli.logging)?;

    let assets = AssetConfig {
        spc700_ipl: cli.assets.spc700_ipl.clone(),
        save_dir: cli.assets.save_dir.clone(),
        state_dir: cli.assets.state_dir.clone(),
        cache_dir: cli.assets.cache_dir.clone(),
        config_path: cli.assets.config.clone(),
    };

    match cli.command {
        Command::Compliance(args) => run_compliance(args, assets),
        Command::Inspect { rom, json } => inspect_rom(rom, json),
        Command::Cheatarium {
            index,
            title,
            json,
            limit,
        } => cheatarium::search(&index, &title, limit, json),
        Command::Library(args) => run_library(args, assets),
        Command::Run(args) => run_rom(args, assets),
        Command::PrintConfig { format } => print_config(format),
        Command::Capabilities => {
            println!(
                "{}",
                serde_json::to_string_pretty(&capabilities_manifest())?
            );
            Ok(())
        }
        Command::Doctor(args) => print_doctor(&assets, args.json),
    }
}

/// Static, side-effect-free command discovery for future agent adapters.
fn capabilities_manifest() -> serde_json::Value {
    json!({
        "schema": "starbyte.capabilities.v1",
        "version": env!("CARGO_PKG_VERSION"),
        "mcp_server": false,
        "transport": "local_cli",
        "commands": [
            {
                "name": "capabilities",
                "argv": ["capabilities"],
                "side_effects": "none"
            },
            {
                "name": "doctor",
                "argv": ["doctor", "--json"],
                "side_effects": "read_local_environment"
            },
            {
                "name": "print_config",
                "argv": ["print-config", "json"],
                "side_effects": "none"
            },
            {
                "name": "cheatarium_candidate_search",
                "argv": ["cheatarium", "--index", "<local_snes_index_json_gz>", "--title", "<game_title>", "--json"],
                "side_effects": "read_explicit_local_index_only",
                "game_identity": "filename_candidate_not_rom_verified",
                "activates_cheats": false
            },
            {
                "name": "rom_inspect",
                "argv": ["inspect", "<user_rom_path>", "--json"],
                "side_effects": "read_local_rom"
            },
            {
                "name": "library_scan",
                "argv": ["library", "scan", "--json"],
                "side_effects": "read_rom_directories_and_write_local_cache"
            },
            {
                "name": "run_probe",
                "argv": ["run", "<user_rom_path>", "--frames", "<count>", "--report-json", "<report_path>"],
                "optional_frame_log": "--frame-log <explicit_jsonl_path>",
                "optional_frame_images": "--frame-images-dir <explicit_directory> [--frame-image-every N] [--max-frame-images N]",
                "optional_controller1_events": "--controller1-events <frame:buttons;frame:none>",
                "optional_no_save_ram": "--no-save-ram",
                "side_effects": "execute_local_rom_and_write_explicit_report"
            }
        ],
        "policy": {
            "user_supplies_roms": true,
            "implicit_network_upload": false,
            "arbitrary_shell_execution": false,
            "unrestricted_file_access": false
        }
    })
}

fn doctor_report(assets: &AssetConfig) -> serde_json::Value {
    json!({
        "schema": "starbyte.doctor.v1",
        "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "session_type": std::env::var("XDG_SESSION_TYPE").ok(),
        "wayland_display_set": std::env::var_os("WAYLAND_DISPLAY").is_some(),
        "x11_display_set": std::env::var_os("DISPLAY").is_some(),
        "config_path": assets.config_path().display().to_string(),
        "config_exists": assets.config_path().exists(),
        "cache_root": assets.cache_root().display().to_string(),
        "cache_exists": assets.cache_root().exists(),
        "native_wayland_verified": false,
        "audio_output_verified": false
    })
}

fn print_doctor(assets: &AssetConfig, json_output: bool) -> Result<()> {
    let report = doctor_report(assets);
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Starbyte environment: {} / {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        println!("Config: {}", assets.config_path().display());
        println!("Cache: {}", assets.cache_root().display());
        println!(
            "Wayland environment variable set: {}",
            std::env::var_os("WAYLAND_DISPLAY").is_some()
        );
        println!("This is an environment probe, not a compositor or game-compatibility test.");
    }
    Ok(())
}

fn load_runtime_config(assets: &AssetConfig) -> Result<RuntimeConfig> {
    let config_path = assets.config_path();
    if assets.config_path.is_some() || config_path.exists() {
        return RuntimeConfig::load_or_default(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))
            .map_err(anyhow::Error::from);
    }

    if assets.config_path.is_none() {
        let worktree_path = assets.legacy_worktree_config_path();
        if worktree_path.exists() {
            return RuntimeConfig::load_or_default(&worktree_path)
                .with_context(|| {
                    format!(
                        "failed to load legacy config from {}",
                        worktree_path.display()
                    )
                })
                .map_err(anyhow::Error::from);
        }
    }

    let legacy_path = assets.legacy_config_path();
    if legacy_path.exists() {
        return RuntimeConfig::load_or_default(&legacy_path)
            .with_context(|| {
                format!(
                    "failed to load legacy config from {}",
                    legacy_path.display()
                )
            })
            .map_err(anyhow::Error::from);
    }

    Ok(RuntimeConfig::default())
}

fn install_tracing(logging: &LoggingArgs) -> Result<()> {
    let filter = logging
        .log_filter
        .clone()
        .unwrap_or_else(|| default_filter(logging.verbose));

    // Keep command output (especially JSON) parseable even when the core logs.
    fmt()
        .with_writer(std::io::stderr)
        .with_max_level(level_from_verbosity(logging.verbose))
        .with_env_filter(EnvFilter::new(filter))
        .with_target(true)
        .with_thread_ids(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))
}

fn default_filter(verbose: u8) -> String {
    match verbose {
        0 => "info,starbyte_core=debug,starbyte_cli=debug".to_owned(),
        1 => "debug".to_owned(),
        _ => "trace".to_owned(),
    }
}

const fn level_from_verbosity(verbose: u8) -> Level {
    match verbose {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    }
}

fn inspect_rom(path: PathBuf, json_output: bool) -> Result<()> {
    let cartridge = Cartridge::load(&path)
        .with_context(|| format!("failed to inspect ROM at {}", path.display()))?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema": "starbyte.rom_inspect.v1",
                "path": path.display().to_string(),
                "title": cartridge.header().title,
                "mapper": format!("{:?}", cartridge.mapper()),
                "coprocessor": cartridge.coprocessor_kind().map(|kind| kind.to_string()),
                "region": format!("{:?}", cartridge.header().region),
                "rom_size_declared_bytes": cartridge.header().rom_size_bytes(),
                "ram_size_declared_bytes": cartridge.header().ram_size_bytes(),
                "gameplay_compatibility_verified": false
            }))?
        );
        return Ok(());
    }

    println!("Title: {}", cartridge.header().title);
    println!("Mapper: {:?}", cartridge.mapper());
    println!(
        "Coprocessor: {}",
        cartridge
            .coprocessor_kind()
            .map(|kind| kind.to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
    println!("Region: {:?}", cartridge.header().region);
    println!(
        "ROM size (declared): {} bytes",
        cartridge.header().rom_size_bytes()
    );
    println!(
        "RAM size (declared): {} bytes",
        cartridge.header().ram_size_bytes()
    );
    Ok(())
}

fn run_library(args: LibraryArgs, assets: AssetConfig) -> Result<()> {
    let config_path = assets.config_path();
    info!(config_path = %config_path.display(), cache_root = %assets.cache_root().display(), "starting library command");
    let mut config = load_runtime_config(&assets)?;

    let target_args = match &args.command {
        LibraryCommand::Scan(scan) => scan.target.clone(),
        LibraryCommand::RefreshMetadata(target)
        | LibraryCommand::RefreshCovers(target)
        | LibraryCommand::RefreshCheats(target)
        | LibraryCommand::RefreshAll(target)
        | LibraryCommand::DownloadRom(target) => target.clone(),
    };

    if !target_args.rom_dirs.is_empty() {
        config.library.rom_dirs = target_args.rom_dirs.clone();
    }

    let mut service = LibraryService::new(config, assets)?;

    match args.command {
        LibraryCommand::Scan(scan) => {
            let snapshot = service.snapshot(LibraryFilter {
                query: scan.query.unwrap_or_default(),
                installed_only: scan.target.installed_only,
                view_mode: service.config().library.active_view,
            })?;
            if scan.target.json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                println!("Total: {}", snapshot.total_count);
                println!("Installed: {}", snapshot.installed_count);
                println!("Missing: {}", snapshot.missing_count);
                for entry in snapshot.entries {
                    let status = match entry.installed_status {
                        starbyte_frontend::InstalledStatus::Installed => "installed",
                        starbyte_frontend::InstalledStatus::Missing => "missing",
                    };
                    println!("[{status}] {} ({})", entry.display_title, entry.game_id);
                }
            }
        }
        LibraryCommand::RefreshMetadata(target) => {
            let count = service.refresh_metadata_index()?;
            service.save_config()?;
            print_library_refresh_json(
                target.json,
                json!({
                    "updated": "metadata",
                    "records": count,
                }),
            )?;
        }
        LibraryCommand::RefreshCovers(target) => {
            let count = service.refresh_covers(&library_target_from_args(&target))?;
            service.save_config()?;
            print_library_refresh_json(
                target.json,
                json!({
                    "updated": "covers",
                    "records": count,
                }),
            )?;
        }
        LibraryCommand::RefreshCheats(target) => {
            let count = service.refresh_cheats(&library_target_from_args(&target))?;
            service.save_config()?;
            print_library_refresh_json(
                target.json,
                json!({
                    "updated": "cheats",
                    "records": count,
                }),
            )?;
        }
        LibraryCommand::RefreshAll(target) => {
            let summary = service.refresh_all(&library_target_from_args(&target))?;
            service.save_config()?;
            print_library_refresh_json(target.json, serde_json::to_value(summary)?)?;
        }
        LibraryCommand::DownloadRom(target) => {
            let payload = json!({
                "updated": "rom_download",
                "supported": false,
                "reason": "ROM downloading is intentionally unsupported in this version",
                "target": {
                    "installed_only": target.installed_only,
                    "game_id": target.game_id,
                    "title": target.title,
                    "rom": target.rom.map(|path| path.display().to_string()),
                }
            });
            if target.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("ROM downloading is intentionally unsupported in this version.");
            }
        }
    }

    Ok(())
}

fn library_target_from_args(args: &LibraryTargetArgs) -> LibraryTarget {
    LibraryTarget {
        installed_only: args.installed_only,
        game_id: args.game_id.clone(),
        title: args.title.clone(),
        rom_path: args.rom.clone(),
    }
}

fn print_library_refresh_json(as_json: bool, payload: serde_json::Value) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if let Some(object) = payload.as_object() {
        for (key, value) in object {
            println!("{key}: {value}");
        }
    }
    Ok(())
}

fn run_compliance(args: ComplianceArgs, assets: AssetConfig) -> Result<()> {
    match args.command {
        ComplianceCommand::Summary { suite, dir } => match suite {
            ComplianceSuite::Cpu65816 => {
                let summary = testing::cpu_65816::summarize(&dir)?;
                println!("Suite: {}", summary.suite_name);
                println!("Files: {}", summary.file_count);
                println!("Vectors: {}", summary.vector_count);
            }
            ComplianceSuite::Spc700 => {
                let summary = testing::spc700::summarize(&dir)?;
                println!("Suite: {}", summary.suite_name);
                println!("Files: {}", summary.file_count);
                println!("Vectors: {}", summary.vector_count);
            }
        },
        ComplianceCommand::VerifyFormat {
            suite,
            dir,
            opcode,
            mode,
        } => {
            let opcode = parse_hex_opcode(&opcode)?;
            match suite {
                ComplianceSuite::Cpu65816 => {
                    let mode = mode.unwrap_or(Cpu65816ModeArg::Native).into();
                    let vectors = testing::cpu_65816::load_opcode_file(&dir, opcode, mode)?;
                    println!(
                        "Verified 65816 opcode 0x{opcode:02X} in {:?} mode: {} vector(s)",
                        mode,
                        vectors.len()
                    );
                }
                ComplianceSuite::Spc700 => {
                    let vectors = testing::spc700::load_opcode_file(&dir, opcode)?;
                    println!(
                        "Verified SPC700 opcode 0x{opcode:02X}: {} vector(s)",
                        vectors.len()
                    );
                }
            }
        }
        ComplianceCommand::RunCurrent {
            suite,
            dir,
            opcode,
            mode,
            max_failures,
        } => {
            let opcode = parse_hex_opcode(&opcode)?;
            match suite {
                ComplianceSuite::Cpu65816 => {
                    let mode = mode.unwrap_or(Cpu65816ModeArg::Native).into();
                    let vectors = testing::cpu_65816::load_opcode_file(&dir, opcode, mode)?;
                    let summary = testing::cpu_65816::run_with_current_core(&vectors, max_failures);
                    print_run_summary(&summary);
                    if summary.failed > 0 {
                        anyhow::bail!(
                            "65816 compliance failures: {} of {} vectors failed",
                            summary.failed,
                            summary.total
                        );
                    }
                }
                ComplianceSuite::Spc700 => {
                    let vectors = testing::spc700::load_opcode_file(&dir, opcode)?;
                    let summary = testing::spc700::run_with_current_core(&vectors, max_failures);
                    print_run_summary(&summary);
                    if summary.failed > 0 {
                        anyhow::bail!(
                            "SPC700 compliance failures: {} of {} vectors failed",
                            summary.failed,
                            summary.total
                        );
                    }
                }
            }
        }
        ComplianceCommand::RomSummary { dir } => {
            let summary = testing::rom::summarize(&dir)?;
            println!("Suite: {}", summary.suite_name);
            println!("Files: {}", summary.file_count);
            println!("Vectors: {}", summary.vector_count);
        }
        ComplianceCommand::RomRunCurrent {
            dir,
            max_failures,
            artifact_dir,
        } => {
            let fixtures = testing::rom::load_suite(&dir)?;
            let evaluations = testing::rom::run_with_current_core_detailed(&fixtures, &assets);
            let summary = build_rom_run_summary(&evaluations, max_failures);
            print_run_summary(&summary);
            maybe_write_regression_artifacts(&summary, &evaluations, artifact_dir.as_deref())?;
            if summary.failed > 0 {
                anyhow::bail!(
                    "ROM regression failures: {} of {} fixtures failed",
                    summary.failed,
                    summary.total
                );
            }
        }
        ComplianceCommand::CommercialSummary { dir } => {
            let summary = testing::commercial::summarize(&dir)?;
            println!("Suite: {}", summary.suite_name);
            println!("Files: {}", summary.file_count);
            println!("Vectors: {}", summary.vector_count);
        }
        ComplianceCommand::CommercialRunCurrent {
            dir,
            max_failures,
            artifact_dir,
            trace_out,
        } => {
            let fixtures = testing::commercial::load_suite(&dir)?;
            let executed = testing::commercial::run_with_current_core_executed(
                &fixtures,
                &assets,
                trace_out.is_some(),
            );
            let evaluations = executed
                .iter()
                .map(|executed_fixture| executed_fixture.evaluation.clone())
                .collect::<Vec<_>>();
            let summary = testing::commercial::build_run_summary(&evaluations, max_failures);
            print_run_summary(&summary);
            maybe_write_commercial_artifacts(
                &summary,
                &executed,
                artifact_dir.as_deref(),
                trace_out.as_deref(),
            )?;
            if summary.failed > 0 {
                anyhow::bail!(
                    "Commercial ROM regression failures: {} of {} fixtures failed",
                    summary.failed,
                    summary.total
                );
            }
        }
        ComplianceCommand::CommercialRecord(args) => {
            let controller1 = args
                .controller1
                .as_deref()
                .map(parse_controller_state)
                .transpose()?
                .unwrap_or_default();
            let mut recorded = testing::commercial::record_fixture_with_trace_start(
                &args.rom,
                args.frames,
                &assets,
                controller1,
                &[],
                args.trace_out.is_some(),
                args.trace_from_frame.unwrap_or(0),
            )?;
            let fixture_dir = args.fixture_out.parent().unwrap_or_else(|| Path::new("."));
            recorded.fixture.rom = make_relative_path(fixture_dir, &args.rom);
            ensure_parent_dir(&args.fixture_out)?;
            std::fs::write(
                &args.fixture_out,
                serde_json::to_string_pretty(&vec![recorded.fixture.clone()])?,
            )
            .with_context(|| {
                format!(
                    "failed to write commercial fixture to {}",
                    args.fixture_out.display()
                )
            })?;

            let report_path = args.fixture_out.with_extension("report.json");
            std::fs::write(
                &report_path,
                serde_json::to_string_pretty(&recorded.report)?,
            )
            .with_context(|| {
                format!(
                    "failed to write commercial record report to {}",
                    report_path.display()
                )
            })?;

            if let Some(trace_path) = args.trace_out.as_deref() {
                if let Some(trace) = recorded.trace.take() {
                    ensure_parent_dir(trace_path)?;
                    std::fs::write(trace_path, serde_json::to_string_pretty(&trace)?)
                        .with_context(|| {
                            format!(
                                "failed to write commercial instruction trace to {}",
                                trace_path.display()
                            )
                        })?;
                }
            }
        }
    }

    Ok(())
}

fn run_rom(args: RunArgs, assets: AssetConfig) -> Result<()> {
    let controller_events = args
        .controller1_events
        .as_deref()
        .map(|input| parse_controller_events(input, args.frames))
        .transpose()?
        .unwrap_or_default();
    if let Some(trace_frame) = args.trace_frame {
        anyhow::ensure!(
            trace_frame <= args.frames,
            "trace frame {trace_frame} exceeds requested frame count {}",
            args.frames
        );
    }
    let cartridge = Cartridge::load(&args.rom)
        .with_context(|| format!("failed to load ROM at {}", args.rom.display()))?;
    let save_ram_path = if args.no_save_ram {
        None
    } else {
        resolve_save_ram_path(&cartridge, assets.save_dir.as_deref())?
    };
    let load_state_path = resolve_state_path(
        args.load_state.as_deref(),
        assets.state_dir.as_deref(),
        &cartridge,
    )?;
    let save_state_path = resolve_state_path(
        args.save_state.as_deref(),
        assets.state_dir.as_deref(),
        &cartridge,
    )?;

    let mut emulator = EmulatorBuilder::new().assets(assets).build();
    let external_ipl_loaded = emulator.load_apu_ipl_rom()?;
    emulator.load_rom(cartridge);
    maybe_load_save_ram(&mut emulator, save_ram_path.as_deref())?;
    maybe_load_state(&mut emulator, load_state_path.as_deref())?;
    if let Some(controller) = args.controller1.as_deref() {
        emulator.set_controller1(parse_controller_state(controller)?);
    }
    // Only an explicitly requested path is written. Flush each frame so a
    // later emulation error preserves the preceding history.
    let mut frame_log = args
        .frame_log
        .as_deref()
        .map(|path| -> Result<std::fs::File> {
            ensure_parent_dir(path)?;
            std::fs::File::create(path)
                .with_context(|| format!("failed to create frame log at {}", path.display()))
        })
        .transpose()?;
    let mut saved_frame_images = 0_u32;
    let mut next_controller_event = 0_usize;
    for index in 0..args.frames {
        if let Some(&(frame, state)) = controller_events.get(next_controller_event)
            && frame == index + 1
        {
            emulator.set_controller1(state);
            info!(frame, "applied scheduled controller-1 input");
            next_controller_event += 1;
        }
        let mut instruction_records = Vec::new();
        let step = if args.trace_frame == Some(index + 1) {
            emulator.run_until_frame_observed(&mut |before, after, bus_events| {
                let address = (u32::from(before.pbr) << 16) | u32::from(before.pc);
                let opcode = bus_events
                    .first()
                    .filter(|event| {
                        event.access == starbyte_core::bus::AccessKind::Read
                            && event.address == address
                    })
                    .map(|event| event.value);
                instruction_records.push(json!({
                    "schema": "starbyte.instruction_trace.v1",
                    "frame": index + 1,
                    "instruction": instruction_records.len(),
                    "opcode": opcode,
                    "before": before,
                    "after": after,
                    "bus_events": bus_events,
                }));
            })
        } else {
            emulator.run_until_frame()
        };
        if let Some(file) = frame_log.as_mut() {
            write_frame_log_entry(file, &emulator, index + 1, step.as_ref().err())?;
        }
        if args.trace_frame == Some(index + 1) {
            write_selected_frame_trace(
                args.trace_out.as_deref(),
                &instruction_records,
                &emulator,
                step.as_ref().err(),
            )?;
        }
        step.with_context(|| format!("emulation failed at requested frame {}", index + 1))?;
        if let Some(dir) = args.frame_images_dir.as_deref()
            && saved_frame_images < args.max_frame_images
            && (index == 0 || (index + 1) % args.frame_image_every == 0)
        {
            let image = dir.join(format!("frame-{:06}.ppm", emulator.timing().frame));
            maybe_write_screenshot(emulator.framebuffer(), Some(&image))?;
            saved_frame_images += 1;
        }
    }

    maybe_write_save_ram(&emulator, save_ram_path.as_deref())?;

    if let Some(path) = save_state_path.as_deref() {
        let state = emulator.save_state()?;
        ensure_parent_dir(path)?;
        std::fs::write(path, state)
            .with_context(|| format!("failed to write save state to {}", path.display()))?;
    }
    maybe_write_screenshot(emulator.framebuffer(), args.screenshot.as_deref())?;
    maybe_write_run_report(
        &emulator,
        &args.rom,
        args.frames,
        save_ram_path.as_deref(),
        save_state_path.as_deref(),
        args.report_json.as_deref(),
    )?;

    let apu_status = emulator.apu_status();
    info!(
        frames = args.frames,
        apu_external_ipl_loaded = external_ipl_loaded,
        apu_has_ipl_rom = apu_status.has_ipl_rom,
        apu_using_builtin_bootstrap = apu_status.using_builtin_bootstrap,
        apu_spc700_steps = apu_status.spc700_steps,
        "completed bootstrap run"
    );
    Ok(())
}

fn resolve_save_ram_path(
    cartridge: &Cartridge,
    save_dir: Option<&Path>,
) -> Result<Option<PathBuf>> {
    let save_len = cartridge.header().ram_size_bytes();
    if save_len == 0 {
        return Ok(None);
    }

    let stem = cartridge
        .source()
        .and_then(Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| sanitize_file_stem(&cartridge.header().title));

    let path = match save_dir {
        Some(dir) => dir.join(format!("{stem}.srm")),
        None => {
            let source = cartridge.source().ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot resolve save path for ROM loaded without a filesystem source"
                )
            })?;
            source.with_extension("srm")
        }
    };

    Ok(Some(path))
}

fn resolve_state_path(
    explicit: Option<&Path>,
    state_dir: Option<&Path>,
    cartridge: &Cartridge,
) -> Result<Option<PathBuf>> {
    match (explicit, state_dir) {
        (Some(path), Some(dir)) if path.components().count() == 1 => Ok(Some(dir.join(path))),
        (Some(path), _) => Ok(Some(path.to_path_buf())),
        (None, Some(dir)) => {
            let stem = cartridge
                .source()
                .and_then(Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| sanitize_file_stem(&cartridge.header().title));
            Ok(Some(dir.join(format!("{stem}.state.json"))))
        }
        (None, None) => Ok(None),
    }
}

fn maybe_load_save_ram(emulator: &mut starbyte_core::Emulator, path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };

    if !path.exists() {
        return Ok(());
    }

    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read save RAM from {}", path.display()))?;
    emulator
        .load_save_ram(&bytes)
        .with_context(|| format!("failed to install save RAM from {}", path.display()))?;
    info!(path = %path.display(), bytes = bytes.len(), "loaded save RAM");
    Ok(())
}

fn maybe_load_state(emulator: &mut starbyte_core::Emulator, path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };

    if !path.exists() {
        return Ok(());
    }

    let state = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read save state from {}", path.display()))?;
    emulator
        .load_state(&state)
        .with_context(|| format!("failed to restore save state from {}", path.display()))?;
    info!(path = %path.display(), "loaded save state");
    Ok(())
}

fn maybe_write_save_ram(emulator: &starbyte_core::Emulator, path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let Some(bytes) = emulator.save_ram() else {
        return Ok(());
    };

    ensure_parent_dir(path)?;
    std::fs::write(path, &bytes)
        .with_context(|| format!("failed to write save RAM to {}", path.display()))?;
    info!(path = %path.display(), bytes = bytes.len(), "wrote save RAM");
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))
}

fn maybe_write_screenshot(
    framebuffer: &starbyte_core::ppu::FrameBuffer,
    path: Option<&Path>,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    ensure_parent_dir(path)?;

    let mut bytes = format!(
        "P6\n{} {}\n255\n",
        framebuffer.width(),
        framebuffer.height()
    )
    .into_bytes();
    for pixel in framebuffer.pixels().chunks_exact(4) {
        bytes.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write screenshot to {}", path.display()))?;
    Ok(())
}

/// Persist a bounded single-frame instruction trace, even when execution
/// returns an emulator error. The regular frame loop is never traced.
fn write_selected_frame_trace(
    path: Option<&Path>,
    records: &[serde_json::Value],
    emulator: &starbyte_core::Emulator,
    error: Option<&starbyte_core::error::Error>,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    ensure_parent_dir(path)?;
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create instruction trace at {}", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    for record in records {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    // The final failing instruction does not yield bus events. Retain its
    // CPU register state and error separately instead of dropping the trace.
    let footer = json!({
        "schema": "starbyte.instruction_trace_end.v1",
        "recorded_instructions": records.len(),
        "completed_frame": emulator.timing().frame,
        "cpu": emulator.cpu_registers(),
        "status": if error.is_some() { "error" } else { "ok" },
        "error": error.map(ToString::to_string),
    });
    serde_json::to_writer(&mut writer, &footer)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Append one independent JSONL record after a frame attempt. The framebuffer
/// describes the most recently *completed* frame when an attempt fails.
fn write_frame_log_entry(
    writer: &mut std::fs::File,
    emulator: &starbyte_core::Emulator,
    requested_frame: u32,
    error: Option<&starbyte_core::error::Error>,
) -> Result<()> {
    let frame = emulator.framebuffer();
    let cpu = emulator.cpu_registers();
    let (nonblack_pixels, distinct_rgb_colors) = framebuffer_color_metrics(frame);
    let (nmitimen, htime, vtime) = emulator.irq_timer_configuration();
    let (host_bits, latched_bits, auto_read_busy) = emulator.joypad_status();
    let report = json!({
        "schema": "starbyte.frame_log.v1",
        "requested_frame": requested_frame,
        "completed_frame": emulator.timing().frame,
        "status": if error.is_some() { "error" } else { "ok" },
        "error": error.map(ToString::to_string),
        "cpu": {
            "pc": cpu.pc,
            "pbr": cpu.pbr,
            "a": cpu.a,
            "x": cpu.x,
            "y": cpu.y,
            "s": cpu.s,
            "d": cpu.d,
            "dbr": cpu.dbr,
            "p": cpu.p,
            "emulation": cpu.emulation,
        },
        "framebuffer": {
            "hash": framebuffer_hash(frame),
            "nonblack_pixels": nonblack_pixels,
            "distinct_rgb_colors": distinct_rgb_colors,
            "width": frame.width(),
            "height": frame.height(),
        },
        "ppu_display": build_ppu_display_report(emulator),
        "ppu_write_activity": build_ppu_write_activity_report(emulator),
        "apu_io_activity": build_apu_io_activity_report(emulator),
        "dma_transferred_bytes": emulator.dma_transferred_bytes(),
        "irq_timer": {
            "nmitimen": nmitimen,
            "htime": htime,
            "vtime": vtime,
        },
        "joypad": {
            "host_controller1_bits": host_bits,
            "latched_controller1_bits": latched_bits,
            "auto_read_busy": auto_read_busy,
        },
        "apu_steps": emulator.apu_status().spc700_steps,
    });
    serde_json::to_writer(&mut *writer, &report)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn maybe_write_run_report(
    emulator: &starbyte_core::Emulator,
    rom: &Path,
    frames: u32,
    save_ram_path: Option<&Path>,
    save_state_path: Option<&Path>,
    report_path: Option<&Path>,
) -> Result<()> {
    let Some(report_path) = report_path else {
        return Ok(());
    };
    ensure_parent_dir(report_path)?;

    let pixels = emulator.framebuffer().pixels();
    let first_pixel = if pixels.len() >= 4 {
        vec![pixels[0], pixels[1], pixels[2], pixels[3]]
    } else {
        Vec::new()
    };
    let ppu_write_activity = build_ppu_write_activity_report(emulator);
    let apu_io_activity = build_apu_io_activity_report(emulator);
    let ppu_display = build_ppu_display_report(emulator);
    let (nonblack_pixels, distinct_rgb_colors) = framebuffer_color_metrics(emulator.framebuffer());
    let center_offset = (emulator.framebuffer().height() / 2 * emulator.framebuffer().width()
        + emulator.framebuffer().width() / 2)
        * 4;
    let center_pixel = pixels
        .get(center_offset..center_offset + 4)
        .map_or_else(Vec::new, |value| value.to_vec());
    let report = json!({
        "schema": "starbyte.run_report.v1",
        "rom": rom.display().to_string(),
        "frames": frames,
        "frame_counter": emulator.timing().frame,
        "cpu": {
            "pc": emulator.cpu_registers().pc,
            "pbr": emulator.cpu_registers().pbr,
        },
        "framebuffer": {
            "width": emulator.framebuffer().width(),
            "height": emulator.framebuffer().height(),
            "first_pixel_rgba": first_pixel,
            "center_pixel_rgba": center_pixel,
            "nonblack_pixels": nonblack_pixels,
            "distinct_rgb_colors": distinct_rgb_colors,
            "hash": framebuffer_hash(emulator.framebuffer()),
        },
        "audio_sample_count": emulator.audio_samples().samples.len(),
        "apu_steps": emulator.apu_status().spc700_steps,
        "apu_io_activity": apu_io_activity,
        "ppu_write_activity": ppu_write_activity,
        "ppu_display": ppu_display,
        "save_ram_path": save_ram_path.map(|path| path.display().to_string()),
        "save_state_path": save_state_path.map(|path| path.display().to_string()),
    });
    std::fs::write(report_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("failed to write run report to {}", report_path.display()))?;
    Ok(())
}

/// Snapshot the registers that most directly explain an all-black or
/// scrambled scene without introducing side effects on VRAM/OAM read ports.
fn build_ppu_display_report(emulator: &starbyte_core::Emulator) -> serde_json::Value {
    let register = |address: u16| emulator.peek_ppu_register(address).unwrap_or(0);
    let brightness = register(0x2100);
    let mode = register(0x2105);
    let mosaic = register(0x2106);
    json!({
        "forced_blank": brightness & 0x80 != 0,
        "brightness": brightness & 0x0F,
        "background_mode": mode & 0x07,
        "bg3_high_priority": mode & 0x08 != 0,
        "bg_tile_size_flags": mode >> 4,
        "mosaic_size": (mosaic >> 4) + 1,
        "mosaic_bg_mask": mosaic & 0x0F,
        "main_screen_enable_mask": register(0x212C),
        "sub_screen_enable_mask": register(0x212D),
        "vmain": register(0x2115),
        "bg_screen_base_registers": [
            register(0x2107),
            register(0x2108),
            register(0x2109),
            register(0x210A),
        ],
        "bg_character_base_registers": [register(0x210B), register(0x210C)],
    })
}

/// Count actual visible color variety, not just a framebuffer hash, to
/// distinguish black-screen startup from frames with useful game graphics.
fn framebuffer_color_metrics(frame: &starbyte_core::ppu::FrameBuffer) -> (usize, usize) {
    let mut nonblack_pixels = 0;
    let mut distinct_colors = std::collections::BTreeSet::new();
    for rgba in frame.pixels().chunks_exact(4) {
        let rgb = [rgba[0], rgba[1], rgba[2]];
        if rgb != [0, 0, 0] {
            nonblack_pixels += 1;
        }
        distinct_colors.insert(rgb);
    }
    (nonblack_pixels, distinct_colors.len())
}

fn build_ppu_write_activity_report(emulator: &starbyte_core::Emulator) -> serde_json::Value {
    let counts = emulator.system_observability().ppu_write_counts();
    let mut touched_registers = Vec::new();
    let mut visible_touched_registers = Vec::new();
    let mut total_writes = 0_u64;
    let mut visible_display_write_count = 0_u64;
    let mut final_register_values = serde_json::Map::new();

    for (index, count) in counts.iter().copied().enumerate() {
        total_writes = total_writes.saturating_add(u64::from(count));
        if count == 0 {
            continue;
        }

        let register = 0x2100_u16 + index as u16;
        let label = format!("${register:04X}");
        touched_registers.push(label.clone());

        if register <= 0x212C {
            visible_display_write_count =
                visible_display_write_count.saturating_add(u64::from(count));
            visible_touched_registers.push(label.clone());
            if let Some(value) = emulator.peek_ppu_register(register) {
                final_register_values.insert(label, json!(value));
            }
        }
    }

    json!({
        "total_writes": total_writes,
        "touched_registers": touched_registers,
        "visible_display_write_count": visible_display_write_count,
        "visible_display_registers_touched": visible_touched_registers,
        "final_register_values": final_register_values,
    })
}

fn build_apu_io_activity_report(emulator: &starbyte_core::Emulator) -> serde_json::Value {
    let observability = emulator.system_observability();
    let mut cpu_read_counts = serde_json::Map::new();
    let mut cpu_write_counts = serde_json::Map::new();

    for port in 0..4 {
        let register = 0x2140_u16 + port as u16;
        let label = format!("${register:04X}");
        cpu_read_counts.insert(
            label.clone(),
            json!(observability.apu_port_read_counts()[port]),
        );
        cpu_write_counts.insert(label, json!(observability.apu_port_write_counts()[port]));
    }

    json!({
        "cpu_to_apu_ports": emulator.cpu_to_apu_ports(),
        "apu_to_cpu_ports": emulator.apu_to_cpu_ports(),
        "cpu_read_counts": cpu_read_counts,
        "cpu_write_counts": cpu_write_counts,
    })
}

fn framebuffer_hash(framebuffer: &starbyte_core::ppu::FrameBuffer) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    framebuffer.width().hash(&mut hasher);
    framebuffer.height().hash(&mut hasher);
    framebuffer.pixels().hash(&mut hasher);
    hasher.finish()
}

fn maybe_write_regression_artifacts(
    summary: &testing::RunSummary,
    evaluations: &[testing::rom::FixtureEvaluation],
    artifact_dir: Option<&Path>,
) -> Result<()> {
    let Some(artifact_dir) = artifact_dir else {
        return Ok(());
    };
    std::fs::create_dir_all(artifact_dir).with_context(|| {
        format!(
            "failed to create artifact directory {}",
            artifact_dir.display()
        )
    })?;
    let summary_path = artifact_dir.join("summary.json");
    let report = json!({
        "suite": summary.suite_name,
        "total": summary.total,
        "passed": summary.passed,
        "failed": summary.failed,
        "failures": summary.failures.iter().map(|failure| json!({
            "label": failure.label,
            "reasons": failure.reasons,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(&summary_path, serde_json::to_string_pretty(&report)?).with_context(|| {
        format!(
            "failed to write regression summary to {}",
            summary_path.display()
        )
    })?;

    for evaluation in evaluations {
        let fixture_name = sanitize_file_stem(&evaluation.name);
        let path = artifact_dir.join(format!("{fixture_name}.json"));
        std::fs::write(&path, serde_json::to_string_pretty(evaluation)?)
            .with_context(|| format!("failed to write fixture report to {}", path.display()))?;
    }
    Ok(())
}

fn maybe_write_commercial_artifacts(
    summary: &testing::RunSummary,
    executed: &[testing::commercial::ExecutedFixture],
    artifact_dir: Option<&Path>,
    trace_out: Option<&Path>,
) -> Result<()> {
    if artifact_dir.is_none() && trace_out.is_none() {
        return Ok(());
    }

    if let Some(artifact_dir) = artifact_dir {
        std::fs::create_dir_all(artifact_dir).with_context(|| {
            format!(
                "failed to create artifact directory {}",
                artifact_dir.display()
            )
        })?;
        let summary_path = artifact_dir.join("summary.json");
        let report = json!({
            "suite": summary.suite_name,
            "total": summary.total,
            "passed": summary.passed,
            "failed": summary.failed,
            "failures": summary.failures.iter().map(|failure| json!({
                "label": failure.label,
                "reasons": failure.reasons,
            })).collect::<Vec<_>>(),
        });
        std::fs::write(&summary_path, serde_json::to_string_pretty(&report)?).with_context(
            || {
                format!(
                    "failed to write commercial regression summary to {}",
                    summary_path.display()
                )
            },
        )?;
    }

    if let Some(trace_dir) = trace_out {
        std::fs::create_dir_all(trace_dir)
            .with_context(|| format!("failed to create trace directory {}", trace_dir.display()))?;
    }

    for executed_fixture in executed {
        let fixture_name = sanitize_file_stem(&executed_fixture.evaluation.name);

        if let Some(artifact_dir) = artifact_dir {
            let path = artifact_dir.join(format!("{fixture_name}.json"));
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&executed_fixture.evaluation)?,
            )
            .with_context(|| format!("failed to write fixture report to {}", path.display()))?;
        }

        if let Some(trace_dir) = trace_out
            && let Some(trace) = &executed_fixture.trace
        {
            let path = trace_dir.join(format!("{fixture_name}.trace.json"));
            std::fs::write(&path, serde_json::to_string_pretty(trace)?)
                .with_context(|| format!("failed to write trace report to {}", path.display()))?;
        }
    }

    Ok(())
}

fn build_rom_run_summary(
    evaluations: &[testing::rom::FixtureEvaluation],
    max_failures: usize,
) -> testing::RunSummary {
    let mut passed = 0;
    let mut failures = Vec::new();

    for evaluation in evaluations {
        if evaluation.reasons.is_empty() {
            passed += 1;
            continue;
        }

        if failures.len() < max_failures {
            failures.push(testing::VectorFailure {
                label: evaluation.name.clone(),
                reasons: evaluation.reasons.clone(),
            });
        }
    }

    testing::RunSummary {
        suite_name: "ROM regression",
        total: evaluations.len(),
        passed,
        failed: evaluations.len().saturating_sub(passed),
        failures,
    }
}

fn make_relative_path(base_dir: &Path, target: &Path) -> PathBuf {
    let base_components = base_dir.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let common_len = base_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();

    if common_len == 0 {
        return target.to_path_buf();
    }

    let mut relative = PathBuf::new();
    for _ in common_len..base_components.len() {
        relative.push("..");
    }
    for component in target_components.iter().skip(common_len) {
        relative.push(component.as_os_str());
    }

    if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    }
}

/// Parse a deterministic one-based controller-1 input timeline.
///
/// Each semicolon-delimited event replaces the prior held state before its
/// frame: "300:start;303:none;400:right,b;430:none".
/// Events must be ordered, unique, and inside the requested frame window.
fn parse_controller_events(input: &str, frames: u32) -> Result<Vec<(u32, ControllerState)>> {
    anyhow::ensure!(
        !input.trim().is_empty(),
        "controller-1 event timeline is empty"
    );
    let mut events = Vec::new();
    let mut previous_frame = 0;
    for event in input.split(';') {
        let (frame_text, buttons_text) = event.trim().split_once(':').ok_or_else(|| {
            anyhow::anyhow!("invalid controller event '{event}'; expected frame:buttons")
        })?;
        let frame = frame_text
            .trim()
            .parse::<u32>()
            .with_context(|| format!("invalid controller event frame '{frame_text}'"))?;
        anyhow::ensure!(
            frame > previous_frame,
            "controller events must be in strictly increasing one-based frame order"
        );
        anyhow::ensure!(
            frame <= frames,
            "controller event at frame {frame} exceeds requested {frames} frames"
        );
        let buttons = buttons_text.trim();
        anyhow::ensure!(
            !buttons.is_empty(),
            "controller event at frame {frame} has no button state; use 'none' to release"
        );
        let state = if buttons.eq_ignore_ascii_case("none") {
            ControllerState::default()
        } else {
            parse_controller_state(buttons)
                .with_context(|| format!("invalid controller event at frame {frame}"))?
        };
        events.push((frame, state));
        previous_frame = frame;
    }
    Ok(events)
}

fn parse_controller_state(input: &str) -> Result<ControllerState> {
    let mut state = ControllerState::default();
    for token in input
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        match token.to_ascii_lowercase().as_str() {
            "b" => state.b = true,
            "y" => state.y = true,
            "select" => state.select = true,
            "start" => state.start = true,
            "up" => state.up = true,
            "down" => state.down = true,
            "left" => state.left = true,
            "right" => state.right = true,
            "a" => state.a = true,
            "x" => state.x = true,
            "l" => state.l = true,
            "r" => state.r = true,
            other => anyhow::bail!("unknown controller button `{other}`"),
        }
    }
    Ok(state)
}

fn sanitize_file_stem(input: &str) -> String {
    let mut stem = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches(|ch| ch == '_' || ch == '.' || ch == ' ')
        .to_owned();

    if stem.is_empty() {
        stem = "starbyte-save".to_owned();
    }

    if is_windows_reserved_stem(&stem) {
        stem.push_str("_rom");
    }

    stem
}

fn is_windows_reserved_stem(stem: &str) -> bool {
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn print_config(format: ConfigFormat) -> Result<()> {
    let config = RuntimeConfig::default();
    match format {
        ConfigFormat::Toml => {
            println!("{}", toml::to_string_pretty(&config)?);
        }
        ConfigFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
    }
    Ok(())
}

fn parse_hex_opcode(input: &str) -> Result<u8> {
    let trimmed = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .unwrap_or(input);
    u8::from_str_radix(trimmed, 16)
        .with_context(|| format!("invalid opcode `{input}`; expected hex such as 00 or 0xA9"))
}

fn print_run_summary(summary: &testing::RunSummary) {
    println!("Suite: {}", summary.suite_name);
    println!("Total: {}", summary.total);
    println!("Passed: {}", summary.passed);
    println!("Failed: {}", summary.failed);
    for failure in &summary.failures {
        println!("Failure: {}", failure.label);
        for reason in &failure.reasons {
            println!("  - {}", reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use starbyte_core::{Emulator, Error, cartridge::Cartridge};
    use tempfile::tempdir;

    use super::{
        parse_controller_events, resolve_state_path, sanitize_file_stem, write_frame_log_entry,
    };

    fn test_cartridge() -> Cartridge {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 20].copy_from_slice(b"STARBYTE PATH TEST  ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        Cartridge::from_bytes(rom, Some(PathBuf::from("C:/ROMs/test game.sfc"))).unwrap()
    }

    #[test]
    fn controller_timeline_supports_press_release_and_multiple_buttons() {
        let events = parse_controller_events("2:start;4:none;5:right,b", 5).unwrap();
        assert_eq!(
            events.iter().map(|(frame, _)| *frame).collect::<Vec<_>>(),
            [2, 4, 5]
        );
        assert!(events[0].1.start);
        assert!(!events[0].1.right);
        assert_eq!(events[1].1.to_bits(), 0);
        assert!(events[2].1.right);
        assert!(events[2].1.b);
        assert_eq!(events[2].1.to_bits(), (1 << 7) | 1);
    }

    #[test]
    fn controller_timeline_rejects_invalid_frames_and_button_names() {
        for invalid in [
            "",
            "0:start",
            "1:start;1:none",
            "3:start;2:none",
            "6:start",
            "2:start;",
            "2:",
            "2start",
            "2:space",
        ] {
            assert!(
                parse_controller_events(invalid, 5).is_err(),
                "should reject invalid timeline: {invalid:?}"
            );
        }
        assert!(parse_controller_events("1:START;5:NoNe", 5).is_ok());
    }

    #[test]
    fn frame_log_error_is_written_with_last_completed_frame() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("frames.jsonl");
        let mut writer = std::fs::File::create(&path).unwrap();
        let emulator = Emulator::default();
        let stall = Error::FrameStalled {
            frame: 0,
            instructions: 20_000,
            elapsed_ms: 0,
            pc: 0x008000,
        };
        write_frame_log_entry(&mut writer, &emulator, 1, Some(&stall)).unwrap();
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(record["schema"], "starbyte.frame_log.v1");
        assert_eq!(record["requested_frame"], 1);
        assert_eq!(record["completed_frame"], 0);
        assert_eq!(record["status"], "error");
        assert!(
            record["error"]
                .as_str()
                .unwrap()
                .contains("CPU PC 0x008000")
        );
        assert!(record["framebuffer"]["hash"].is_number());
    }

    #[test]
    fn sanitizes_windows_hostile_file_stems() {
        assert_eq!(sanitize_file_stem("CON"), "CON_rom");
        assert_eq!(sanitize_file_stem("AUX"), "AUX_rom");
        assert_eq!(sanitize_file_stem("bad:name*test"), "bad_name_test");
        assert_eq!(sanitize_file_stem(" trailing. "), "trailing");
    }

    #[test]
    fn resolves_state_path_inside_state_dir() {
        let cartridge = test_cartridge();
        let path = resolve_state_path(None, Some(PathBuf::from("states").as_path()), &cartridge)
            .unwrap()
            .unwrap();
        assert_eq!(path, PathBuf::from("states/test game.state.json"));
    }
}
