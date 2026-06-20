# Starbyte Roadmap

Starbyte is being bootstrapped as a correctness-first, CLI-first SNES emulator with a reusable core and future modular frontends. Linux is the first shipping target. Windows and `egui` follow after the core reaches a stable, test-backed baseline. Debugging features are intentionally a non-feature for now.

## Milestones

- [ ] Workspace and developer foundation
- [ ] Cartridge loading and ROM mapping
- [ ] 65816 compliance harness
- [ ] 65816 execution core
- [ ] SPC700 compliance harness and APU bootstrap
- [ ] Main SNES memory map and interrupt/timing model
- [ ] DMA/HDMA correctness
- [ ] PPU register model and frame generation
- [ ] ROM-based CPU/APU/PPU regression suite
- [ ] CLI usability and artifact management
- [ ] Linux playable baseline
- [ ] Save RAM and save states
- [ ] Performance profiling and hot-path optimization
- [ ] Windows support hardening
- [ ] `egui` frontend bootstrap
- [ ] Additional frontends and advanced user features
- [ ] Coprocessor support
- [ ] Nice-to-have features such as filters, shaders, rewind, movie recording
- [ ] Explicit non-feature for now: debugger tooling

## Milestone Notes

### Workspace and developer foundation

- [ ] Maintain a Cargo workspace with `starbyte-core` and `starbyte-cli`.
- [ ] Keep frontend-facing traits in the core so CLI and future GUIs share the same host boundary.
- [ ] Standardize structured logging with subsystem-oriented tracing filters.
- [ ] Add linting, tests, integration tests, and benchmark harnesses early.

### Cartridge loading and ROM mapping

- [ ] Parse cartridge headers and normalize core metadata.
- [ ] Detect LoROM and HiROM cleanly and surface actionable diagnostics for invalid images.
- [ ] Add save RAM plumbing and path management in the CLI layer.

### 65816 compliance harness

- [ ] Build a local harness interface for single-step JSON vectors.
- [ ] Validate registers, memory deltas, cycle counts, and bus traces.
- [ ] Make the harness automation-friendly for CI and local iteration.

### 65816 execution core

- [ ] Implement decode and execution with correctness ahead of optimization.
- [ ] Track bus-visible behavior tightly enough to support compliance testing.
- [ ] Iterate until the synthetic corpus reaches a trustworthy pass rate.

### SPC700 compliance harness and APU bootstrap

- [ ] Mirror the 65816 harness strategy for SPC700.
- [ ] Establish APU-side timing and communication boundaries before audio polish.
- [ ] Require user-supplied firmware only; do not ship blobs.

### Main SNES memory map and interrupt/timing model

- [ ] Model WRAM, MMIO, DMA registers, joypad latching, NMI, IRQ, and open-bus behavior as needed for test coverage.
- [ ] Prefer explicit timing state over hidden implicit ordering.

### DMA/HDMA correctness

- [ ] Implement DMA and HDMA around a testable controller model.
- [ ] Verify timing-sensitive cases before optimizing transfer paths.

### PPU register model and frame generation

- [ ] Start with correctness-oriented register behavior and software rendering.
- [ ] Target enough fidelity for test ROMs and early game boot before frontend polish.
- [ ] Defer shaders, filters, and high-end presentation features.

### ROM-based CPU/APU/PPU regression suite

- [ ] Add pass/fail ROM execution support for CPU, APU, and PPU regression cases.
- [ ] Keep test inputs local-only and configurable rather than vendor-shipped.

### CLI usability and artifact management

- [ ] Support ROM inspection, headless execution, save-state dump/load, log control, and deterministic exit codes.
- [ ] Add screenshot and regression artifact workflows once frame output is stable.

### Linux playable baseline

- [ ] Reach a tested baseline where selected ROMs boot, render, accept input, and produce audio.
- [ ] Treat this as a subsystem-correctness milestone, not a compatibility finish line.

### Save RAM and save states

- [ ] Persist battery-backed RAM safely.
- [ ] Keep state serialization explicit and versionable.
- [ ] Add round-trip determinism tests.

### Performance profiling and hot-path optimization

- [ ] Use benchmarks and tracing to identify real bottlenecks.
- [ ] Optimize only after correctness and regression coverage are in place.

### Windows support hardening

- [ ] Validate path handling, CI, packaging assumptions, and platform-specific host integration.
- [ ] Keep the core platform-agnostic while the host layer adapts.

### `egui` frontend bootstrap

- [ ] Build a separate `egui` crate on top of the stable host traits.
- [ ] Support both Linux Wayland and Windows 11.
- [ ] Include day/night mode and keep the UI modular so alternate frontends remain viable.
- [ ] Use bsnes as a loose information architecture reference, not a copy target.

### Additional frontends and advanced user features

- [ ] Keep frontend work isolated from the emulation core.
- [ ] Allow experimentation with multiple UI shells without changing subsystem behavior.

### Coprocessor support

- [ ] Defer all enhancement chips until the base console is stable and well-tested.
- [ ] Add chips behind clearly bounded subsystem work rather than widening the early core.

### Nice-to-have features such as filters, shaders, rewind, movie recording

- [ ] Tackle only after the core is robust, measurable, and user-facing basics are stable.

### Explicit non-feature for now: debugger tooling

- [ ] Do not spend roadmap capacity on debugger UI or advanced introspection workflows during bootstrap.
