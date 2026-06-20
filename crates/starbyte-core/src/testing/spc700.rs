//! SPC700 compliance vector loading.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::bus::AccessKind;
use crate::error::{Error, Result};

use crate::apu::spc700::Spc700;

use super::{RunSummary, SuiteSummary, VectorFailure};

/// Sparse RAM byte used by SPC700 vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryByte {
    /// 16-bit SPC700 address space.
    pub address: u16,
    /// Value at the address.
    pub value: u8,
}

/// Initial/final SPC700 state encoded by the suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuState {
    /// Program counter.
    pub pc: u16,
    /// Accumulator.
    pub a: u8,
    /// X register.
    pub x: u8,
    /// Y register.
    pub y: u8,
    /// Stack pointer.
    pub sp: u8,
    /// Processor status.
    pub psw: u8,
    /// Sparse RAM bytes touched by the vector.
    pub ram: Vec<MemoryByte>,
}

/// A bus-cycle expectation from the SPC700 corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleExpectation {
    /// Address on the bus.
    pub address: Option<u16>,
    /// Optional bus value.
    pub value: Option<u8>,
    /// Access direction.
    pub access: AccessKind,
}

/// One SPC700 single-step vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestVector {
    /// Corpus-provided name if any.
    pub name: Option<String>,
    /// Opcode file this vector came from.
    pub opcode: u8,
    /// Initial machine state.
    pub initial: CpuState,
    /// Expected post-step machine state.
    pub final_state: CpuState,
    /// Expected bus cycles.
    pub cycles: Vec<CycleExpectation>,
}

/// Load all vectors for a specific SPC700 opcode.
pub fn load_opcode_file(dir: impl AsRef<Path>, opcode: u8) -> Result<Vec<TestVector>> {
    let path = dir.as_ref().join(format!("{opcode:02x}.json"));
    let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
    let raw_vectors: Vec<RawVector> = serde_json::from_str(&raw)?;

    raw_vectors
        .into_iter()
        .map(|vector| vector.into_vector(opcode))
        .collect()
}

/// Summarize an on-disk SPC700 suite directory.
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

        file_count += 1;
        let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
        let vectors: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
        vector_count += vectors.len();
    }

    Ok(SuiteSummary {
        suite_name: "SPC700",
        file_count,
        vector_count,
    })
}

/// Execute vectors against the current in-tree `SPC700` core implementation.
#[must_use]
pub fn run_with_current_core(vectors: &[TestVector], max_failures: usize) -> RunSummary {
    let mut cpu = Spc700::default();
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
        suite_name: "SPC700",
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
            matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("json")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[derive(Debug, Clone, Deserialize)]
struct RawState {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    psw: u8,
    ram: Vec<[u16; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawVector {
    #[serde(default)]
    name: Option<String>,
    initial: RawState,
    #[serde(rename = "final")]
    final_state: RawState,
    cycles: Vec<(Option<u16>, Option<u8>, String)>,
}

impl RawState {
    fn into_state(self) -> CpuState {
        CpuState {
            pc: self.pc,
            a: self.a,
            x: self.x,
            y: self.y,
            sp: self.sp,
            psw: self.psw,
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
    fn into_vector(self, opcode: u8) -> Result<TestVector> {
        let cycles = self
            .cycles
            .into_iter()
            .map(|(address, value, access)| {
                let access = match access.as_str() {
                    "read" => AccessKind::Read,
                    "write" => AccessKind::Write,
                    "wait" => AccessKind::Wait,
                    _ => {
                        return Err(Error::InvalidRom(format!(
                            "unsupported SPC700 cycle access `{access}`"
                        )));
                    }
                };
                Ok(CycleExpectation {
                    address,
                    value,
                    access,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(TestVector {
            name: self.name,
            opcode,
            initial: self.initial.into_state(),
            final_state: self.final_state.into_state(),
            cycles,
        })
    }
}

fn evaluate_vector(cpu: &mut Spc700, vector: &TestVector) -> Vec<String> {
    let memory = RefCell::new(SparseMemory::new(&vector.initial.ram));
    cpu.load_state(
        vector.initial.pc,
        vector.initial.a,
        vector.initial.x,
        vector.initial.y,
        vector.initial.sp,
        vector.initial.psw,
    );
    let trace = match cpu.step_with_memory(
        |address| memory.borrow().read(address),
        |address, value| {
            memory.borrow_mut().write(address, value);
        },
    ) {
        Ok(trace) => trace,
        Err(error) => return vec![error.to_string()],
    };

    let mut reasons = Vec::new();

    let actual_state = CpuState {
        pc: cpu.pc,
        a: cpu.a,
        x: cpu.x,
        y: cpu.y,
        sp: cpu.sp,
        psw: cpu.psw,
        ram: Vec::new(),
    };
    let mut expected_state = vector.final_state.clone();
    expected_state.ram.clear();
    if actual_state != expected_state {
        reasons.push(format!(
            "register mismatch: expected {:?}, saw {:?}",
            expected_state, actual_state
        ));
    }

    for expected in &vector.final_state.ram {
        let actual = memory.borrow().read(expected.address);
        if actual != expected.value {
            reasons.push(format!(
                "RAM mismatch at 0x{:04X}: expected {:02X}, saw {:02X}",
                expected.address, expected.value, actual
            ));
        }
    }

    if trace.len() != vector.cycles.len() {
        reasons.push(format!(
            "cycle mismatch: expected {}, saw {}",
            vector.cycles.len(),
            trace.len()
        ));
    } else {
        for (actual, expected) in trace.iter().zip(&vector.cycles) {
            if actual.access != expected.access {
                reasons.push(format!(
                    "trace mismatch at cycle {}: expected {:?}, saw {:?}",
                    actual.cycle, expected.access, actual.access
                ));
                break;
            }

            if expected.access != AccessKind::Wait {
                if expected.address.map(u32::from) != Some(actual.address) {
                    reasons.push(format!(
                        "trace address mismatch at cycle {}: expected 0x{:04X}, saw 0x{:04X}",
                        actual.cycle,
                        expected.address.unwrap_or_default(),
                        actual.address
                    ));
                    break;
                }
            }

            if let Some(value) = expected.value {
                if actual.value != value {
                    reasons.push(format!(
                        "trace value mismatch at cycle {}: expected {:02X}, saw {:02X}",
                        actual.cycle, value, actual.value
                    ));
                    break;
                }
            }
        }
    }

    reasons
}

fn vector_label(vector: &TestVector) -> String {
    match &vector.name {
        Some(name) if !name.is_empty() => format!("{} (opcode 0x{:02X})", name, vector.opcode),
        _ => format!("opcode 0x{:02X}", vector.opcode),
    }
}

#[derive(Debug, Default)]
struct SparseMemory {
    bytes: HashMap<u16, u8>,
}

impl SparseMemory {
    fn new(initial: &[MemoryByte]) -> Self {
        let mut bytes = HashMap::with_capacity(initial.len());
        for byte in initial {
            bytes.insert(byte.address, byte.value);
        }
        Self { bytes }
    }

    fn read(&self, address: u16) -> u8 {
        self.bytes.get(&address).copied().unwrap_or(0)
    }

    fn write(&mut self, address: u16, value: u8) {
        self.bytes.insert(address, value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::bus::AccessKind;

    use super::{
        CpuState, CycleExpectation, MemoryByte, TestVector, discover_suite_files, load_opcode_file,
        run_with_current_core, summarize,
    };

    #[test]
    fn loads_single_opcode_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("00.json");
        fs::write(
            &path,
            r#"[{
              "name":"nop smoke",
              "initial":{"pc":4096,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[[4096,0]]},
              "final":{"pc":4097,"a":1,"x":2,"y":3,"sp":255,"psw":18,"ram":[[4097,0]]},
              "cycles":[[4096,0,"read"],[4097,0,"write"]]
            }]"#,
        )
        .unwrap();

        let vectors = load_opcode_file(dir.path(), 0x00).unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].opcode, 0x00);
        assert_eq!(vectors[0].cycles.len(), 2);
    }

    #[test]
    fn summarizes_suite_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("00.json"), "[]").unwrap();
        fs::write(dir.path().join("01.json"), "[]").unwrap();

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
            opcode: 0x00,
            initial: CpuState {
                pc: 0x1000,
                a: 1,
                x: 2,
                y: 3,
                sp: 0xFF,
                psw: 0x12,
                ram: vec![MemoryByte {
                    address: 0x1000,
                    value: 0x00,
                }],
            },
            final_state: CpuState {
                pc: 0x1001,
                a: 1,
                x: 2,
                y: 3,
                sp: 0xFF,
                psw: 0x12,
                ram: vec![MemoryByte {
                    address: 0x1000,
                    value: 0x00,
                }],
            },
            cycles: vec![
                CycleExpectation {
                    address: Some(0x1000),
                    value: Some(0),
                    access: AccessKind::Read,
                },
                CycleExpectation {
                    address: Some(0x1001),
                    value: None,
                    access: AccessKind::Read,
                },
            ],
        };

        let summary = run_with_current_core(&[vector], 4);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 0);
    }
}
