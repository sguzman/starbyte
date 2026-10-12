//! Independent, MIT-licensed SPC700 single-step reference checks.
//!
//! Fixture source: SingleStepTests/spc700, v1 (attribution and full MIT
//! notice in fixtures/LICENSE-spc700.txt). These tests exercise original
//! instruction CPU state, RAM side effects and per-cycle memory bus activity.
//! No SNES ROM or sound-program binary is bundled.

use std::cell::RefCell;

use serde::Deserialize;
use starbyte_core::{
    apu::spc700::Spc700,
    bus::{AccessKind, BusEvent},
};

#[derive(Debug, Deserialize)]
struct CpuState {
    pc: u16,
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    psw: u8,
    ram: Vec<(u16, u8)>,
}

#[derive(Debug, Deserialize)]
struct ReferenceStep {
    name: String,
    initial: CpuState,
    #[serde(rename = "final")]
    expected: CpuState,
    // Unspecified bus addresses and values are deliberate wildcards in
    // the corpus; comparing them as zero would invent reference evidence.
    cycles: Vec<(Option<u16>, Option<u8>, String)>,
}

fn check_registers(actual: &Spc700, expected: &CpuState, label: &str) {
    assert_eq!(actual.pc, expected.pc, "{label}: PC");
    assert_eq!(actual.a, expected.a, "{label}: A");
    assert_eq!(actual.x, expected.x, "{label}: X");
    assert_eq!(actual.y, expected.y, "{label}: Y");
    assert_eq!(actual.sp, expected.sp, "{label}: SP");
    assert_eq!(actual.psw, expected.psw, "{label}: PSW");
}

fn check_cycle(
    actual: &BusEvent,
    &(address, value, ref kind): &(Option<u16>, Option<u8>, String),
    label: &str,
    index: usize,
) {
    let access = match kind.as_str() {
        "read" => AccessKind::Read,
        "write" => AccessKind::Write,
        "wait" => AccessKind::Wait,
        other => panic!("{label} cycle {index}: unexpected reference access {other}"),
    };
    assert_eq!(actual.access, access, "{label} cycle {index}: access");
    if let Some(address) = address {
        assert_eq!(
            actual.address,
            u32::from(address),
            "{label} cycle {index}: address"
        );
    }
    if let Some(value) = value {
        assert_eq!(actual.value, value, "{label} cycle {index}: data");
    }
}

#[test]
fn spc700_matches_independent_single_step_cpu_memory_and_bus_cases() {
    let mut references: Vec<ReferenceStep> =
        serde_json::from_str(include_str!("fixtures/spc700_singlestep.json")).unwrap();
    for fixture in [
        include_str!("fixtures/spc700_ref_01_08.json"),
        include_str!("fixtures/spc700_ref_09_10.json"),
        include_str!("fixtures/spc700_ref_11_18.json"),
        include_str!("fixtures/spc700_ref_19_20.json"),
        include_str!("fixtures/spc700_ref_21_28.json"),
        include_str!("fixtures/spc700_ref_29_30.json"),
        include_str!("fixtures/spc700_ref_31_38.json"),
        include_str!("fixtures/spc700_ref_39_3e.json"),
        include_str!("fixtures/spc700_ref_40_47.json"),
        include_str!("fixtures/spc700_ref_48_4f.json"),
        include_str!("fixtures/spc700_ref_50_57.json"),
        include_str!("fixtures/spc700_ref_58_5f.json"),
        include_str!("fixtures/spc700_ref_60_67.json"),
        include_str!("fixtures/spc700_ref_68_6e.json"),
        include_str!("fixtures/spc700_ref_70_77.json"),
        include_str!("fixtures/spc700_ref_78_7f.json"),
        include_str!("fixtures/spc700_ref_80_87.json"),
        include_str!("fixtures/spc700_ref_88_8e.json"),
        include_str!("fixtures/spc700_ref_90_97.json"),
        include_str!("fixtures/spc700_ref_98_9f.json"),
        include_str!("fixtures/spc700_ref_a0_a7.json"),
        include_str!("fixtures/spc700_ref_a8_ae.json"),
        include_str!("fixtures/spc700_ref_b0_b7.json"),
        include_str!("fixtures/spc700_ref_b8_bf.json"),
    ] {
        references.extend(serde_json::from_str::<Vec<ReferenceStep>>(fixture).unwrap());
    }
    assert_eq!(references.len(), 1680, "reference fixture truncated");
    for case in references {
        let label = case.name.as_str();
        let mut cpu = Spc700::default();
        cpu.load_state(
            case.initial.pc,
            case.initial.a,
            case.initial.x,
            case.initial.y,
            case.initial.sp,
            case.initial.psw,
        );
        let mut ram = vec![0_u8; 65_536];
        for &(address, value) in &case.initial.ram {
            ram[usize::from(address)] = value;
        }
        let ram = RefCell::new(ram);
        let trace = cpu
            .step_with_memory(
                |address| ram.borrow()[usize::from(address)],
                |address, value| ram.borrow_mut()[usize::from(address)] = value,
            )
            .unwrap_or_else(|error| panic!("{label}: unable to execute: {error}"));

        check_registers(&cpu, &case.expected, label);
        for &(address, expected) in &case.expected.ram {
            assert_eq!(
                ram.borrow()[usize::from(address)],
                expected,
                "{label}: RAM at {address:04X}"
            );
        }
        assert_eq!(trace.len(), case.cycles.len(), "{label}: cycle count");
        for (index, (actual, expected)) in trace.iter().zip(&case.cycles).enumerate() {
            check_cycle(actual, expected, label, index);
        }
    }
}
