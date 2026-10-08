//! Commercial-ROM boot harness for local milestone recording and replay.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bus::{AccessKind, BusEvent};
use crate::cartridge::Cartridge;
use crate::emulator::{Emulator, EmulatorBuilder};
use crate::error::{Error, Result};
use crate::input::ControllerState;
use crate::manifest::AssetConfig;

use super::{RunSummary, SuiteSummary, VectorFailure};
use crate::testing::rom::PixelSampleExpectation;

const DEFAULT_MMIO_PROBES: [(&str, u16); 6] = [
    ("inidisp", 0x2100),
    ("obsel", 0x2101),
    ("bg1sc", 0x2107),
    ("bg2sc", 0x2108),
    ("bg3sc", 0x2109),
    ("tm", 0x212C),
];
const DEFAULT_WRAM_PROBE_LIMIT: usize = 8;

/// One local commercial-ROM boot fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommercialFixture {
    /// Human-readable fixture name.
    pub name: String,
    /// ROM path relative to the suite file.
    pub rom: PathBuf,
    /// Number of frames to execute.
    pub frames: u32,
    /// Optional controller-1 state held during the run.
    #[serde(default)]
    pub controller1: ControllerState,
    /// Host writes applied after ROM load and before execution.
    #[serde(default)]
    pub setup_writes: Vec<(u32, u8)>,
    /// Expected milestone evidence.
    pub expected: ExpectedCommercialOutcome,
    /// Optional trace capture preference for this fixture.
    pub trace: Option<TraceCaptureOptions>,
}

/// Expected milestone evidence for one commercial boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedCommercialOutcome {
    /// Expected frame counter after execution.
    pub frame: Option<u64>,
    /// Expected final CPU program counter.
    pub cpu_pc: Option<u16>,
    /// Expected inclusive CPU program counter range.
    pub cpu_pc_range: Option<[u16; 2]>,
    /// Expected final CPU program bank.
    pub cpu_pbr: Option<u8>,
    /// Expected framebuffer hash.
    pub frame_hash: Option<u64>,
    /// Expected first framebuffer pixel.
    pub first_pixel_rgba: Option<[u8; 4]>,
    /// Expected sampled pixels.
    #[serde(default)]
    pub pixel_samples: Vec<PixelSampleExpectation>,
    /// Exact-value WRAM probes.
    #[serde(default)]
    pub wram_probes: Vec<ByteProbeExpectation>,
    /// Exact-value MMIO probes captured without side effects.
    #[serde(default)]
    pub mmio_probes: Vec<MmioProbeExpectation>,
    /// Expected APU I/O activity.
    pub apu_io_activity: Option<ApuIoExpectation>,
    /// Expected PPU write activity.
    pub ppu_write_activity: Option<PpuWriteActivityExpectation>,
}

/// One named expected WRAM value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteProbeExpectation {
    /// Human-readable probe label.
    pub name: String,
    /// Absolute CPU-visible address.
    pub address: u32,
    /// Expected byte value.
    pub expected: u8,
}

/// One named expected MMIO register value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmioProbeExpectation {
    /// Human-readable probe label.
    pub name: String,
    /// CPU-visible MMIO register.
    pub register: u16,
    /// Expected byte value.
    pub expected: u8,
}

/// Optional count expectation used for activity-based checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountExpectation {
    /// Lower bound if any.
    pub min: Option<u64>,
    /// Upper bound if any.
    pub max: Option<u64>,
}

impl CountExpectation {
    fn matches(&self, actual: u64) -> bool {
        self.min.is_none_or(|min| actual >= min) && self.max.is_none_or(|max| actual <= max)
    }
}

/// One named APU port counter expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApuPortCountExpectation {
    /// Human-readable label.
    pub name: String,
    /// CPU-visible APU MMIO register.
    pub register: u16,
    /// Allowed count range.
    pub count: CountExpectation,
}

/// Expected APU I/O activity summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApuIoExpectation {
    /// Expected final CPU-written ports if any.
    pub cpu_to_apu_ports: Option<[u8; 4]>,
    /// Expected final APU-return ports if any.
    pub apu_to_cpu_ports: Option<[u8; 4]>,
    /// Expected CPU read counts from `$2140-$2143`.
    #[serde(default)]
    pub cpu_read_counts: Vec<ApuPortCountExpectation>,
    /// Expected CPU write counts to `$2140-$2143`.
    #[serde(default)]
    pub cpu_write_counts: Vec<ApuPortCountExpectation>,
}

/// Expected PPU write activity summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PpuWriteActivityExpectation {
    /// Registers that must have been touched at least once.
    #[serde(default)]
    pub required_visible_registers_touched: Vec<u16>,
    /// Minimum total PPU writes.
    pub min_total_writes: Option<u64>,
    /// Minimum writes into visible-display setup registers.
    pub min_visible_display_write_count: Option<u64>,
}

/// Optional trace-capture preference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceCaptureOptions {
    /// Whether instruction trace capture is desired for this fixture.
    pub capture_instruction_trace: bool,
    /// Number of initial completed frames excluded from trace capture.
    #[serde(default)]
    pub trace_start_frame: u32,
}

/// Recorded evidence from one commercial boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommercialRunReport {
    /// Human-readable fixture name.
    pub name: String,
    /// Loaded ROM path.
    pub rom: String,
    /// Requested frame count.
    pub frames_requested: u32,
    /// Completed frame count.
    pub frame_counter: u64,
    /// Final CPU program counter.
    pub cpu_pc: u16,
    /// Final CPU program bank.
    pub cpu_pbr: u8,
    /// Final framebuffer hash.
    pub frame_hash: u64,
    /// First framebuffer pixel.
    pub first_pixel_rgba: [u8; 4],
    /// Final APU step count.
    pub apu_steps: u64,
    /// Observed WRAM probes.
    pub wram_probes: Vec<ObservedByteProbe>,
    /// Observed MMIO probes.
    pub mmio_probes: Vec<ObservedMmioProbe>,
    /// Observed APU I/O activity summary.
    pub apu_io_activity: ObservedApuIoActivity,
    /// Observed PPU write activity summary.
    pub ppu_write_activity: ObservedPpuWriteActivity,
    /// Observed framebuffer samples.
    pub sampled_pixels: Vec<PixelSampleExpectation>,
    /// Trace metadata if trace capture ran.
    pub trace: Option<TraceArtifactMetadata>,
}

/// One observed WRAM probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedByteProbe {
    /// Human-readable probe label.
    pub name: String,
    /// Absolute address.
    pub address: u32,
    /// Observed byte value.
    pub value: u8,
}

/// One observed MMIO register probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedMmioProbe {
    /// Human-readable probe label.
    pub name: String,
    /// MMIO register.
    pub register: u16,
    /// Observed byte value.
    pub value: u8,
}

/// Observed counter for one APU MMIO port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedApuPortCount {
    /// MMIO register.
    pub register: u16,
    /// Observed count.
    pub count: u64,
}

/// Observed APU I/O activity summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedApuIoActivity {
    /// Final CPU-written ports.
    pub cpu_to_apu_ports: [u8; 4],
    /// Final APU-return ports.
    pub apu_to_cpu_ports: [u8; 4],
    /// CPU reads from `$2140-$2143`.
    pub cpu_read_counts: Vec<ObservedApuPortCount>,
    /// CPU writes to `$2140-$2143`.
    pub cpu_write_counts: Vec<ObservedApuPortCount>,
}

/// Observed PPU write activity summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedPpuWriteActivity {
    /// Total writes into `$2100-$213F`.
    pub total_writes: u64,
    /// Registers touched at least once.
    pub touched_registers: Vec<u16>,
    /// Total writes into `$2100-$212C`.
    pub visible_display_write_count: u64,
    /// Visible-display registers touched at least once.
    pub visible_display_registers_touched: Vec<u16>,
}

/// Optional trace artifact metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceArtifactMetadata {
    /// Instruction records written.
    pub instruction_count: usize,
    /// Artifact path if persisted by the caller.
    pub path: Option<String>,
}

/// One recorded instruction-level commercial trace entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionTraceRecord {
    /// Frame number at the start of the instruction.
    pub frame: u64,
    /// Program bank at the start of the instruction.
    pub pbr: u8,
    /// Program counter at the start of the instruction.
    pub pc: u16,
    /// Fetched opcode, or null for an interrupt-service step with no opcode fetch.
    pub opcode: Option<u8>,
    /// MMIO or APU-related bus events seen during this instruction.
    pub mmio_events: Vec<BusEvent>,
}

/// Detailed result from one commercial fixture execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureEvaluation {
    /// Human-readable fixture name.
    pub name: String,
    /// Failure reasons, if any.
    pub reasons: Vec<String>,
    /// Captured execution evidence.
    pub report: CommercialRunReport,
}

/// One executed commercial fixture plus any optional trace records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutedFixture {
    /// Evaluation result for the fixture.
    pub evaluation: FixtureEvaluation,
    /// Optional trace records collected during execution.
    pub trace: Option<Vec<InstructionTraceRecord>>,
}

/// Output from recording a commercial fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedFixture {
    /// Fixture JSON payload to persist.
    pub fixture: CommercialFixture,
    /// Evidence captured while recording.
    pub report: CommercialRunReport,
    /// Optional trace records if requested.
    pub trace: Option<Vec<InstructionTraceRecord>>,
}

/// Load every commercial fixture file in a suite directory.
pub fn load_suite(dir: impl AsRef<Path>) -> Result<Vec<CommercialFixture>> {
    let dir = dir.as_ref();
    let mut fixtures = Vec::new();
    for path in discover_suite_files(dir) {
        let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
        let rows: Vec<RawCommercialFixture> = serde_json::from_str(&raw)?;
        fixtures.extend(
            rows.into_iter()
                .map(|row| row.into_fixture(path.parent().unwrap_or(dir))),
        );
    }
    Ok(fixtures)
}

/// Summarize a commercial fixture directory.
pub fn summarize(dir: impl AsRef<Path>) -> Result<SuiteSummary> {
    let dir = dir.as_ref();
    let files = discover_suite_files(dir);
    let mut vector_count = 0;
    for path in &files {
        let raw = fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
        vector_count += rows.len();
    }

    Ok(SuiteSummary {
        suite_name: "Commercial ROM regression",
        file_count: files.len(),
        vector_count,
    })
}

/// Resolve all suite file paths for deterministic iteration.
#[must_use]
pub fn discover_suite_files(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let is_json = matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("json")
            );
            let stem = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("");
            is_json && !stem.ends_with(".report") && !stem.ends_with(".trace")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

/// Execute the suite against the current in-tree emulator.
#[must_use]
pub fn run_with_current_core(
    fixtures: &[CommercialFixture],
    assets: &AssetConfig,
    max_failures: usize,
) -> RunSummary {
    let evaluations = run_with_current_core_detailed(fixtures, assets, false);
    build_run_summary(&evaluations, max_failures)
}

/// Execute the suite and return detailed reports for each fixture.
#[must_use]
pub fn run_with_current_core_detailed(
    fixtures: &[CommercialFixture],
    assets: &AssetConfig,
    capture_trace: bool,
) -> Vec<FixtureEvaluation> {
    fixtures
        .iter()
        .map(|fixture| execute_fixture(fixture, assets, capture_trace).evaluation)
        .collect()
}

/// Execute the suite and retain optional trace records.
#[must_use]
pub fn run_with_current_core_executed(
    fixtures: &[CommercialFixture],
    assets: &AssetConfig,
    capture_trace: bool,
) -> Vec<ExecutedFixture> {
    fixtures
        .iter()
        .map(|fixture| execute_fixture(fixture, assets, capture_trace))
        .collect()
}

/// Record one commercial fixture directly from a local ROM run.
pub fn record_fixture(
    rom_path: &Path,
    frames: u32,
    assets: &AssetConfig,
    controller1: ControllerState,
    setup_writes: &[(u32, u8)],
    capture_trace: bool,
) -> Result<RecordedFixture> {
    record_fixture_with_trace_start(
        rom_path,
        frames,
        assets,
        controller1,
        setup_writes,
        capture_trace,
        0,
    )
}

/// Record a commercial fixture while tracing only from a selected frame.
/// Earlier frames are executed through the normal guarded frame runner.
pub fn record_fixture_with_trace_start(
    rom_path: &Path,
    frames: u32,
    assets: &AssetConfig,
    controller1: ControllerState,
    setup_writes: &[(u32, u8)],
    capture_trace: bool,
    trace_start_frame: u32,
) -> Result<RecordedFixture> {
    if trace_start_frame > frames {
        return Err(Error::InvalidRom(format!(
            "trace start frame {trace_start_frame} exceeds requested {frames} frames"
        )));
    }
    let cartridge = Cartridge::load(rom_path)?;
    let title = cartridge.header().title.trim().to_owned();

    let mut emulator = EmulatorBuilder::new().assets(assets.clone()).build();
    let _ = emulator.load_apu_ipl_rom();
    emulator.load_rom(cartridge);
    emulator.set_controller1(controller1);
    for (address, value) in setup_writes {
        emulator.host_write_u8(*address, *value);
    }

    let trace = run_emulator_for_frames(&mut emulator, frames, capture_trace, trace_start_frame)?;
    let wram_probes = select_default_wram_probes(&mut emulator, DEFAULT_WRAM_PROBE_LIMIT);
    let mmio_probes = default_mmio_probe_expectations(&emulator);
    let report = build_report(
        &format!("{title} commercial boot"),
        rom_path,
        frames,
        &mut emulator,
        &wram_probes,
        &mmio_probes,
        &[],
        trace.as_ref().map(|records| TraceArtifactMetadata {
            instruction_count: records.len(),
            path: None,
        }),
    );

    let fixture = CommercialFixture {
        name: format!("{title} commercial boot"),
        rom: rom_path.to_path_buf(),
        frames,
        controller1,
        setup_writes: setup_writes.to_vec(),
        expected: ExpectedCommercialOutcome {
            frame: Some(report.frame_counter),
            cpu_pc: Some(report.cpu_pc),
            cpu_pc_range: None,
            cpu_pbr: Some(report.cpu_pbr),
            frame_hash: Some(report.frame_hash),
            first_pixel_rgba: Some(report.first_pixel_rgba),
            pixel_samples: report.sampled_pixels.clone(),
            wram_probes,
            mmio_probes,
            apu_io_activity: Some(recorded_apu_expectation(&report.apu_io_activity)),
            ppu_write_activity: Some(recorded_ppu_expectation(&report.ppu_write_activity)),
        },
        trace: Some(TraceCaptureOptions {
            capture_instruction_trace: capture_trace,
            trace_start_frame,
        }),
    };

    Ok(RecordedFixture {
        fixture,
        report,
        trace,
    })
}

/// Build a run summary from detailed commercial evaluations.
#[must_use]
pub fn build_run_summary(evaluations: &[FixtureEvaluation], max_failures: usize) -> RunSummary {
    let mut passed = 0;
    let mut failures = Vec::new();

    for evaluation in evaluations {
        if evaluation.reasons.is_empty() {
            passed += 1;
            continue;
        }

        if failures.len() < max_failures {
            failures.push(VectorFailure {
                label: evaluation.name.clone(),
                reasons: evaluation.reasons.clone(),
            });
        }
    }

    RunSummary {
        suite_name: "Commercial ROM regression",
        total: evaluations.len(),
        passed,
        failed: evaluations.len().saturating_sub(passed),
        failures,
    }
}

fn execute_fixture(
    fixture: &CommercialFixture,
    assets: &AssetConfig,
    capture_trace: bool,
) -> ExecutedFixture {
    let mut reasons = Vec::new();

    let cartridge = match Cartridge::load(&fixture.rom) {
        Ok(cartridge) => cartridge,
        Err(error) => {
            return ExecutedFixture {
                evaluation: FixtureEvaluation {
                    name: fixture.name.clone(),
                    reasons: vec![error.to_string()],
                    report: CommercialRunReport {
                        name: fixture.name.clone(),
                        rom: fixture.rom.display().to_string(),
                        frames_requested: fixture.frames,
                        frame_counter: 0,
                        cpu_pc: 0,
                        cpu_pbr: 0,
                        frame_hash: 0,
                        first_pixel_rgba: [0, 0, 0, 0],
                        apu_steps: 0,
                        wram_probes: Vec::new(),
                        mmio_probes: Vec::new(),
                        apu_io_activity: empty_apu_report(),
                        ppu_write_activity: empty_ppu_report(),
                        sampled_pixels: Vec::new(),
                        trace: None,
                    },
                },
                trace: None,
            };
        }
    };

    let mut emulator = EmulatorBuilder::new().assets(assets.clone()).build();
    let _ = emulator.load_apu_ipl_rom();
    emulator.load_rom(cartridge);
    emulator.set_controller1(fixture.controller1);
    for (address, value) in &fixture.setup_writes {
        emulator.host_write_u8(*address, *value);
    }

    let trace_start_frame = fixture
        .trace
        .as_ref()
        .map_or(0, |options| options.trace_start_frame);
    let trace = match run_emulator_for_frames(
        &mut emulator,
        fixture.frames,
        capture_trace,
        trace_start_frame,
    ) {
        Ok(trace) => trace,
        Err(error) => {
            reasons.push(error.to_string());
            let report = build_report(
                &fixture.name,
                &fixture.rom,
                fixture.frames,
                &mut emulator,
                &fixture.expected.wram_probes,
                &fixture.expected.mmio_probes,
                &fixture.expected.pixel_samples,
                None,
            );
            return ExecutedFixture {
                evaluation: FixtureEvaluation {
                    name: fixture.name.clone(),
                    reasons,
                    report,
                },
                trace: None,
            };
        }
    };

    let report = build_report(
        &fixture.name,
        &fixture.rom,
        fixture.frames,
        &mut emulator,
        &fixture.expected.wram_probes,
        &fixture.expected.mmio_probes,
        &fixture.expected.pixel_samples,
        trace.as_ref().map(|records| TraceArtifactMetadata {
            instruction_count: records.len(),
            path: None,
        }),
    );
    evaluate_report_against_expectations(&report, &fixture.expected, &mut reasons);

    ExecutedFixture {
        evaluation: FixtureEvaluation {
            name: fixture.name.clone(),
            reasons,
            report,
        },
        trace,
    }
}

fn evaluate_report_against_expectations(
    report: &CommercialRunReport,
    expected: &ExpectedCommercialOutcome,
    reasons: &mut Vec<String>,
) {
    if let Some(expected_frame) = expected.frame
        && report.frame_counter != expected_frame
    {
        reasons.push(format!(
            "frame mismatch: expected {}, got {}",
            expected_frame, report.frame_counter
        ));
    }

    if let Some(expected_pc) = expected.cpu_pc
        && report.cpu_pc != expected_pc
    {
        reasons.push(format!(
            "cpu pc mismatch: expected 0x{expected_pc:04X}, got 0x{:04X}",
            report.cpu_pc
        ));
    }

    if let Some([start, end]) = expected.cpu_pc_range
        && !(start..=end).contains(&report.cpu_pc)
    {
        reasons.push(format!(
            "cpu pc range mismatch: expected 0x{:04X} inside 0x{start:04X}..=0x{end:04X}",
            report.cpu_pc
        ));
    }

    if let Some(expected_pbr) = expected.cpu_pbr
        && report.cpu_pbr != expected_pbr
    {
        reasons.push(format!(
            "cpu pbr mismatch: expected 0x{expected_pbr:02X}, got 0x{:02X}",
            report.cpu_pbr
        ));
    }

    if let Some(expected_hash) = expected.frame_hash
        && report.frame_hash != expected_hash
    {
        reasons.push(format!(
            "frame hash mismatch: expected 0x{expected_hash:016X}, got 0x{:016X}",
            report.frame_hash
        ));
    }

    if let Some(expected_pixel) = expected.first_pixel_rgba
        && report.first_pixel_rgba != expected_pixel
    {
        reasons.push(format!(
            "first pixel mismatch: expected {:?}, got {:?}",
            expected_pixel, report.first_pixel_rgba
        ));
    }

    for (expected_probe, actual_probe) in expected.wram_probes.iter().zip(&report.wram_probes) {
        if actual_probe.value != expected_probe.expected {
            reasons.push(format!(
                "WRAM probe `{}` mismatch at 0x{:06X}: expected 0x{:02X}, got 0x{:02X}",
                expected_probe.name,
                expected_probe.address,
                expected_probe.expected,
                actual_probe.value
            ));
        }
    }

    for (expected_probe, actual_probe) in expected.mmio_probes.iter().zip(&report.mmio_probes) {
        if actual_probe.value != expected_probe.expected {
            reasons.push(format!(
                "MMIO probe `{}` mismatch at ${:04X}: expected 0x{:02X}, got 0x{:02X}",
                expected_probe.name,
                expected_probe.register,
                expected_probe.expected,
                actual_probe.value
            ));
        }
    }

    for sample in &expected.pixel_samples {
        let actual = report
            .sampled_pixels
            .iter()
            .find(|candidate| candidate.x == sample.x && candidate.y == sample.y)
            .map(|candidate| candidate.rgba)
            .unwrap_or([0, 0, 0, 0]);
        if actual != sample.rgba {
            reasons.push(format!(
                "pixel mismatch at ({}, {}): expected {:?}, got {:?}",
                sample.x, sample.y, sample.rgba, actual
            ));
        }
    }

    if let Some(expected_apu) = &expected.apu_io_activity {
        if let Some(expected_ports) = expected_apu.cpu_to_apu_ports
            && report.apu_io_activity.cpu_to_apu_ports != expected_ports
        {
            reasons.push(format!(
                "cpu_to_apu_ports mismatch: expected {:?}, got {:?}",
                expected_ports, report.apu_io_activity.cpu_to_apu_ports
            ));
        }
        if let Some(expected_ports) = expected_apu.apu_to_cpu_ports
            && report.apu_io_activity.apu_to_cpu_ports != expected_ports
        {
            reasons.push(format!(
                "apu_to_cpu_ports mismatch: expected {:?}, got {:?}",
                expected_ports, report.apu_io_activity.apu_to_cpu_ports
            ));
        }

        for expectation in &expected_apu.cpu_read_counts {
            match report
                .apu_io_activity
                .cpu_read_counts
                .iter()
                .find(|count| count.register == expectation.register)
            {
                Some(actual) if !expectation.count.matches(actual.count) => reasons.push(format!(
                    "APU read count `{}` mismatch at ${:04X}: expected {:?}, got {}",
                    expectation.name, expectation.register, expectation.count, actual.count
                )),
                None => reasons.push(format!(
                    "missing APU read count for `${:04X}`",
                    expectation.register
                )),
                _ => {}
            }
        }

        for expectation in &expected_apu.cpu_write_counts {
            match report
                .apu_io_activity
                .cpu_write_counts
                .iter()
                .find(|count| count.register == expectation.register)
            {
                Some(actual) if !expectation.count.matches(actual.count) => reasons.push(format!(
                    "APU write count `{}` mismatch at ${:04X}: expected {:?}, got {}",
                    expectation.name, expectation.register, expectation.count, actual.count
                )),
                None => reasons.push(format!(
                    "missing APU write count for `${:04X}`",
                    expectation.register
                )),
                _ => {}
            }
        }
    }

    if let Some(expected_ppu) = &expected.ppu_write_activity {
        if let Some(min_total) = expected_ppu.min_total_writes
            && report.ppu_write_activity.total_writes < min_total
        {
            reasons.push(format!(
                "PPU total writes below minimum: expected at least {}, got {}",
                min_total, report.ppu_write_activity.total_writes
            ));
        }

        if let Some(min_visible) = expected_ppu.min_visible_display_write_count
            && report.ppu_write_activity.visible_display_write_count < min_visible
        {
            reasons.push(format!(
                "PPU visible-display writes below minimum: expected at least {}, got {}",
                min_visible, report.ppu_write_activity.visible_display_write_count
            ));
        }

        for register in &expected_ppu.required_visible_registers_touched {
            if !report
                .ppu_write_activity
                .visible_display_registers_touched
                .contains(register)
            {
                reasons.push(format!(
                    "PPU register `${register:04X}` was not touched during the run"
                ));
            }
        }
    }
}

fn build_report(
    name: &str,
    rom: &Path,
    frames: u32,
    emulator: &mut Emulator,
    wram_probes: &[ByteProbeExpectation],
    mmio_probes: &[MmioProbeExpectation],
    pixel_samples: &[PixelSampleExpectation],
    trace: Option<TraceArtifactMetadata>,
) -> CommercialRunReport {
    let mut observed_wram_probes = Vec::with_capacity(wram_probes.len());
    for probe in wram_probes {
        observed_wram_probes.push(ObservedByteProbe {
            name: probe.name.clone(),
            address: probe.address,
            value: emulator.host_read_u8(probe.address),
        });
    }

    let mut observed_mmio_probes = Vec::with_capacity(mmio_probes.len());
    for probe in mmio_probes {
        observed_mmio_probes.push(ObservedMmioProbe {
            name: probe.name.clone(),
            register: probe.register,
            value: emulator.peek_ppu_register(probe.register).unwrap_or(0),
        });
    }

    let sampled_pixels = pixel_samples
        .iter()
        .map(|sample| PixelSampleExpectation {
            x: sample.x,
            y: sample.y,
            rgba: pixel_rgba(emulator.framebuffer(), sample.x, sample.y),
        })
        .collect();
    CommercialRunReport {
        name: name.to_owned(),
        rom: rom.display().to_string(),
        frames_requested: frames,
        frame_counter: emulator.timing().frame,
        cpu_pc: emulator.cpu_registers().pc,
        cpu_pbr: emulator.cpu_registers().pbr,
        frame_hash: framebuffer_hash(emulator.framebuffer()),
        first_pixel_rgba: pixel_rgba(emulator.framebuffer(), 0, 0),
        apu_steps: emulator.apu_status().spc700_steps,
        wram_probes: observed_wram_probes,
        mmio_probes: observed_mmio_probes,
        apu_io_activity: collect_apu_io_activity(emulator),
        ppu_write_activity: collect_ppu_write_activity(emulator),
        sampled_pixels,
        trace,
    }
}

fn run_emulator_for_frames(
    emulator: &mut Emulator,
    frames: u32,
    capture_trace: bool,
    trace_start_frame: u32,
) -> Result<Option<Vec<InstructionTraceRecord>>> {
    // Match the core's deterministic frame guard so a broken commercial ROM
    // cannot hang evidence recording (including instruction-trace capture).
    const MAX_FRAME_INSTRUCTIONS: usize = 20_000;
    let mut trace_records = capture_trace.then(Vec::new);
    for frame_index in 0..frames {
        let _ = emulator.audio_samples();
        if !capture_trace || frame_index < trace_start_frame {
            emulator.run_until_frame()?;
            continue;
        }
        let start_frame = emulator.timing().frame;
        let mut instructions = 0_usize;
        while emulator.timing().frame == start_frame {
            if instructions >= MAX_FRAME_INSTRUCTIONS {
                let registers = emulator.cpu_registers();
                return Err(Error::FrameStalled {
                    frame: start_frame,
                    instructions,
                    elapsed_ms: 0,
                    pc: (u32::from(registers.pbr) << 16) | u32::from(registers.pc),
                });
            }
            let frame = emulator.timing().frame;
            let pbr = emulator.cpu_registers().pbr;
            let pc = emulator.cpu_registers().pc;
            let bus_events = emulator.step_instruction_with_trace()?;
            instructions += 1;
            if let Some(records) = &mut trace_records {
                // Interrupt service may begin with stack writes, not an
                // opcode fetch; never label that data as executable code.
                let opcode = bus_events.first().and_then(|event| {
                    let instruction_address = (u32::from(pbr) << 16) | u32::from(pc);
                    (event.access == AccessKind::Read && event.address == instruction_address)
                        .then_some(event.value)
                });
                let mmio_events = bus_events
                    .into_iter()
                    .filter(|event| is_traceworthy_event(event))
                    .collect();
                records.push(InstructionTraceRecord {
                    frame,
                    pbr,
                    pc,
                    opcode,
                    mmio_events,
                });
            }
        }
        emulator.refresh_framebuffer();
    }
    Ok(trace_records)
}

fn is_traceworthy_event(event: &BusEvent) -> bool {
    if matches!(event.access, AccessKind::Wait) {
        return false;
    }

    let register = (event.address & 0xFFFF) as u16;
    matches!(
        register,
        0x2100..=0x2183 | 0x4016 | 0x4017 | 0x4200..=0x437F
    )
}

fn collect_apu_io_activity(emulator: &Emulator) -> ObservedApuIoActivity {
    let observability = emulator.system_observability();
    let mut cpu_to_apu_ports = [0; 4];
    let mut apu_to_cpu_ports = [0; 4];
    cpu_to_apu_ports.copy_from_slice(emulator.cpu_to_apu_ports());
    apu_to_cpu_ports.copy_from_slice(emulator.apu_to_cpu_ports());

    let cpu_read_counts = (0..4)
        .map(|port| ObservedApuPortCount {
            register: 0x2140 + port as u16,
            count: u64::from(observability.apu_port_read_counts()[port]),
        })
        .collect();
    let cpu_write_counts = (0..4)
        .map(|port| ObservedApuPortCount {
            register: 0x2140 + port as u16,
            count: u64::from(observability.apu_port_write_counts()[port]),
        })
        .collect();

    ObservedApuIoActivity {
        cpu_to_apu_ports,
        apu_to_cpu_ports,
        cpu_read_counts,
        cpu_write_counts,
    }
}

fn collect_ppu_write_activity(emulator: &Emulator) -> ObservedPpuWriteActivity {
    let counts = emulator.system_observability().ppu_write_counts();
    let mut touched_registers = Vec::new();
    let mut visible_display_registers_touched = Vec::new();
    let mut total_writes = 0_u64;
    let mut visible_display_write_count = 0_u64;

    for (index, count) in counts.iter().copied().enumerate() {
        let register = 0x2100 + index as u16;
        total_writes = total_writes.saturating_add(u64::from(count));
        if count > 0 {
            touched_registers.push(register);
            if register <= 0x212C {
                visible_display_registers_touched.push(register);
                visible_display_write_count =
                    visible_display_write_count.saturating_add(u64::from(count));
            }
        }
    }

    ObservedPpuWriteActivity {
        total_writes,
        touched_registers,
        visible_display_write_count,
        visible_display_registers_touched,
    }
}

fn select_default_wram_probes(emulator: &mut Emulator, limit: usize) -> Vec<ByteProbeExpectation> {
    let mut probes = Vec::new();
    for offset in 0..0x20_000_u32 {
        let address = if offset < 0x10000 {
            0x7E0000 + offset
        } else {
            0x7F0000 + (offset - 0x10000)
        };
        let value = emulator.host_read_u8(address);
        if value == 0 {
            continue;
        }

        probes.push(ByteProbeExpectation {
            name: format!("wram_{address:06X}"),
            address,
            expected: value,
        });
        if probes.len() >= limit {
            break;
        }
    }
    probes
}

fn default_mmio_probe_expectations(emulator: &Emulator) -> Vec<MmioProbeExpectation> {
    DEFAULT_MMIO_PROBES
        .iter()
        .filter_map(|(name, register)| {
            emulator
                .peek_ppu_register(*register)
                .map(|value| MmioProbeExpectation {
                    name: (*name).to_owned(),
                    register: *register,
                    expected: value,
                })
        })
        .collect()
}

fn recorded_apu_expectation(report: &ObservedApuIoActivity) -> ApuIoExpectation {
    ApuIoExpectation {
        cpu_to_apu_ports: Some(report.cpu_to_apu_ports),
        apu_to_cpu_ports: Some(report.apu_to_cpu_ports),
        cpu_read_counts: report
            .cpu_read_counts
            .iter()
            .map(|count| ApuPortCountExpectation {
                name: format!("read_${:04X}", count.register),
                register: count.register,
                count: CountExpectation {
                    min: Some(count.count),
                    max: Some(count.count),
                },
            })
            .collect(),
        cpu_write_counts: report
            .cpu_write_counts
            .iter()
            .map(|count| ApuPortCountExpectation {
                name: format!("write_${:04X}", count.register),
                register: count.register,
                count: CountExpectation {
                    min: Some(count.count),
                    max: Some(count.count),
                },
            })
            .collect(),
    }
}

fn recorded_ppu_expectation(report: &ObservedPpuWriteActivity) -> PpuWriteActivityExpectation {
    PpuWriteActivityExpectation {
        required_visible_registers_touched: report.visible_display_registers_touched.clone(),
        min_total_writes: Some(report.total_writes),
        min_visible_display_write_count: Some(report.visible_display_write_count),
    }
}

fn empty_apu_report() -> ObservedApuIoActivity {
    ObservedApuIoActivity {
        cpu_to_apu_ports: [0; 4],
        apu_to_cpu_ports: [0; 4],
        cpu_read_counts: (0..4)
            .map(|port| ObservedApuPortCount {
                register: 0x2140 + port as u16,
                count: 0,
            })
            .collect(),
        cpu_write_counts: (0..4)
            .map(|port| ObservedApuPortCount {
                register: 0x2140 + port as u16,
                count: 0,
            })
            .collect(),
    }
}

fn empty_ppu_report() -> ObservedPpuWriteActivity {
    ObservedPpuWriteActivity {
        total_writes: 0,
        touched_registers: Vec::new(),
        visible_display_write_count: 0,
        visible_display_registers_touched: Vec::new(),
    }
}

fn framebuffer_hash(framebuffer: &crate::ppu::FrameBuffer) -> u64 {
    let mut hasher = DefaultHasher::new();
    framebuffer.width().hash(&mut hasher);
    framebuffer.height().hash(&mut hasher);
    framebuffer.pixels().hash(&mut hasher);
    hasher.finish()
}

fn pixel_rgba(framebuffer: &crate::ppu::FrameBuffer, x: usize, y: usize) -> [u8; 4] {
    if x >= framebuffer.width() || y >= framebuffer.height() {
        return [0, 0, 0, 0];
    }

    let offset = (y * framebuffer.width() + x) * 4;
    let pixels = framebuffer.pixels();
    [
        pixels[offset],
        pixels[offset + 1],
        pixels[offset + 2],
        pixels[offset + 3],
    ]
}

#[derive(Debug, Clone, Deserialize)]
struct RawCommercialFixture {
    name: String,
    rom: PathBuf,
    #[serde(default = "default_frames")]
    frames: u32,
    #[serde(default)]
    controller1: ControllerState,
    #[serde(default)]
    setup_writes: Vec<(u32, u8)>,
    expected: ExpectedCommercialOutcome,
    trace: Option<TraceCaptureOptions>,
}

impl RawCommercialFixture {
    fn into_fixture(self, base_dir: &Path) -> CommercialFixture {
        CommercialFixture {
            name: self.name,
            rom: base_dir.join(self.rom),
            frames: self.frames,
            controller1: self.controller1,
            setup_writes: self.setup_writes,
            expected: self.expected,
            trace: self.trace,
        }
    }
}

const fn default_frames() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use crate::cartridge::Cartridge;
    use crate::emulator::EmulatorBuilder;
    use crate::input::ControllerState;
    use crate::manifest::AssetConfig;

    use super::{
        ByteProbeExpectation, CommercialFixture, ExpectedCommercialOutcome, MmioProbeExpectation,
        PpuWriteActivityExpectation, load_suite, record_fixture, record_fixture_with_trace_start,
        run_emulator_for_frames, run_with_current_core_detailed, run_with_current_core_executed,
        summarize,
    };

    fn write_test_rom(path: &Path) {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE COMMERCIAL  ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x00;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        fs::write(path, rom).unwrap();
    }

    #[test]
    fn summarizes_and_loads_commercial_suite() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);
        fs::write(
            dir.path().join("suite.json"),
            r#"[{
              "name":"commercial boot",
              "rom":"commercial.sfc",
              "frames":1,
              "expected":{
                "frame":1,
                "cpu_pc":32769,
                "cpu_pbr":0,
                "wram_probes":[],
                "mmio_probes":[],
                "pixel_samples":[]
              }
            }]"#,
        )
        .unwrap();

        let summary = summarize(dir.path()).unwrap();
        assert_eq!(summary.vector_count, 1);

        let fixtures = load_suite(dir.path()).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].name, "commercial boot");
    }

    #[test]
    fn record_fixture_captures_probe_and_trace_evidence() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);

        let recorded = record_fixture(
            &rom_path,
            1,
            &AssetConfig::default(),
            ControllerState::default(),
            &[],
            true,
        )
        .unwrap();

        assert_eq!(recorded.fixture.expected.frame, Some(1));
        assert_eq!(recorded.report.frame_counter, 1);
        assert!(
            recorded
                .trace
                .as_ref()
                .is_some_and(|trace| !trace.is_empty())
        );
        assert!(!recorded.fixture.expected.mmio_probes.is_empty());
    }

    #[test]
    fn late_frame_trace_skips_initial_frames_and_replays_consistently() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);

        let recorded = record_fixture_with_trace_start(
            &rom_path,
            3,
            &AssetConfig::default(),
            ControllerState::default(),
            &[],
            true,
            2,
        )
        .unwrap();
        assert_eq!(recorded.report.frame_counter, 3);
        assert_eq!(
            recorded.fixture.trace.as_ref().unwrap().trace_start_frame,
            2
        );
        let recorded_trace = recorded.trace.unwrap();
        assert!(!recorded_trace.is_empty());
        assert!(recorded_trace.iter().all(|entry| entry.frame == 2));

        let executed =
            run_with_current_core_executed(&[recorded.fixture], &AssetConfig::default(), true);
        let replay_trace = executed[0].trace.as_ref().unwrap();
        assert!(!replay_trace.is_empty());
        assert!(replay_trace.iter().all(|entry| entry.frame == 2));
    }

    #[test]
    fn late_frame_trace_rejects_start_after_end() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);

        let error = record_fixture_with_trace_start(
            &rom_path,
            1,
            &AssetConfig::default(),
            ControllerState::default(),
            &[],
            true,
            2,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("trace start frame 2 exceeds requested 1")
        );
    }

    #[test]
    fn interrupt_service_step_has_no_misleading_opcode() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);
        let mut emulator = EmulatorBuilder::new().build();
        emulator.load_rom(Cartridge::load(&rom_path).unwrap());
        // Enable VBlank NMI; servicing it writes the stack before reading the
        // interrupt vector, so the first bus event is not an opcode fetch.
        emulator.host_write_u8(0x004200, 0x80);

        let trace = run_emulator_for_frames(&mut emulator, 1, true, 0)
            .unwrap()
            .unwrap();
        assert!(trace.iter().any(|entry| entry.opcode.is_some()));
        assert!(trace.iter().any(|entry| entry.opcode.is_none()));
        assert!(
            trace
                .iter()
                .any(|entry| entry.opcode.is_none() && entry.frame == 0)
        );
    }

    #[test]
    fn run_current_checks_commercial_expectations() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("commercial.sfc");
        write_test_rom(&rom_path);

        let fixture = CommercialFixture {
            name: "commercial boot".to_owned(),
            rom: rom_path,
            frames: 1,
            controller1: ControllerState::default(),
            setup_writes: vec![],
            expected: ExpectedCommercialOutcome {
                frame: Some(1),
                // The one-frame fixture runs past the initial NOP into the
                // zero-filled BRK loop; 0x8001 is not the end-of-frame PC.
                cpu_pc: Some(0x0000),
                cpu_pc_range: None,
                cpu_pbr: Some(0x00),
                frame_hash: None,
                first_pixel_rgba: Some([0, 0, 0, 255]),
                pixel_samples: Vec::new(),
                wram_probes: vec![ByteProbeExpectation {
                    name: "stack_page".to_owned(),
                    address: 0x7E0100,
                    expected: 0x34,
                }],
                mmio_probes: vec![MmioProbeExpectation {
                    name: "inidisp".to_owned(),
                    register: 0x2100,
                    expected: 0x00,
                }],
                apu_io_activity: None,
                ppu_write_activity: Some(PpuWriteActivityExpectation {
                    required_visible_registers_touched: Vec::new(),
                    min_total_writes: Some(0),
                    min_visible_display_write_count: Some(0),
                }),
            },
            trace: None,
        };

        let evaluations =
            run_with_current_core_detailed(&[fixture], &AssetConfig::default(), false);
        assert_eq!(evaluations.len(), 1);
        assert!(evaluations[0].reasons.is_empty(), "{evaluations:#?}");
    }
}
