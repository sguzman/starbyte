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
- [x] Replace bootstrap command behavior with authentic `DSP-1` command semantics.
- [x] Add command coverage for the real `DSP-1` operations needed by early target software.
- [x] Add regression inputs that validate operand packing and result ordering against known-good behavior.
- [x] Reach a point where at least one `DSP-1` title or dedicated test path boots meaningfully.
- The current DSP runtime now executes real `DSP-1` math/geometry commands for multiply, inverse, attitude/objective/subjective/scalar/gyrate transforms, triangle, radius, range, distance, rotate, and polar paths, with command-latency staging and chip-level regression tests.
- ROM-regression coverage now includes dedicated DSP-family fixtures that exercise `DSP-1`, `DSP-1B`, `DSP-2`, `DSP-3`, and `DSP-4` through the full emulator path.

## Phase 3: DSP Family Maturity

- [x] Distinguish `DSP-1`, `DSP-1B`, `DSP-2`, `DSP-3`, and `DSP-4` where behavior diverges.
- [x] Add per-variant detection rules that do not break generic DSP cartridges.
- [x] Add timing/latency behavior only after command correctness is established.
- [x] Add ROM-based regressions for each supported DSP variant.
- The current code now has a `DspVariant` classification layer plus variant-specific command-availability, dump behavior, and ROM-backed regression coverage.
- The remaining maturity gap is broader real-software compatibility and additional authentic command coverage, not the absence of per-variant regression scaffolding.

## Phase 4: SuperFX

- [x] Define a bounded `SuperFX` coprocessor interface and memory-view model.
- [x] Model register file, instruction stepping, ROM/RAM access, and framebuffer interaction.
- [x] Integrate `SuperFX` timing with the main bus without destabilizing CPU correctness.
- [x] Add focused command/instruction tests before game-level compatibility claims.
- [x] Reach a meaningful boot/render baseline for an early `SuperFX` target.
- The current runtime now includes a bounded executable SuperFX core with cache-backed fetch, immediate loads, prefix handling, plotting, ROM-buffer reads, and framebuffer overlay output, all covered by targeted regression tests.
- A synthetic execute-and-draw bootstrap path now runs end-to-end through the ROM regression harness and produces a validated rendered overlay frame.

## Phase 5: SA-1

- [x] Design the `SA-1` boundary as a second CPU-class subsystem rather than a lightweight peripheral.
- [x] Model shared memory, MMIO, interrupts, and synchronization with the base 65816.
- [x] Add dedicated compliance-style tests for `SA-1` host interaction.
- [x] Reach a deterministic boot baseline for an early `SA-1` target.
- The current SA-1 runtime exposes a bounded second-CPU boundary with MMIO control, internal RAM, BW-RAM, host mailboxes, interrupt signaling, and a deterministic boot-complete handshake that is covered by system and ROM-regression tests.

## Phase 6: Cx4

- [x] Add `Cx4` detection and register interface.
- [x] Implement core math/transform behavior needed by target software.
- [x] Add regression cases for command inputs and outputs.
- [x] Reach a meaningful in-game execution baseline for a `Cx4` title.
- The current Cx4 runtime now detects dedicated boards, exposes a bounded register/RAM window, and executes length, rotate, and perspective-style transform commands with system and ROM-regression coverage.

## Phase 7: S-DD1 And Others

- [x] Add `S-DD1` detection and decompression-path modeling.
- [x] Add `OBC1` support.
- [x] Add `S-RTC` support.
- [x] Reassess any other enhancement chips after the major compatibility chips are stable.
- The secondary-chip pass now includes a bounded S-DD1 stream/decompression model, an OBC1 object-window controller, and a deterministic S-RTC serial time source, all routed through the system bus and regression harness.

## Completion Criteria

- [x] `DSP-1` is meaningfully usable and regression-tested.
- [x] `SuperFX` has a tested boot/render baseline.
- [x] `SA-1` has a tested boot baseline.
- [x] `Cx4` has targeted command or game-path validation.
- [x] At least one secondary chip from the `S-DD1` / `OBC1` / `S-RTC` group is implemented correctly.
- [ ] The top-level `Coprocessor support` milestone can be checked off without overstating compatibility.
