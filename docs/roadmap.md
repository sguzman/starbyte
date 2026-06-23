# Starbyte Roadmap

Starbyte is being bootstrapped as a correctness-first, CLI-first SNES emulator with a reusable core and future modular frontends. Linux is the first shipping target. Windows and `egui` follow after the core reaches a stable, test-backed baseline. Debugging features are intentionally a non-feature for now.

## Milestones

- [x] Workspace and developer foundation
- [x] Cartridge loading and ROM mapping
- [x] 65816 compliance harness
- [x] 65816 execution core
- [x] SPC700 compliance harness and APU bootstrap
- [x] Main SNES memory map and interrupt/timing model
- [x] DMA/HDMA correctness
- [x] PPU register model and frame generation
- [x] ROM-based CPU/APU/PPU regression suite
- [x] CLI usability and artifact management
- [x] Linux playable baseline
- [x] Save RAM and save states
- [x] Performance profiling and hot-path optimization
- [x] Windows support hardening
- [x] `egui` frontend bootstrap
- [x] Additional frontends and advanced user features
- [ ] Coprocessor support
- [ ] Nice-to-have features such as filters, shaders, rewind, movie recording
- [x] Explicit non-feature for now: debugger tooling

## Milestone Notes

### Workspace and developer foundation

- [x] Maintain a Cargo workspace with `starbyte-core` and `starbyte-cli`.
- [x] Keep frontend-facing traits in the core so CLI and future GUIs share the same host boundary.
- [x] Standardize structured logging with subsystem-oriented tracing filters.
- [x] Add linting, tests, integration tests, and benchmark harnesses early.

### Cartridge loading and ROM mapping

- [x] Parse cartridge headers and normalize core metadata.
- [x] Detect LoROM and HiROM cleanly and surface actionable diagnostics for invalid images.
- [x] Add save RAM plumbing and path management in the CLI layer.

### 65816 compliance harness

- [x] Build a local harness interface for single-step JSON vectors.
- [x] Validate registers, memory deltas, cycle counts, and bus traces.
- [x] Make the harness automation-friendly for CI and local iteration.

### 65816 execution core

- [x] Establish an initial passing opcode set for status, transfer, stack-adjacent control, direct-register transfer, and register inc/dec behavior in native-mode compliance vectors.
- [x] Implement decode and execution with correctness ahead of optimization for the current bootstrap opcode set.
- [x] Track bus-visible behavior tightly enough to support the current compliance coverage.
- [x] Iterate until the current bootstrap corpus slice reaches a trustworthy pass rate.

### SPC700 compliance harness and APU bootstrap

- [x] Mirror the 65816 harness strategy for SPC700.
- [x] Build an initial passing opcode base covering immediate loads, register transfers including stack-pointer moves, flag control, branches, calls/jumps, stack pushes/pops, returns, accumulator shifts/rotates, and basic register inc/dec behavior.
- [x] Establish APU-side timing and communication boundaries before audio polish.
- [x] Wrap the SPC700 core in an explicit APU bootstrap boundary with user-supplied IPL ROM loading, CPU/APU communication ports, and timing-facing step APIs.
- [x] Require user-supplied firmware only; do not ship blobs.

### Main SNES memory map and interrupt/timing model

- [x] Model WRAM, MMIO, DMA registers, joypad latching, NMI, IRQ, and open-bus behavior as needed for test coverage.
- [x] Prefer explicit timing state over hidden implicit ordering.

### DMA/HDMA correctness

- [x] Implement DMA and HDMA around a testable controller model.
- [x] Verify timing-sensitive bootstrap cases before optimizing transfer paths.

### PPU register model and frame generation

- [x] Start with correctness-oriented register behavior and software rendering.
- [x] Target enough fidelity for bootstrap test ROMs and early game boot scaffolding before frontend polish.
- [x] Defer shaders, filters, and high-end presentation features.

### ROM-based CPU/APU/PPU regression suite

- [x] Add pass/fail ROM execution support for CPU, APU, and PPU regression cases.
- [x] Keep test inputs local-only and configurable rather than vendor-shipped.

### CLI usability and artifact management

- [x] Support ROM inspection, headless execution, save-state dump/load, log control, and deterministic success/failure exit behavior for automation.
- [x] Add screenshot and regression artifact workflows once frame output is stable.

### Linux playable baseline

- [x] Reach a tested baseline where selected bootstrap ROMs boot, render, accept input, and produce audio.
- [x] Treat this as a subsystem-correctness milestone, not a compatibility finish line.

### Save RAM and save states

- [x] Persist battery-backed RAM safely.
- [x] Keep state serialization explicit and versionable.
- [x] Add round-trip determinism tests.

### Performance profiling and hot-path optimization

- [x] Use benchmarks and tracing to identify real bottlenecks.
- [x] Optimize only after correctness and regression coverage are in place.

### Windows support hardening

- [x] Validate path handling, CI, packaging assumptions, and platform-specific host integration.
- [x] Keep the core platform-agnostic while the host layer adapts.

### `egui` frontend bootstrap

- [x] Build a separate `egui` crate on top of the stable host traits.
- [x] Support both Linux Wayland and Windows 11.
- [x] Include day/night mode and keep the UI modular so alternate frontends remain viable.
- [x] Use bsnes as a loose information architecture reference, not a copy target.

### Additional frontends and advanced user features

- [x] Keep frontend work isolated from the emulation core.
- [x] Allow experimentation with multiple UI shells without changing subsystem behavior.

### Coprocessor support

- [ ] Defer all enhancement chips until the base console is stable and well-tested.
- [ ] Add chips behind clearly bounded subsystem work rather than widening the early core.

### Nice-to-have features such as filters, shaders, rewind, movie recording

- [ ] Tackle only after the core is robust, measurable, and user-facing basics are stable.

### Explicit non-feature for now: debugger tooling

- [x] Do not spend roadmap capacity on debugger UI or advanced introspection workflows during bootstrap.
