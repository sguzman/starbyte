# Starbyte product roadmap

**North star:** a comfy, feature-rich Rust SNES emulator for Linux/Wayland. Real gameplay, responsiveness, accessibility and pleasant player features outrank hardware accuracy for its own sake.

## P0 — reach a genuinely playable game

- [ ] Validate native Wayland launch, tiled-window behavior, fullscreen, focus and gamepad use on the target Linux desktop.
- [x] Provide a copyright-free, built-in visual demo for local Wayland smoke testing and generated CPU-to-PPU pixel regression (implemented; native desktop verification remains open).
- [ ] Capture a recognizable Super Mario World title frame from a user-supplied ROM; rule out synthetic/nonblack placeholders.
- [ ] Drive title/menu navigation using controller input, then reach first controllable gameplay.
- [ ] Trace and fix the earliest observed real-game blockers in PPU, CPU, DMA/HDMA and timing; do not introduce ROM-specific hacks.
- [ ] Replace placeholder audio with authentic sound synthesis and a responsive Linux audio output path.
- [ ] Measure and improve frame pacing, latency, pauses and save stability during sustained play.
- [x] Replace the 60-frame blocking UI callback with incremental, cancellable stepping; expose measured per-frame runtime and over-budget counters (source; native Wayland timing pending).
- [x] Add content-scoped XDG cartridge SRAM persistence with safe atomic writes and timed flushes (implemented; real-game verification remains open).
- [ ] Establish a reproducible compatibility matrix: exact ROM identity, title/menu/gameplay/audio/input/save acceptance and screenshots or traces.

## P1 — make the desktop comfortable

- [x] Provide a Play View rather than confining the framebuffer to the session sidebar (code and hosted CI; native usability unverified).
- [x] Provide Play/Pause, F9/Escape library navigation, F5/F8 volatile quick saves, pixel scaling and fullscreen commands (native usability unverified).
- [x] Replace oversized window minimums with small-tile-friendly settings/session/log popups (native usability unverified).
- [x] Add three ROM-content-scoped persistent disk slots with atomic writes and a safe cartridge match check (implemented; real-game verification remains open).
- [ ] Build a friendly save-slot browser with thumbnails, backup/export and recovery affordances.
- [ ] Make library covers/metadata and installed/offline status pleasant even with no network; measure large-library performance.
- [x] Discover local ~/Games/Roms/SNES when present; persist source directory in config and rescan alongside cached snapshots on restart (CI pending).
- [x] Add local, bounded Recent Games menu with deduplication and safe missing-file handling (source; native verification pending).
- [x] Allow a single local ROM to be opened through a native file picker or drag-and-drop without preconfiguring a library directory (implemented; native desktop verification remains open).
- [ ] Improve gamepad/keyboard remapping, reconnection, key focus and full-screen transitions on Wayland.
- [x] Track held buttons per connected gamepad and clear unplugged pad state without releasing other controllers (source; hardware test pending).
- [x] Export native-resolution PNG screenshots with F12 and a Play View toolbar action (implemented; real-game verification remains open).
- [ ] Explore optional shaders, rewind and session history only after the core loop is comfortable.

## P2 — reproducible engineering and AI/MCP friendliness

- [x] Restore a green Linux/Windows CI baseline; preserve all genuine emulator regressions.
- [x] Build focused Mode 0/1 PPU regressions, bus and ROM boot fixtures, and headless frame diagnostics; see [PPU coverage](ppu-coverage.md).
- [x] Publish versioned JSON CLI capability discovery, platform diagnostics and cartridge inspection; no real MCP server yet.
- [ ] Add reliable, bounded compatibility-probe commands and diagnostics that can classify evidence without claiming playability.
- [ ] Build an opt-in **read-only**, permission-scoped MCP adapter exposing version, diagnostics and local library summaries.
- [ ] Define and test user authorization for future mutations: explicit ROM paths, no implicit upload, no arbitrary shell execution.
- [ ] Create a deliberate Linux release/packaging process, with semver and reproducible binaries when the player experience is ready.

## Engineering principles

Use [status](status.md) for verified readiness and [PPU coverage](ppu-coverage.md) for supported register behavior. The older [commercial ROM](commercial-rom-roadmap.md), [GUI](gui-roadmap.md), and [coprocessor](coprocessor-roadmap.md) worklists are useful technical inventories but do **not** override actual playability criteria. Preserve fun-first priorities, natural Rust module boundaries, CI-backed tests, and direct documentation of what remains unverified.

**Non-goals:** cycle-perfect emulation as an independent objective; shipping copyrighted ROMs or firmware; unrestricted agent access to local files; polishing every enhancement chip before a single comfortable game works.
