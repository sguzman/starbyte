//! ROM-based regression suite loading and execution.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cartridge::Cartridge;
use crate::emulator::EmulatorBuilder;
use crate::error::{Error, Result};
use crate::manifest::AssetConfig;

use super::{RunSummary, SuiteSummary, VectorFailure};

/// One host-assisted ROM regression case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFixture {
    /// Human-readable fixture name.
    pub name: String,
    /// ROM path relative to the suite file.
    pub rom: PathBuf,
    /// Number of frames to execute.
    pub frames: u32,
    /// Host writes applied after ROM load and before execution.
    pub setup_writes: Vec<(u32, u8)>,
    /// Host reads verified after execution completes.
    pub expected_reads: Vec<(u32, u8)>,
    /// Expected post-run results.
    pub expected: ExpectedOutcome,
}

/// Expected result from a ROM regression fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedOutcome {
    /// Expected frame counter after execution.
    pub frame: Option<u64>,
    /// Expected CPU program counter after execution.
    pub cpu_pc: Option<u16>,
    /// Expected framebuffer hash.
    pub frame_hash: Option<u64>,
    /// Expected save-RAM length.
    pub save_ram_len: Option<usize>,
    /// Minimum APU step count.
    pub min_apu_steps: Option<u64>,
    /// Optional first pixel RGBA value.
    pub first_pixel_rgba: Option<[u8; 4]>,
}

/// Load every ROM regression file in a suite directory.
pub fn load_suite(dir: impl AsRef<Path>) -> Result<Vec<RomFixture>> {
    let dir = dir.as_ref();
    let mut fixtures = Vec::new();
    for path in discover_suite_files(dir) {
        let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
        let rows: Vec<RawFixture> = serde_json::from_str(&raw)?;
        fixtures.extend(
            rows.into_iter()
                .map(|row| row.into_fixture(path.parent().unwrap_or(dir))),
        );
    }
    Ok(fixtures)
}

/// Summarize a ROM regression suite directory.
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
        suite_name: "ROM regression",
        file_count: files.len(),
        vector_count,
    })
}

/// Execute the suite against the current in-tree emulator.
#[must_use]
pub fn run_with_current_core(
    fixtures: &[RomFixture],
    assets: &AssetConfig,
    max_failures: usize,
) -> RunSummary {
    let mut passed = 0;
    let mut failures = Vec::new();

    for fixture in fixtures {
        let reasons = evaluate_fixture(fixture, assets);
        if reasons.is_empty() {
            passed += 1;
            continue;
        }

        if failures.len() < max_failures {
            failures.push(VectorFailure {
                label: fixture.name.clone(),
                reasons,
            });
        }
    }

    RunSummary {
        suite_name: "ROM regression",
        total: fixtures.len(),
        passed,
        failed: fixtures.len().saturating_sub(passed),
        failures,
    }
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
            matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("json")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn evaluate_fixture(fixture: &RomFixture, assets: &AssetConfig) -> Vec<String> {
    let mut reasons = Vec::new();

    let cartridge = match Cartridge::load(&fixture.rom) {
        Ok(cartridge) => cartridge,
        Err(error) => return vec![error.to_string()],
    };

    let mut emulator = EmulatorBuilder::new().assets(assets.clone()).build();
    let _ = emulator.load_apu_ipl_rom();
    emulator.load_rom(cartridge);

    for (address, value) in &fixture.setup_writes {
        emulator.host_write_u8(*address, *value);
    }

    for _ in 0..fixture.frames {
        if let Err(error) = emulator.run_until_frame() {
            return vec![error.to_string()];
        }
    }

    for (address, expected) in &fixture.expected_reads {
        let actual = emulator.host_read_u8(*address);
        if actual != *expected {
            reasons.push(format!(
                "host read mismatch at 0x{address:06X}: expected 0x{expected:02X}, got 0x{actual:02X}"
            ));
        }
    }

    if let Some(expected_frame) = fixture.expected.frame
        && emulator.timing().frame != expected_frame
    {
        reasons.push(format!(
            "frame mismatch: expected {}, got {}",
            expected_frame,
            emulator.timing().frame
        ));
    }

    if let Some(expected_pc) = fixture.expected.cpu_pc
        && emulator.cpu_registers().pc != expected_pc
    {
        reasons.push(format!(
            "cpu pc mismatch: expected 0x{expected_pc:04X}, got 0x{:04X}",
            emulator.cpu_registers().pc
        ));
    }

    if let Some(expected_hash) = fixture.expected.frame_hash {
        let actual_hash = framebuffer_hash(emulator.framebuffer());
        if actual_hash != expected_hash {
            reasons.push(format!(
                "frame hash mismatch: expected 0x{expected_hash:016X}, got 0x{actual_hash:016X}"
            ));
        }
    }

    if let Some(expected_len) = fixture.expected.save_ram_len {
        let actual_len = emulator.save_ram().map_or(0, |bytes| bytes.len());
        if actual_len != expected_len {
            reasons.push(format!(
                "save RAM length mismatch: expected {}, got {}",
                expected_len, actual_len
            ));
        }
    }

    if let Some(expected_steps) = fixture.expected.min_apu_steps {
        let actual = emulator.apu_status().spc700_steps;
        if actual < expected_steps {
            reasons.push(format!(
                "APU steps below minimum: expected at least {}, got {}",
                expected_steps, actual
            ));
        }
    }

    if let Some(expected_pixel) = fixture.expected.first_pixel_rgba {
        let pixels = emulator.framebuffer().pixels();
        let actual = [pixels[0], pixels[1], pixels[2], pixels[3]];
        if actual != expected_pixel {
            reasons.push(format!(
                "first pixel mismatch: expected {:?}, got {:?}",
                expected_pixel, actual
            ));
        }
    }

    reasons
}

fn framebuffer_hash(framebuffer: &crate::ppu::FrameBuffer) -> u64 {
    let mut hasher = DefaultHasher::new();
    framebuffer.width().hash(&mut hasher);
    framebuffer.height().hash(&mut hasher);
    framebuffer.pixels().hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, Deserialize)]
struct RawFixture {
    name: String,
    rom: PathBuf,
    #[serde(default = "default_frames")]
    frames: u32,
    #[serde(default)]
    setup_writes: Vec<(u32, u8)>,
    #[serde(default)]
    expected_reads: Vec<(u32, u8)>,
    expected: RawExpectedOutcome,
}

#[derive(Debug, Clone, Deserialize)]
struct RawExpectedOutcome {
    frame: Option<u64>,
    cpu_pc: Option<u16>,
    frame_hash: Option<u64>,
    save_ram_len: Option<usize>,
    min_apu_steps: Option<u64>,
    first_pixel_rgba: Option<[u8; 4]>,
}

impl RawFixture {
    fn into_fixture(self, base_dir: &Path) -> RomFixture {
        RomFixture {
            name: self.name,
            rom: base_dir.join(self.rom),
            frames: self.frames,
            setup_writes: self.setup_writes,
            expected_reads: self.expected_reads,
            expected: ExpectedOutcome {
                frame: self.expected.frame,
                cpu_pc: self.expected.cpu_pc,
                frame_hash: self.expected.frame_hash,
                save_ram_len: self.expected.save_ram_len,
                min_apu_steps: self.expected.min_apu_steps,
                first_pixel_rgba: self.expected.first_pixel_rgba,
            },
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
    use crate::emulator::Emulator;
    use crate::manifest::AssetConfig;

    use super::{
        discover_suite_files, framebuffer_hash, load_suite, run_with_current_core, summarize,
    };

    fn write_test_rom(path: &Path) {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE REGRESSION  ");
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

    fn write_coprocessor_rom(
        path: &Path,
        title: &[u8; 21],
        rom_type: u8,
        ram_size_code: u8,
        program: &[(usize, u8)],
    ) {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(title);
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = rom_type;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = ram_size_code;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        rom[0x0000] = 0xEA;
        for (offset, value) in program {
            rom[*offset] = *value;
        }
        fs::write(path, rom).unwrap();
    }

    #[test]
    fn summarizes_and_runs_synthetic_regression_suite() {
        let dir = tempdir().unwrap();
        let rom_path = dir.path().join("case.sfc");
        write_test_rom(&rom_path);

        let expected_hash = {
            let mut emulator = Emulator::default();
            emulator.load_rom(Cartridge::load(&rom_path).unwrap());
            emulator.host_write_u8(0x002121, 0x00);
            emulator.host_write_u8(0x002122, 0x00);
            emulator.host_write_u8(0x002122, 0x7C);
            emulator.host_write_u8(0x00212C, 0x01);
            emulator.run_until_frame().unwrap();
            framebuffer_hash(emulator.framebuffer())
        };

        fs::write(
            dir.path().join("suite.json"),
            format!(
                r#"[{{
                  "name":"ppu backdrop",
                  "rom":"case.sfc",
                  "frames":1,
                  "setup_writes":[[8481,0],[8482,0],[8482,124],[8492,1]],
                  "expected_reads":[],
                  "expected":{{
                    "frame":1,
                    "frame_hash":{expected_hash},
                    "save_ram_len":2048,
                    "min_apu_steps":1,
                    "first_pixel_rgba":[248,0,0,255]
                  }}
                }}]"#
            ),
        )
        .unwrap();

        let summary = summarize(dir.path()).unwrap();
        assert_eq!(summary.file_count, 1);
        assert_eq!(summary.vector_count, 1);
        assert_eq!(discover_suite_files(dir.path()).len(), 1);

        let fixtures = load_suite(dir.path()).unwrap();
        let run = run_with_current_core(&fixtures, &AssetConfig::default(), 8);
        assert_eq!(run.failed, 0);
        assert_eq!(run.passed, 1);
    }

    #[test]
    fn runs_dsp_variant_and_superfx_regression_suite() {
        let dir = tempdir().unwrap();

        let dsp1 = dir.path().join("dsp1.sfc");
        let dsp1b = dir.path().join("dsp1b.sfc");
        let dsp2 = dir.path().join("dsp2.sfc");
        let dsp3 = dir.path().join("dsp3.sfc");
        let dsp4 = dir.path().join("dsp4.sfc");
        let superfx = dir.path().join("superfx.sfc");
        let sa1 = dir.path().join("sa1.sfc");
        let cx4 = dir.path().join("cx4.sfc");
        let sdd1 = dir.path().join("sdd1.sfc");
        let obc1 = dir.path().join("obc1.sfc");
        let srtc = dir.path().join("srtc.sfc");

        write_coprocessor_rom(&dsp1, b"STARBYTE DSP-1 TEST  ", 0x03, 0x00, &[]);
        write_coprocessor_rom(&dsp1b, b"STARBYTE DSP-1B TEST ", 0x03, 0x00, &[]);
        write_coprocessor_rom(&dsp2, b"STARBYTE DSP-2 TEST  ", 0x03, 0x00, &[]);
        write_coprocessor_rom(&dsp3, b"STARBYTE DSP-3 TEST  ", 0x03, 0x00, &[]);
        write_coprocessor_rom(&dsp4, b"STARBYTE DSP-4 TEST  ", 0x03, 0x00, &[]);
        write_coprocessor_rom(&sa1, b"STARBYTE SA-1 TEST   ", 0x34, 0x01, &[]);
        write_coprocessor_rom(&cx4, b"STARBYTE CX4 TEST    ", 0xF3, 0x01, &[]);
        write_coprocessor_rom(&sdd1, b"STARBYTE S-DD1 TEST  ", 0x43, 0x00, &[]);
        write_coprocessor_rom(&obc1, b"STARBYTE OBC1 TEST   ", 0x23, 0x00, &[]);
        write_coprocessor_rom(&srtc, b"STARBYTE SRTC TEST   ", 0x53, 0x00, &[]);
        write_coprocessor_rom(
            &superfx,
            b"STARBYTE SUPERFX ROM ",
            0x13,
            0x00,
            &[
                (0x0020, 0xFE), (0x0021, 0x40), (0x0022, 0x00),
                (0x0023, 0xDF),
                (0x0024, 0x4C),
                (0x0025, 0x00),
                (0x0040, 0x33),
            ],
        );

        let expected_superfx_hash = {
            let mut emulator = Emulator::default();
            emulator.load_rom(Cartridge::load(&superfx).unwrap());
            emulator.host_write_u8(0x00301E, 0x20);
            emulator.host_write_u8(0x00301F, 0x00);
            emulator.run_until_frame().unwrap();
            framebuffer_hash(emulator.framebuffer())
        };

        fs::write(
            dir.path().join("suite.json"),
            format!(
                r#"[{{
                  "name":"dsp1 multiply",
                  "rom":"dsp1.sfc",
                  "frames":1,
                  "setup_writes":[[3178496,0],[3178497,0],[3178496,0],[3178497,64],[3178496,0],[3178497,64]],
                  "expected_reads":[[3194880,192],[3178496,0],[3178497,32],[3194880,128]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"dsp1b dump",
                  "rom":"dsp1b.sfc",
                  "frames":1,
                  "setup_writes":[[3178496,31],[3178497,0],[3178496,52],[3178497,18]],
                  "expected_reads":[[3178496,62],[3178497,139]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"dsp2 multiply2",
                  "rom":"dsp2.sfc",
                  "frames":1,
                  "setup_writes":[[3178496,32],[3178497,0],[3178496,0],[3178497,64],[3178496,0],[3178497,64]],
                  "expected_reads":[[3178496,1],[3178497,32]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"dsp3 dump",
                  "rom":"dsp3.sfc",
                  "frames":1,
                  "setup_writes":[[3178496,31],[3178497,0],[3178496,52],[3178497,18]],
                  "expected_reads":[[3178496,15],[3178497,186]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"dsp4 distance",
                  "rom":"dsp4.sfc",
                  "frames":1,
                  "setup_writes":[[3178496,40],[3178497,0],[3178496,3],[3178497,0],[3178496,4],[3178497,0],[3178496,12],[3178497,0]],
                  "expected_reads":[[3178496,13],[3178497,0]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"sa1 boot baseline",
                  "rom":"sa1.sfc",
                  "frames":1,
                  "setup_writes":[[8706,120],[8707,86],[8710,8],[8712,85],[8714,1],[8704,128],[12288,170],[4210688,204]],
                  "expected_reads":[[8705,192],[8713,136],[8716,120],[8717,86],[12288,170],[4210688,204]],
                  "expected":{{"frame":1,"save_ram_len":2048,"min_apu_steps":1}}
                }},{{
                  "name":"cx4 length",
                  "rom":"cx4.sfc",
                  "frames":1,
                  "setup_writes":[[24576,3],[24577,0],[24578,4],[24579,0],[24580,12],[24581,0],[32576,16]],
                  "expected_reads":[[24592,13],[24593,0],[32577,129]],
                  "expected":{{"frame":1,"save_ram_len":2048,"min_apu_steps":1}}
                }},{{
                  "name":"sdd1 decompress",
                  "rom":"sdd1.sfc",
                  "frames":1,
                  "setup_writes":[[18432,1],[18433,1],[18436,130],[18436,65]],
                  "expected_reads":[[18438,129],[18437,65],[18437,65],[18437,65]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"obc1 object window",
                  "rom":"obc1.sfc",
                  "frames":1,
                  "setup_writes":[[32758,5],[32752,18],[32753,52]],
                  "expected_reads":[[32752,18],[32753,52]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"srtc latched digits",
                  "rom":"srtc.sfc",
                  "frames":1,
                  "setup_writes":[[10241,14],[10241,15]],
                  "expected_reads":[[10240,1],[10240,0]],
                  "expected":{{"frame":1,"save_ram_len":0,"min_apu_steps":1}}
                }},{{
                  "name":"superfx overlay",
                  "rom":"superfx.sfc",
                  "frames":1,
                  "setup_writes":[[12318,32],[12319,0]],
                  "expected_reads":[[12337,128]],
                  "expected":{{"frame":1,"frame_hash":{expected_superfx_hash},"save_ram_len":0,"min_apu_steps":1,"first_pixel_rgba":[255,0,255,255]}}
                }}]"#
            ),
        )
        .unwrap();

        let fixtures = load_suite(dir.path()).unwrap();
        let run = run_with_current_core(&fixtures, &AssetConfig::default(), 8);
        assert_eq!(run.failed, 0, "{run:#?}");
        assert_eq!(run.passed, 11);
    }
}
