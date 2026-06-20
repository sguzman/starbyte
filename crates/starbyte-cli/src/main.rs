//! Starbyte CLI bootstrap entrypoint.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use tracing::{Level, info};
use tracing_subscriber::{EnvFilter, fmt};

use starbyte_core::{
    EmulatorBuilder,
    cartridge::Cartridge,
    manifest::{AssetConfig, RuntimeConfig},
    testing,
};

#[derive(Debug, Parser)]
#[command(
    name = "starbyte",
    about = "CLI-first bootstrap runner for the Starbyte SNES emulator"
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
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect or validate external compliance corpora.
    Compliance(ComplianceArgs),
    /// Inspect ROM metadata without running emulation.
    Inspect { rom: PathBuf },
    /// Run the bootstrap emulator for a fixed number of frames.
    Run(RunArgs),
    /// Emit a sample runtime configuration file to stdout.
    PrintConfig { format: ConfigFormat },
}

#[derive(Debug, Args)]
struct RunArgs {
    /// ROM image to load.
    rom: PathBuf,

    /// Number of placeholder frames to execute before exiting.
    #[arg(long, default_value_t = 1)]
    frames: u32,

    /// Serialize emulator state to this file before exit.
    #[arg(long)]
    save_state: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ComplianceArgs {
    #[command(subcommand)]
    command: ComplianceCommand,
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
    };

    match cli.command {
        Command::Compliance(args) => run_compliance(args),
        Command::Inspect { rom } => inspect_rom(rom),
        Command::Run(args) => run_rom(args, assets),
        Command::PrintConfig { format } => print_config(format),
    }
}

fn install_tracing(logging: &LoggingArgs) -> Result<()> {
    let filter = logging
        .log_filter
        .clone()
        .unwrap_or_else(|| default_filter(logging.verbose));

    fmt()
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

fn inspect_rom(path: PathBuf) -> Result<()> {
    let cartridge = Cartridge::load(&path)
        .with_context(|| format!("failed to inspect ROM at {}", path.display()))?;

    println!("Title: {}", cartridge.header().title);
    println!("Mapper: {:?}", cartridge.mapper());
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

fn run_compliance(args: ComplianceArgs) -> Result<()> {
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
    }

    Ok(())
}

fn run_rom(args: RunArgs, assets: AssetConfig) -> Result<()> {
    if assets.spc700_ipl.is_none() {
        anyhow::bail!("missing required firmware path: pass --spc700-ipl /path/to/spc700.rom");
    }

    let cartridge = Cartridge::load(&args.rom)
        .with_context(|| format!("failed to load ROM at {}", args.rom.display()))?;

    let mut emulator = EmulatorBuilder::new().assets(assets).build();
    emulator.load_rom(cartridge);
    for _ in 0..args.frames {
        emulator.run_until_frame()?;
    }

    if let Some(path) = args.save_state {
        let state = emulator.save_state()?;
        std::fs::write(&path, state)
            .with_context(|| format!("failed to write save state to {}", path.display()))?;
    }

    info!(frames = args.frames, "completed bootstrap run");
    Ok(())
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
