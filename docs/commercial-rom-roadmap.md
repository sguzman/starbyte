# Commercial ROM Roadmap

This document tracks the compatibility push from bootstrap/test ROM behavior toward real commercial game boot and gameplay, using `Super Mario World` as the proving target. The goal is to improve general SNES subsystem behavior for LoROM commercial software rather than add game-specific hacks.

## Rules For Checking Boxes

- [x] Mark a box complete only when the code is implemented and verified by a reproducible check.
- [x] Prefer subsystem checks, regression artifacts, or saved runtime evidence over visual guesswork.
- [x] Keep `Super Mario World` as the proving target, but avoid title-specific conditionals or hacks in the emulator core.

## Current Verified Status

- [x] Zip-backed ROM inspection works in the CLI `inspect` path.
- [x] Zip-backed ROM loading works in the direct CLI `run` path.
- [x] Early commercial-ROM 65816 bootstrap work has advanced past the first unsupported reset opcodes.
- [x] Unsupported-opcode errors now include CPU-visible instruction addresses for faster commercial-ROM debugging.
- [x] `Super Mario World` can complete at least one headless frame with the current bootstrap core.
- [x] `Super Mario World` can complete a 60-frame headless run with the current bootstrap core.
- [x] The early SPC/APU startup wait loop is no longer the first blocker after the APU port/handshake fixes.
- [x] Core emulator regression tests still pass after the current bootstrap CPU/APU changes.

## Phase 1: CPU And APU Bootstrap Viability

- [x] Fix direct ROM loading so zipped commercial ROM archives can be loaded through the same host paths as extracted ROMs.
- [x] Establish a repeatable `Super Mario World` headless boot probe using CLI `run`, save-state output, and JSON run reports.
- [x] Correct 65816 reset defaults well enough for commercial reset code to execute meaningfully.
- [ ] Implement the remaining early commercial-boot opcode and addressing-mode set needed to move past startup/upload loops.
- [ ] Support the remaining stack, flag, compare, rotate, branch, and memory-access behavior exercised during SMW init.
- [ ] Keep new 65816 behavior covered by focused unit tests or vector-style checks as each opcode family lands.
- [ ] Preserve accurate CPU/APU communication-port visibility through the system bus during CPU stepping.
- [ ] Move past the later SMW startup/upload loops without regressing synthetic bootstrap ROM behavior.
- [ ] Reach a stable post-init PC/state where SMW starts programming visible display state instead of only bootstrap handshakes.

## Phase 2: Commercial-ROM Boot Harness

- [ ] Add a dedicated commercial-ROM regression fixture flow alongside the existing ROM regression support.
- [ ] Add a `Super Mario World` fixture that records milestone expectations such as frame progress, PC/state ranges, and selected MMIO or WRAM reads.
- [ ] Support richer verification probes for commercial boot cases:
CPU PC or PC-range checks
frame progression checks
selected WRAM and MMIO reads
framebuffer signature or sampled-region checks
- [ ] Add optional trace capture useful for commercial-ROM debugging without coupling debugger UI into the core.
- [ ] Keep the harness local-only and configurable rather than bundling ROM assets into the repo.

## Phase 3: PPU Bring-Up For Visible Commercial Boot

- [ ] Replace the current synthetic stripe renderer with VRAM-backed rendering behavior.
- [ ] Implement CPU-visible VRAM address and data access needed for commercial setup code.
- [ ] Implement tile/tilemap-backed BG rendering for the minimum modes SMW uses during boot and title flow.
- [ ] Preserve CGRAM-backed palette output through the renderer.
- [ ] Respect screen enable and forced-blank behavior in a way that matches commercial boot sequencing.
- [ ] Implement enough scroll and screen-base behavior to produce non-placeholder title/boot visuals.
- [ ] Reach a verified non-black, non-placeholder SMW boot frame in headless artifacts and the GUI.
- [ ] Keep existing synthetic PPU tests green or replace them with stronger VRAM/tile-backed tests.

## Phase 4: Title Screen And Menu Interaction

- [ ] Advance from visible boot output to a stable title-screen or attract-mode state.
- [ ] Confirm that controller input changes title or menu state deterministically.
- [ ] Add milestone checks for input-driven progression rather than only frame advancement.
- [ ] Support any remaining MMIO, DMA, HDMA, or timing behavior required for title/menu flow.
- [ ] Verify the GUI can load SMW and remain stable through title/menu interaction.

## Phase 5: First Gameplay

- [ ] Reach a first controllable gameplay scene in `Super Mario World` without ROM-specific hacks.
- [ ] Verify that directional and face-button input changes gameplay state in a reproducible way.
- [ ] Support any additional sprite, background, or timing behavior needed for first gameplay.
- [ ] Capture a regression artifact for first-gameplay entry so future work can detect regressions quickly.

## Phase 6: Generalization Beyond SMW

- [ ] Refactor any SMW-driven implementation work into clearly general subsystem behavior.
- [ ] Add at least one additional non-SMW LoROM commercial smoke target after SMW first-gameplay is stable.
- [ ] Identify and separate "general commercial compatibility" work from "SMW-specific proving coverage" in future checkboxes.
- [ ] Keep this document updated as milestones finish so it remains the source of truth for commercial-ROM progress.
