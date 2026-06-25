# Coprocessor Roadmap

This document tracks the coprocessor milestone at a lower level than the main roadmap. The goal is to add enhancement-chip support without destabilizing the base console core.

## Status

- [x] Cartridge-level coprocessor detection from header metadata.
- [x] Core coprocessor abstraction layer in `starbyte-core`.
- [x] System-bus routing for coprocessor-mapped CPU reads and writes.
- [x] Save-state-safe runtime ownership of installed coprocessors.
- [ ] Full coprocessor milestone complete.

## Rules

- [x] Land one chip family at a time.
- [x] Keep coprocessor logic in `starbyte-core`, not in frontend crates.
- [x] Preserve correctness-first development with tests before compatibility claims.
- [ ] Do not mark a chip complete until at least one real software path is regression-tested.

## Phase 1: Shared Infrastructure

- [x] Detect coprocessor families from cartridge header fields.
- [x] Install coprocessor runtime state when a cartridge is loaded.
- [x] Route mapped bus reads and writes through the coprocessor before ROM fallback.
- [x] Keep coprocessor state serializable for save states.
- [x] Add tracing domains for per-chip command traffic and timing.
- [x] Add CLI-facing ROM inspection output for detected coprocessor family.

## Phase 2: DSP Family Bootstrap

- [x] Detect `DSP` cartridges and create a dedicated runtime model.
- [x] Support LoROM and HiROM DSP register windows.
- [x] Implement a command/operand/result state machine instead of a single-word stub.
- [x] Expose status bits for ready, operand wait, and result availability.
- [x] Add deterministic bootstrap commands for validation of the protocol shape.
- Variant-aware DSP scaffolding now distinguishes likely `DSP-1`, `DSP-1B`, `DSP-2`, `DSP-3`, and `DSP-4` titles, and the `0x1F` dump-style command path is in place for expanded validation.
- The DSP FSM now also models a freeze command family and a broader opcode envelope so future command-accurate work has a better scaffold.
- [ ] Replace bootstrap command behavior with authentic `DSP-1` command semantics.
- [ ] Add command coverage for the real `DSP-1` operations needed by early target software.
- [ ] Add regression inputs that validate operand packing and result ordering against known-good behavior.
- [ ] Reach a point where at least one `DSP-1` title or dedicated test path boots meaningfully.

## Phase 3: DSP Family Maturity

- [ ] Distinguish `DSP-1`, `DSP-1B`, `DSP-2`, `DSP-3`, and `DSP-4` where behavior diverges.
- [ ] Add per-variant detection rules that do not break generic DSP cartridges.
- [ ] Add timing/latency behavior only after command correctness is established.
- [ ] Add ROM-based regressions for each supported DSP variant.
- Current code now has a `DspVariant` classification layer, but behavioral divergence still needs real chip-specific command semantics and regression coverage before this phase can be called complete.
- The variant layer now sits alongside a freeze-aware DSP command path, but the real per-chip math still needs to be ported before this phase is done.

## Phase 4: SuperFX

- [x] Define a bounded `SuperFX` coprocessor interface and memory-view model.
- [ ] Model register file, instruction stepping, ROM/RAM access, and framebuffer interaction.
- [ ] Integrate `SuperFX` timing with the main bus without destabilizing CPU correctness.
- [ ] Add focused command/instruction tests before game-level compatibility claims.
- [ ] Reach a meaningful boot/render baseline for an early `SuperFX` target.
- The current runtime scaffold covers register routing, cache-window access, and timing hooks, but not full instruction execution or framebuffer production yet.
- The scaffold now also has a visible cache-window roundtrip test so the boundary stays exercised while instruction emulation is still pending.

## Phase 5: SA-1

- [ ] Design the `SA-1` boundary as a second CPU-class subsystem rather than a lightweight peripheral.
- [ ] Model shared memory, MMIO, interrupts, and synchronization with the base 65816.
- [ ] Add dedicated compliance-style tests for `SA-1` host interaction.
- [ ] Reach a deterministic boot baseline for an early `SA-1` target.

## Phase 6: Cx4

- [ ] Add `Cx4` detection and register interface.
- [ ] Implement core math/transform behavior needed by target software.
- [ ] Add regression cases for command inputs and outputs.
- [ ] Reach a meaningful in-game execution baseline for a `Cx4` title.

## Phase 7: S-DD1 And Others

- [ ] Add `S-DD1` detection and decompression-path modeling.
- [ ] Add `OBC1` support.
- [ ] Add `S-RTC` support.
- [ ] Reassess any other enhancement chips after the major compatibility chips are stable.

## Completion Criteria

- [ ] `DSP-1` is meaningfully usable and regression-tested.
- [ ] `SuperFX` has a tested boot/render baseline.
- [ ] `SA-1` has a tested boot baseline.
- [ ] `Cx4` has targeted command or game-path validation.
- [ ] At least one secondary chip from the `S-DD1` / `OBC1` / `S-RTC` group is implemented correctly.
- [ ] The top-level `Coprocessor support` milestone can be checked off without overstating compatibility.
