use criterion::{Criterion, criterion_group, criterion_main};
use starbyte_core::Emulator;

fn benchmark_step_instruction(criterion: &mut Criterion) {
    criterion.bench_function("emulator_step_without_rom_error", |bench| {
        let mut emulator = Emulator::default();
        bench.iter(|| {
            let _ = emulator.step_instruction();
        });
    });
}

criterion_group!(benches, benchmark_step_instruction);
criterion_main!(benches);
