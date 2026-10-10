# Starbyte product roadmap

**North star:** a comfy, feature-rich Rust SNES emulator for Linux/Wayland. Real gameplay, responsiveness, accessibility and pleasant player features outrank hardware accuracy for its own sake.

## P0 — reach a genuinely playable game

- [ ] Validate native Wayland launch, tiled-window behavior, fullscreen, focus and gamepad use on the target Linux desktop.
- [x] Provide a copyright-free, built-in visual demo for local Wayland smoke testing and generated CPU-to-PPU pixel regression (implemented; native desktop verification remains open).
- [x] Capture a recognizable Super Mario World title frame from a user-supplied ROM; real-ROM screenshot evidence is documented in [the compatibility record](compatibility/super-mario-world.md).
- [x] Drive Super Mario World title, save-selection, welcome and overworld navigation into Yoshi's Island 2; deterministic headless input confirms sustained rightward movement, a running jump, camera scrolling and death/recovery. Native controls remain a separate open gate.
- [x] Trace and fix the observed SMW boot/gameplay blockers without ROM-specific hacks: controller encoding and VBlank polling, PPU subscreen compositing, and CPU multiply/divide MMIO. More accurate bus timing and broader ROM compatibility remain ongoing.
- [x] Preserve observable early-frame emulator errors in Play View and Logs rather than silently proceeding after failed first frame (source; core panic/OS crash triage still open).
- [ ] Complete authentic SNES sound synthesis and a responsive Linux audio output path. The core now supports 32 kHz DSP sample scheduling, BRR decode/loop, multi-voice mixing and basic gain/envelope in synthetic tests. The isolated SPC700 bus maps DSP $F2/$F3, mailboxes $F4-$F7, timers $FA-$FF, CONTROL port clears and the optional 64-byte IPL overlay. A small sound CPU program can execute against it in tests. An opt-in, isolated IPL decoder now captures indexed multi-block transfers into SPC RAM and handles entrypoint selection; synthetic tests connect uploaded BRR bytes to DSP output. Next validate the uploader against real commercial-ROM CPU-to-APU traffic, then replace the production bootstrap only when authentic SPC700 sound drivers can run reliably. SPC700 now recognizes 256/256 opcode values (synthetic CI verified at `bbcfde7e`), with an opt-in isolated uploaded-driver runner and bounded synthetic audio execution. Next: exercise real CPU-to-APU uploader traffic without title-specific shortcuts, validate real sound drivers, correct instruction/timer/DSP fidelity, implement missing ADSR/gaussian/echo/noise, then connect Linux speaker output. Commercial ROM audio is still silent.
- [ ] Measure and improve frame pacing, latency, pauses and save stability during sustained play.
- [x] Isolate ROM materialization from the cover-download worker and expose an explicit Play Selected action and visible job feedback (hosted CI passed at `9fe41ff5`; native Wayland launch still unverified).
- [x] Add core frame instruction-progress guard and GUI 1.5-second wall-clock budget with frame/PC diagnostics to prevent indefinite synchronous stalls (native test pending).
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
- [x] Batch ROM scan manifest writes, safely skip corrupt ZIPs, limit ZIP member size, and atomically repair truncated extracted ROM caches (CI pending).
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
- [x] Add read-only local Cheatarium SNES index candidate search to CLI (exact ROM identity and cheat execution not implied).
- [ ] Surface Cheatarium candidates in the game UI with explicit region/build matching and code-device-specific execution validation.
- [ ] Add reliable, bounded compatibility-probe commands and diagnostics that can classify evidence without claiming playability.
- [ ] Integrate Cheatarium as an optional, read-only curated SNES cheat source after its SNES schema stabilizes; match ROM identity/region, retain source attribution, never silently apply cheats or rewrite archives.
- [ ] Build an opt-in **read-only**, permission-scoped MCP adapter exposing version, diagnostics and local library summaries.
- [ ] Define and test user authorization for future mutations: explicit ROM paths, no implicit upload, no arbitrary shell execution.
- [ ] Create a deliberate Linux release/packaging process, with semver and reproducible binaries when the player experience is ready.

## Engineering principles

Use [status](status.md) for verified readiness and [PPU coverage](ppu-coverage.md) for supported register behavior. The older [commercial ROM](commercial-rom-roadmap.md), [GUI](gui-roadmap.md), and [coprocessor](coprocessor-roadmap.md) worklists are useful technical inventories but do **not** override actual playability criteria. Preserve fun-first priorities, natural Rust module boundaries, CI-backed tests, and direct documentation of what remains unverified.

**Non-goals:** cycle-perfect emulation as an independent objective; shipping copyrighted ROMs or firmware; unrestricted agent access to local files; polishing every enhancement chip before a single comfortable game works.
