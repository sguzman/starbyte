//! Starbyte CLI bootstrap entrypoint.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
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
        Command::Compliance(args) => run_compliance(args, assets),
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
            let summary = testing::rom::run_with_current_core(&fixtures, &assets, max_failures);
            print_run_summary(&summary);
            maybe_write_regression_artifacts(&summary, artifact_dir.as_deref())?;
            if summary.failed > 0 {
                anyhow::bail!(
                    "ROM regression failures: {} of {} fixtures failed",
                    summary.failed,
                    summary.total
                );
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
    let save_ram_path = resolve_save_ram_path(&cartridge, assets.save_dir.as_deref())?;
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
    emulator.load_apu_ipl_rom()?;
    emulator.load_rom(cartridge);
    maybe_load_save_ram(&mut emulator, save_ram_path.as_deref())?;
    maybe_load_state(&mut emulator, load_state_path.as_deref())?;
    if let Some(controller) = args.controller1.as_deref() {
        emulator.set_controller1(parse_controller_state(controller)?);
    }
    for _ in 0..args.frames {
        emulator.run_until_frame()?;
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
        apu_has_ipl_rom = apu_status.has_ipl_rom,
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
    let report = json!({
        "rom": rom.display().to_string(),
        "frames": frames,
        "frame_counter": emulator.timing().frame,
        "framebuffer": {
            "width": emulator.framebuffer().width(),
            "height": emulator.framebuffer().height(),
            "first_pixel_rgba": first_pixel,
        },
        "audio_sample_count": emulator.audio_samples().samples.len(),
        "apu_steps": emulator.apu_status().spc700_steps,
        "save_ram_path": save_ram_path.map(|path| path.display().to_string()),
        "save_state_path": save_state_path.map(|path| path.display().to_string()),
    });
    std::fs::write(report_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("failed to write run report to {}", report_path.display()))?;
    Ok(())
}

fn maybe_write_regression_artifacts(
    summary: &testing::RunSummary,
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
    Ok(())
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

    use starbyte_core::cartridge::Cartridge;

    use super::{resolve_state_path, sanitize_file_stem};

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
