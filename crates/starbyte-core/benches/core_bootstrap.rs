use criterion::{Criterion, criterion_group, criterion_main};
use starbyte_core::cartridge::Cartridge;
use starbyte_core::timing::{DOTS_PER_SCANLINE, NTSC_SCANLINES_PER_FRAME, TimingState};
use starbyte_core::Emulator;

fn benchmark_step_instruction_without_rom(criterion: &mut Criterion) {
    criterion.bench_function("emulator_step_without_rom_error", |bench| {
        let mut emulator = Emulator::default();
        bench.iter(|| {
            let _ = emulator.step_instruction();
        });
    });
}

fn benchmark_step_instruction_with_rom(criterion: &mut Criterion) {
    criterion.bench_function("emulator_step_with_rom", |bench| {
        let mut emulator = Emulator::default();
        emulator.load_rom(test_cartridge());
        bench.iter(|| {
            let _ = emulator.step_instruction();
        });
    });
}

fn benchmark_run_until_frame(criterion: &mut Criterion) {
    criterion.bench_function("emulator_run_until_frame", |bench| {
        let mut emulator = Emulator::default();
        emulator.load_rom(test_cartridge());
        bench.iter(|| {
            let _ = emulator.run_until_frame();
        });
    });
}

fn benchmark_timing_advance(criterion: &mut Criterion) {
    criterion.bench_function("timing_full_frame_advance", |bench| {
        let frame_clocks = u64::from(DOTS_PER_SCANLINE) * u64::from(NTSC_SCANLINES_PER_FRAME);
        bench.iter(|| {
            let mut timing = TimingState::default();
            let _ = timing.advance_master_clocks(frame_clocks);
        });
    });
}

fn test_cartridge() -> Cartridge {
    let mut rom = vec![0_u8; 0x10000];
    let base = 0x7FC0;
    rom[base..base + 21].copy_from_slice(b"STARBYTE BENCHMARK   ");
    rom[base + 0x15] = 0x20;
    rom[base + 0x16] = 0x00;
    rom[base + 0x17] = 0x09;
    rom[base + 0x18] = 0x01;
    rom[base + 0x19] = 0x01;
    rom[base + 0x1C] = 0xFF;
    rom[base + 0x1D] = 0xFF;
    rom[base + 0x1E] = 0x00;
    rom[base + 0x1F] = 0x00;
    rom[0x7FFC] = 0x00;
    rom[0x7FFD] = 0x80;
    rom[0x0000] = 0xEA;
    Cartridge::from_bytes(rom, None).expect("benchmark ROM should parse")
}

criterion_group!(
    benches,
    benchmark_step_instruction_without_rom,
    benchmark_step_instruction_with_rom,
    benchmark_run_until_frame,
    benchmark_timing_advance
);
criterion_main!(benches);
