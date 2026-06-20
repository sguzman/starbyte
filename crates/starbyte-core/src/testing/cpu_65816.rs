//! 65816 compliance vector loading.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bus::{AccessKind, Address};
use crate::cpu_65816::registers::Registers;
use crate::error::{Error, Result};

use crate::cpu_65816::Cpu65816;

use super::{RunSummary, SuiteSummary, VectorFailure};

/// Execution mode encoded in the 65816 test corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Emulation mode vectors.
    Emulation,
    /// Native mode vectors.
    Native,
}

impl Mode {
    /// Suffix used by the common vector corpus file naming.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Emulation => "e",
            Self::Native => "n",
        }
    }
}

/// A memory write/read expectation encoded by the suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryByte {
    /// Address touched by the vector.
    pub address: Address,
    /// Value at the address.
    pub value: u8,
}

/// Initial/final 65816 state encoded by the suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuState {
    /// Register file snapshot.
    pub registers: Registers,
    /// Sparse RAM bytes relevant to the vector.
    pub ram: Vec<MemoryByte>,
}

/// A bus-cycle expectation from the vector corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleExpectation {
    /// Address on the bus.
    pub address: Address,
    /// Optional bus value.
    pub value: Option<u8>,
    /// Access direction.
    pub access: AccessKind,
    /// Opaque status text from the source corpus.
    pub status: String,
}

/// One 65816 single-step vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestVector {
    /// Corpus-provided name if any.
    pub name: Option<String>,
    /// Opcode file this vector came from.
    pub opcode: u8,
    /// Execution mode.
    pub mode: Mode,
    /// Initial machine state.
    pub initial: CpuState,
    /// Expected post-step machine state.
    pub final_state: CpuState,
    /// Expected bus cycles.
    pub cycles: Vec<CycleExpectation>,
}

/// Load all vectors for a specific opcode and mode.
pub fn load_opcode_file(dir: impl AsRef<Path>, opcode: u8, mode: Mode) -> Result<Vec<TestVector>> {
    let path = dir
        .as_ref()
        .join(format!("{opcode:02x}.{}.json", mode.suffix()));
    let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
    let raw_vectors: Vec<RawVector> = serde_json::from_str(&raw)?;

    raw_vectors
        .into_iter()
        .map(|vector| vector.into_vector(opcode, mode))
        .collect()
}

/// Summarize an on-disk 65816 suite directory.
pub fn summarize(dir: impl AsRef<Path>) -> Result<SuiteSummary> {
    let dir = dir.as_ref();
    let mut file_count = 0;
    let mut vector_count = 0;

    for entry in fs::read_dir(dir).map_err(|source| Error::io(dir, source))? {
        let entry = entry.map_err(|source| Error::io(dir, source))?;
        let path = entry.path();
        if !matches!(
            path.extension().and_then(std::ffi::OsStr::to_str),
            Some("json")
        ) {
            continue;
        }

        let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if !name.ends_with(".e.json") && !name.ends_with(".n.json") {
            continue;
        }

        file_count += 1;
        let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
        let vectors: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
        vector_count += vectors.len();
    }

    Ok(SuiteSummary {
        suite_name: "65816",
        file_count,
        vector_count,
    })
}

/// Execute vectors against the current in-tree 65816 core implementation.
#[must_use]
pub fn run_with_current_core(vectors: &[TestVector], max_failures: usize) -> RunSummary {
    let mut cpu = Cpu65816::default();
    let mut passed = 0;
    let mut failures = Vec::new();

    for vector in vectors {
        let reasons = evaluate_vector(&mut cpu, vector);
        if reasons.is_empty() {
            passed += 1;
            continue;
        }

        if failures.len() < max_failures {
            failures.push(VectorFailure {
                label: vector_label(vector),
                reasons,
            });
        }
    }

    RunSummary {
        suite_name: "65816",
        total: vectors.len(),
        passed,
        failed: vectors.len().saturating_sub(passed),
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
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                return false;
            };
            name.ends_with(".e.json") || name.ends_with(".n.json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[derive(Debug, Clone, Deserialize)]
struct RawState {
    a: u16,
    x: u16,
    y: u16,
    s: u16,
    d: u16,
    pc: u16,
    #[serde(rename = "pbr")]
    program_bank: u8,
    dbr: u8,
    p: u8,
    #[serde(rename = "e")]
    emulation_mode: u8,
    ram: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawVector {
    #[serde(default)]
    name: Option<String>,
    initial: RawState,
    #[serde(rename = "final")]
    final_state: RawState,
    cycles: Vec<(u32, Option<u8>, String)>,
}

impl RawState {
    fn into_state(self) -> CpuState {
        CpuState {
            registers: Registers {
                a: self.a,
                x: self.x,
                y: self.y,
                s: self.s,
                d: self.d,
                pc: self.pc,
                pbr: self.program_bank,
                dbr: self.dbr,
                p: self.p,
                emulation: self.emulation_mode != 0,
            },
            ram: self
                .ram
                .into_iter()
                .map(|[address, value]| MemoryByte {
                    address,
                    value: value as u8,
                })
                .collect(),
        }
    }
}

impl RawVector {
    fn into_vector(self, opcode: u8, mode: Mode) -> Result<TestVector> {
        let cycles = self
            .cycles
            .into_iter()
            .map(|(address, value, status)| {
                let access = match status.chars().nth(3) {
                    Some('r') => AccessKind::Read,
                    Some('w') => AccessKind::Write,
                    _ => {
                        return Err(Error::InvalidRom(format!(
                            "unsupported 65816 cycle status `{status}`"
                        )));
                    }
                };

                Ok(CycleExpectation {
                    address,
                    value,
                    access,
                    status,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(TestVector {
            name: self.name,
            opcode,
            mode,
            initial: self.initial.into_state(),
            final_state: self.final_state.into_state(),
            cycles,
        })
    }
}

fn evaluate_vector(cpu: &mut Cpu65816, vector: &TestVector) -> Vec<String> {
    cpu.load_registers(vector.initial.registers.clone());
    cpu.step();

    let mut reasons = Vec::new();

    if cpu.registers != vector.final_state.registers {
        reasons.push(format!(
            "register mismatch: expected {:?}, saw {:?}",
            vector.final_state.registers, cpu.registers
        ));
    }

    let expected_cycles = vector.cycles.len() as u64;
    if cpu.cycles() != expected_cycles {
        reasons.push(format!(
            "cycle mismatch: expected {expected_cycles}, saw {}",
            cpu.cycles()
        ));
    }

    reasons
}

fn vector_label(vector: &TestVector) -> String {
    match &vector.name {
        Some(name) if !name.is_empty() => format!(
            "{} (opcode 0x{:02X}, {:?})",
            name, vector.opcode, vector.mode
        ),
        _ => format!("opcode 0x{:02X} {:?}", vector.opcode, vector.mode),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::bus::AccessKind;
    use crate::cpu_65816::registers::Registers;

    use super::{
        CpuState, CycleExpectation, Mode, TestVector, discover_suite_files, load_opcode_file,
        run_with_current_core, summarize,
    };

    #[test]
    fn loads_single_opcode_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("00.n.json");
        fs::write(
            &path,
            r#"[{
              "name":"brk smoke",
              "initial":{"a":1,"x":2,"y":3,"s":65535,"d":0,"pc":32768,"pbr":0,"dbr":0,"p":52,"e":0,"ram":[[32768,0]]},
              "final":{"a":1,"x":2,"y":3,"s":65535,"d":0,"pc":32769,"pbr":0,"dbr":0,"p":52,"e":0,"ram":[[32769,1]]},
              "cycles":[[32768,0,"pcrr"],[32769,1,"pcww"]]
            }]"#,
        )
        .unwrap();

        let vectors = load_opcode_file(dir.path(), 0x00, Mode::Native).unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].opcode, 0x00);
        assert_eq!(vectors[0].cycles.len(), 2);
    }

    #[test]
    fn summarizes_suite_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("00.n.json"), "[]").unwrap();
        fs::write(dir.path().join("00.e.json"), "[]").unwrap();
        fs::write(dir.path().join("README.txt"), "ignored").unwrap();

        let summary = summarize(dir.path()).unwrap();
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.vector_count, 0);

        let files = discover_suite_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn placeholder_core_can_match_a_synthetic_vector() {
        let vector = TestVector {
            name: Some("placeholder pass".to_owned()),
            opcode: 0xEA,
            mode: Mode::Native,
            initial: CpuState {
                registers: Registers {
                    pc: 0x1234,
                    pbr: 0,
                    ..Registers::default()
                },
                ram: Vec::new(),
            },
            final_state: CpuState {
                registers: Registers {
                    pc: 0x1235,
                    pbr: 0,
                    ..Registers::default()
                },
                ram: Vec::new(),
            },
            cycles: vec![CycleExpectation {
                address: 0x1234,
                value: Some(0xEA),
                access: AccessKind::Read,
                status: "pcrr".to_owned(),
            }],
        };

        let summary = run_with_current_core(&[vector], 4);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 0);
    }
}
