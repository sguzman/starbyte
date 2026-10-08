# Starbyte product roadmap

Mission: a fun-first, comfy Rust SNES emulator for Linux/Wayland, with a delightful game library and useful player features. Hardware accuracy is justified by visible compatibility, stability or fun; it is not the overriding metric.

**Status is not completion.** This roadmap lists future acceptance gates; the old checklist was a scaffold inventory, not proof of commercial playability. See [status](status.md) for evidence.

## P0 — establish a truthful playable baseline

- [x] Return GitHub Actions to green without masking emulator failures (verified on 9b4b8bda).

- [ ] Run and record Linux `fmt`, `clippy`, `test`, GUI build, and headless smoke checks.
- [ ] Verify native Wayland startup in a real compositor and observe resize, tiling, fullscreen, focus and dialogs.
- [x] Redirect config and cache defaults to XDG user directories while retaining legacy reading and explicit overrides (code change; local verification pending).
- [ ] Inspect the complete PPU title-screen path, including scroll, DMA, windows, raster effects and color math.
- [x] Implement OBJ 16×16 tile-index wrapping and Y modulo 256, with synthetic visual tests (CI pending).
- [x] Implement screen-anchored Mode 0/1 background mosaic sampling and synthetic pixel tests (CI pending).
- [x] Correct BG screen-map and character-data VRAM base units and add a direct regression (source; CI pending).
- [x] Respect forced blank and whole-frame brightness register levels for startup/fade baseline (source; CI pending).
- [x] Implement VMAIN writing port selection/stride/remapping with synthetic tests (CI pending).
- [x] Implement VRAM read-port buffering, address prefetch, and read-side increments, plus streaming OAM/CGRAM reads (source; CI pending).
- [ ] Validate VRAM read/write timing restrictions, open-bus behavior and DMA interactions on commercial titles.
- [x] Add Mode 0/1 BG/OBJ priority composition and 16x16 background characters with synthetic regression fixtures (source; CI pending).
- [ ] Reach a verified, non-placeholder Super Mario World title scene with a user-supplied ROM.
- [ ] Reach input-driven title/menu transition and first controllable gameplay.
- [ ] Record audio continuity, latency and save/load behavior during real play.
- [ ] Publish an evidence-based compatibility matrix with reproducible milestone criteria.

## P1 — make the desktop genuinely cozy

- [x] Lower the forced window minimum and add compact-width/short-height pop-up panels, including logs (source change; native Wayland UX still unverified).
- [ ] Make the library fast, readable and keyboard/gamepad friendly; improve cover/metadata fallback and offline behavior.
- [x] Implement Play/Pause and memory-only quick-save/load controls with F5/F8, clearing slot on ROM change (source; CI verification pending).
- [ ] Add persistent save-slot browsing, recent games, and per-game controls.
- [ ] Test remapping, controller reconnect, Wayland focus and hot-plug across realistic play sessions.
- [ ] Improve scaling, aspect-ratio handling, crisp pixel rendering, fullscreen transitions and frame pacing.
- [ ] Replace the new bounded GUI frame scheduler with a measured, responsive gameplay loop once real-game frame costs are known.
- [ ] Connect a real audio output backend; current core samples are synthetic and not played by the GUI.
- [ ] Measure audio/video responsiveness and solve real stutter before adding expensive visual effects.

## P2 — convenience and personality

- [ ] Rewind and a visible timeline, with explicit memory/performance budgets.
- [ ] Save-slot browser, thumbnails, automatic safe checkpoints and straightforward backup/export.
- [ ] Optional visual filters/shaders and accessible scaling choices.
- [ ] Screenshot/clip capture and tasteful overlays.
- [ ] Per-game profiles and a low-friction cozy session launcher.

## P3 — AI and MCP integration

- [x] Publish initial `capabilities` and `doctor --json` discovery/diagnostic entry points (source implementation; CI confirmation pending).
- [x] Add versioned JSON ROM inspection (source implementation; CI confirmation pending).
- [ ] Stabilize JSON CLI surfaces: sessions, compatibility probes and library status.
- [ ] Document a versioned machine-readable introspection schema for emulator and library state.
- [ ] Prototype an opt-in **read-only** MCP adapter with bounded resources/tools and tests.
- [ ] Add explicit, user-authorized mutations only where safe (e.g. local library refresh); constrain file access and networking.
- [ ] Build agent-friendly regression reports with reproducible local fixtures; never transmit ROM content implicitly.

## Ongoing engineering

- Fix general emulator issues demonstrated by selected game regressions; no ROM-specific hacks.
- Keep the core/platform/frontend boundaries intact.
- Preserve reproducible tests and native Linux performance.
- Maintain dependency/security hygiene and a release process with deliberate semver once user-facing behavior is established.
- Treat X11 and Windows as secondary, not equal-priority targets.

## Explicit non-goals

- Cycle-perfect accuracy as an end in itself.
- Every enhancement chip or commercial title before the core play experience works.
- Shipping commercial ROMs or proprietary firmware.
- Unrestricted agent control of local files or the emulator.
- Building a full debugger before it directly aids gameplay compatibility.

Deep technical worklists remain in [commercial ROM](commercial-rom-roadmap.md), [coprocessors](coprocessor-roadmap.md), and [GUI](gui-roadmap.md); their historical checked boxes do not supersede observed product readiness.
