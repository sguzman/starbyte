# Starbyte: current state

**Updated:** 2026-10-09. **Product maturity:** experimental Rust SNES emulator, intended for comfortable Linux/Wayland play rather than cycle-perfect hardware accuracy.

**Evidence rule:** passing synthetic tests is not evidence that a specific commercial title is playable. [GitHub Actions on `main`](https://github.com/sguzman/starbyte/actions/workflows/ci.yml) runs Linux formatting/Clippy and Ubuntu/Windows workspace tests. Confirm that the **latest completed run matches the exact commit** before making verification claims. Hosted CI does not constitute a native Wayland desktop test.

## What exists today

| Area | Implementation and evidence | Remaining uncertainty |
| --- | --- | --- |
| Rust workspace | Four crates: core, frontend, native egui, CLI; CI builds and tests all targets | No distributed Linux release validated |
| CPU and system | 65816 dispatch recognizes all 256 opcode values; DMA/HDMA, timer IRQs, master-clock-to-dot timing; $4016 serial and VBlank automatic $4218/$4219 joypad polling; synthetic regressions | Precise bus timing, auto-joypad read delay and interactive gameplay response remain under real-ROM test |
| PPU | Software Mode 0/1 tile backgrounds, sprites, CGRAM, priority, scroll, windows, color math and brightness; successful headless Nintendo opening and **900-frame animated SMW title demo** | Interactive menus/in-level graphics unverified; other modes, raster timing and color effects incomplete |
| APU/audio | SPC700/APU bootstrap, port traffic and placeholder sample generation | No authentic DSP audio output or working speaker backend |
| Saves | Core state serialization and save-RAM paths, CLI state handling; frontend in-memory quick slot F5/F8 | Disk-backed slots 1–3 and XDG SRAM autosave now exist; actual game save reliability and native Wayland UX remain unverified |
| Desktop | Library, covers, metadata, cheats, gamepads, Play View, F9/Escape navigation, responsive tiled-window layout, fullscreen and integer display scaling | Native Hyprland/Wayland play, input focus, frame pacing, resize and dialogs not firsthand tested |
| Automation | Versioned CLI JSON introspection, diagnostics, ROM inspection and run reports; read-only local Cheatarium SNES candidate search | No running MCP server; cheat lookup does not identify exact cartridge builds or activate cheats |
| Packaging | Cargo sources and test workflow | No release installer or app repository |

See [PPU coverage](ppu-coverage.md) for a register-by-register account. The latest user-supplied **Super Mario World 900-frame probe completed without errors** on Linux, with 900 JSONL frame records and 46 PPM captures. The confirmed title screen first appears around frame 215; frames 320–900 show a continuously animated **title demo** with Mario, Yoshi, enemies, coins and scrolling foreground terrain. A scheduled Start press at frames 450–454 was delivered to Starbyte's host input but **did not visibly exit the demo**, so interactive title navigation and gameplay are not yet verified. The game's NMITIMEN register remains `$81`, enabling automatic joypad reads, while Starbyte previously only latched controller data after manual `$4016` strobes. This **verified core omission** has been corrected with VBlank polling, approximate busy timing on `$4212` and fresh `$4218/$4219` results, plus synthetic tests; frame logs now expose host versus latched button bits for the upcoming commercial retest. All 256 CPU opcode values remain decoded and synthetic CPU/PPU tests pass, but cycle fidelity, native interactive input, audio DSP and SRAM reliability are separate tasks. The APU output remains a synthetic placeholder, not authentic game sound. No proprietary ROM bytes or screenshots were added to the public repository. See the [game-specific compatibility investigation](compatibility/super-mario-world.md). The older [commercial ROM roadmap](commercial-rom-roadmap.md) remains historical engineering context.

## Controller encoding correction awaiting real-ROM retest

A completed 560-frame Super Mario World run delivered the scheduled Start pulse to the host and latched joypad word at frames 450–454 but did not enter file selection. Source comparison against the SNES controller layout identified an incorrect button encoding: Start was represented as `$0008` instead of hardware-correct `$1000`; all twelve bits were misplaced. The encoded word and serial shift direction have been corrected, with unconnected ports returning zero. Synthetic regressions are included; **real-ROM title input response remains pending retest**. See [Super Mario World compatibility](compatibility/super-mario-world.md).

## Cartridge save persistence

Starbyte's desktop session now loads content-scoped SRAM from XDG data storage when a ROM is opened, and atomically persists modified SRAM on game switch, at approximately 30-second intervals while the GUI is active, and on app exit. Unchanged SRAM isn't rewritten. This is independent of disk save-state slots and of temporary F5/F8 quick saves. Saving and loading on real commercial titles remains unverified.

## Archive-backed user libraries

Starbyte discovers the existing `~/Games/Roms/SNES` collection when that directory is present and persists it as a normal removable `library.rom_dirs` configuration entry. ROMs within ZIP archives are indexed by their internal cartridge header; archives remain read-only, and only chosen members are materialized in `$XDG_CACHE_HOME/starbyte/extracted-roms/`. Library cache scans run in a worker thread and refresh on launch, even when a previous snapshot exists. Cache manifests are atomically replaced once per completed scan rather than rewritten for every ROM, damaged ZIPs are skipped without hiding healthy games, and corrupt/truncated extracted cache files are recreated atomically. ZIP members are size-bounded (64 MiB). The core's direct ZIP loader also bounds decompression and skips invalid-header ROM members to allow a later valid member in the same archive. This does not establish that the two previously rejected user ZIPs are fixed; their contents have not been available for inspection. Each completed scan also writes `manifests/rom-scan-report.json` with source names and reasons for skipped archives or members; the Settings panel provides an **Open Scan Report** action. This is implementation behavior, not a claim that every ZIP or every SNES title loads.

## Cover art

Retail box art is not ordinarily embedded in SNES ROM images. The library offers **Get Covers** to explicitly index Libretro's thumbnail catalog and download artwork for installed games to XDG cache. It leaves game ZIPs unchanged, reuses existing images instead of repeatedly downloading them, and shows a placeholder where a game lacks a match. The operation uses the network and may be lengthy for large collections; commercial game artwork is not bundled in Starbyte.

## Launch and background-work isolation

The library's **Play Selected** toolbar action and each installed grid card's **Play game** button now queue an explicit ROM launch. Resume/Pause applies only to a previously loaded game. ROM materialization has a separate worker queue from network artwork/metadata, so slow artwork downloads cannot block the start of a selected local game. A persistent bottom status strip displays launch/worker feedback. The launcher/isolation changes passed hosted CI at `9fe41ff5` (2026-10-08); native Wayland behavior remains unverified. This removes a startup scheduling problem; it does **not** prove that Super Mario World renders or plays correctly.

## Built-in visual smoke test

`cargo run -p starbyte-egui -- --demo` creates a copyright-free 65816 LoROM in the XDG cache and opens its alternating red/cyan checkerboard through the normal emulator frontend. This permits testing tiling, focus, fullscreen, screenshot export and scaling without a commercial cartridge. The Play View's Input overlay reports the effective keyboard/gamepad SNES button state for local input smoke tests; the generated checkerboard itself remains static. A core unit test checks exact screen pixels, but **the native Wayland window must still be tested by a user with that compositor**.

## How to use what's implemented

- Start the desktop with `cargo run -p starbyte-egui --`; pass `--rom /path/to/game.sfc` or `--rom-dir /path/to/roms`, or select **Open ROM...** in the library. Dragging a local ROM or single-game ZIP onto the desktop window also uses the same validated loader. Multi-ROM ZIPs require selection through the library. Opening a ROM switches to the central **Play View**.
- In Play View: **F9** toggles the game/library; **Escape** returns to library; **F5/F8** save/load one temporary in-memory slot. The toolbar has Pause, Fullscreen and a Disk Slots menu for three per-ROM persistent slots; F12 exports a screenshot to XDG data storage. Disk slots remain local and are never uploaded.
- For reproducible, local-only inspection: `cargo run -p starbyte-cli -- doctor --json`, `capabilities`, `inspect /path/to/game.sfc --json`, or `run /path/to/game.sfc --frames 1 --report-json /path/to/report.json`. See [automation](automation.md).
- New settings default to XDG config/cache locations. Prior checkout-local settings can be read on first run from the original directory; see [README](../README.md). Config writes use atomic temporary-file replacement so interruptions do not truncate a previously valid configuration.

## Desktop responsiveness instrumentation

The Session panel's 60-frame debug step now advances one frame per UI update with a Cancel control, rather than calling all 60 frames synchronously and freezing the interface for the entire batch. Play View has an optional Timing overlay; the Session panel also displays the exponential moving average, peak, and number of emulation frames exceeding the 16.7 ms budget. These are measurements of `FrontendSession::run_frame` only, not full wall-clock input latency or GPU presentation FPS. **Each individual emulation frame still runs on the egui UI thread**, so it can stall briefly; a new 1.5-second per-frame deadline and instruction-progress guard now pause playback and report the frame/PC rather than allowing an unbounded loop. This is a safety net, not improved rendering correctness or background emulation.

## Reproducible early-frame evidence

The headless `starbyte-cli run` command now supports `--frame-log PATH` (one flushed `starbyte.frame_log.v1` record per attempted frame, including CPU registers, PPU-write and APU-port counters) and optional bounded local framebuffer images (`--frame-images-dir DIR`, `--frame-image-every N`, `--max-frame-images N`). On a recoverable frame error, the JSONL log retains earlier completed frames and a final error record, whereas the end-of-run report may not exist. Frame images contain only successfully completed framebuffers and do not prove the artwork is rendered correctly. Use `--no-save-ram` to avoid reading or writing SRAM sidecars during controlled diagnosis. Implementation and CLI integration tests exist; do not treat the tools themselves as evidence that Super Mario World is playable. See [automation](automation.md) for example commands.

## Early-frame compatibility failures

The commercial-ROM fixture recorder now uses the core's guarded frame runner for ordinary capture and an explicit 20,000-instruction-per-frame ceiling while collecting full instruction traces. Stalled probes return a frame/CPU-address error rather than running indefinitely. This bounds instruction count, not wall-clock time or trace size across many requested frames; it does not imply that an actual commercial title is playable.

A test of a commercial game that stops within a few frames needs the exact game's name and diagnostic log before its root cause can be determined. Starbyte now surfaces a recoverable emulation error in Play View, pauses playback, and records the error in Logs; this cannot protect against native process crashes or Rust panics. A terminal backtrace is required for those. A successful native checkerboard still does not demonstrate commercial playability.

## Cheatarium integration boundary

The CLI already provides a **read-only candidate lookup** against a locally supplied, versioned Cheatarium SNES index (`starbyte-cli cheatarium --index PATH --title TITLE`). The lookup does not establish exact ROM identity or region, decode/apply cheat devices, alter a loaded cartridge, or enable any cheat. Interactive integration remains future work: require explicit cartridge/region matching, provenance display, user opt-in, and verified runtime execution before advertising usable cheats.

## Playability acceptance gates still open

1. Boot a user-provided, legally held Super Mario World ROM and capture a stable, recognizable title image, not just a nonblack frame.
2. Drive the title/menu with input and reach controllable gameplay; check timing, DMA/HDMA, background/sprite display and basic save behavior.
3. Integrate genuine audio synthesis and an actual Linux sound output backend; confirm sustained low-stutter playback.
4. Run the desktop natively under Wayland/Hyprland. Exercise tiling, maximize/fullscreen, focus, key/controller mappings, reconnects and gamepad input.
5. Record game-by-game outcomes in a compatibility matrix with exact ROM identity, test steps, and evidence. Keep broken/unknown outcomes visible.
6. Package versioned Linux builds only after the above core usability has been established.

**Next engineering focus:** finish observable real-game title/gameplay bring-up and improve the Play View's measured responsiveness. Hardware features beyond that must justify their value for fun or compatibility; do not chase checkbox accuracy for its own sake.

See the [product roadmap](roadmap.md) for prioritized outstanding work. Git commits and GitHub Actions retain the chronological development log; this page is intentionally a *current-state description*, not a running changelog.
