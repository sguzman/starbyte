# Starbyte: current state

**Updated:** 2026-10-08. **Product maturity:** experimental Rust SNES emulator, intended for comfortable Linux/Wayland play rather than cycle-perfect hardware accuracy.

**Evidence rule:** passing synthetic tests is not evidence that a specific commercial title is playable. The latest fully verified CI baseline at this writing is [`a0459619`](https://github.com/sguzman/starbyte/actions/runs/37738795816): Ubuntu and Windows tests, plus Linux formatting/Clippy, all passed. Later commits need their own CI results. This is not a local Wayland acceptance test.

## What exists today

| Area | Implementation and evidence | Remaining uncertainty |
| --- | --- | --- |
| Rust workspace | Four crates: core, frontend, native egui, CLI; CI builds and tests all targets | No distributed Linux release validated |
| CPU and system | 65816, cartridge mapping, DMA/HDMA, timing, joypads and multiple enhancement-chip implementations; extensive synthetic tests | Individual commercial ROM compatibility not proven |
| PPU | Software Mode 0/1 tile backgrounds, sprites, CGRAM, priority, scroll, window masks, fixed-color math and brightness; synthetic pixel regressions | No verified Super Mario World title/gameplay; other modes, subscreen, full raster timing and color effects incomplete |
| APU/audio | SPC700/APU bootstrap, port traffic and placeholder sample generation | No authentic DSP audio output or working speaker backend |
| Saves | Core state serialization and save-RAM paths, CLI state handling; frontend in-memory quick slot F5/F8 | Persistent friendly save-slot browser and save reliability in actual games not proven |
| Desktop | Library, covers, metadata, cheats, gamepads, Play View, F9/Escape navigation, responsive tiled-window layout, fullscreen and integer display scaling | Native Hyprland/Wayland play, input focus, frame pacing, resize and dialogs not firsthand tested |
| Automation | Versioned CLI JSON introspection, diagnostics, ROM inspection and run reports; extensive CI coverage | No running MCP server; tools are CLI only |
| Packaging | Cargo sources and test workflow | No release installer or app repository |

See [PPU coverage](ppu-coverage.md) for a register-by-register account. The old [commercial ROM roadmap](commercial-rom-roadmap.md) reports **historical** 1-, 60-, and 300-frame headless Super Mario World attempts, but does not establish a working title screen. We have no user's ROM available here for a new commercial-game probe.

## How to use what's implemented

- Start the desktop with `cargo run -p starbyte-egui --`; pass `--rom /path/to/game.sfc` or `--rom-dir /path/to/roms`. Opening a ROM switches to the central **Play View**.
- In Play View: **F9** toggles the game/library; **Escape** returns to library; **F5/F8** save/load one temporary in-memory slot. The toolbar has Pause and Fullscreen. Don't interpret the slot as a persistent game save.
- For reproducible, local-only inspection: `cargo run -p starbyte-cli -- doctor --json`, `capabilities`, `inspect /path/to/game.sfc --json`, or `run /path/to/game.sfc --frames 1 --report-json /path/to/report.json`. See [automation](automation.md).
- New settings default to XDG config/cache locations. Prior checkout-local settings can be read on first run from the original directory; see [README](../README.md).

## Playability acceptance gates still open

1. Boot a user-provided, legally held Super Mario World ROM and capture a stable, recognizable title image, not just a nonblack frame.
2. Drive the title/menu with input and reach controllable gameplay; check timing, DMA/HDMA, background/sprite display and basic save behavior.
3. Integrate genuine audio synthesis and an actual Linux sound output backend; confirm sustained low-stutter playback.
4. Run the desktop natively under Wayland/Hyprland. Exercise tiling, maximize/fullscreen, focus, key/controller mappings, reconnects and gamepad input.
5. Record game-by-game outcomes in a compatibility matrix with exact ROM identity, test steps, and evidence. Keep broken/unknown outcomes visible.
6. Package versioned Linux builds only after the above core usability has been established.

**Next engineering focus:** finish observable real-game title/gameplay bring-up and improve the Play View's measured responsiveness. Hardware features beyond that must justify their value for fun or compatibility; do not chase checkbox accuracy for its own sake.

See the [product roadmap](roadmap.md) for prioritized outstanding work. Git commits and GitHub Actions retain the chronological development log; this page is intentionally a *current-state description*, not a running changelog.
