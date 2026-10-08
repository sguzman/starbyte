# Starbyte product roadmap

Mission: a fun-first, comfy Rust SNES emulator for Linux/Wayland, with a delightful game library and useful player features. Hardware accuracy is justified by visible compatibility, stability or fun; it is not the overriding metric.

**Status is not completion.** This roadmap lists future acceptance gates; the old checklist was a scaffold inventory, not proof of commercial playability. See [status](status.md) for evidence.

## P0 — establish a truthful playable baseline

- [ ] Run and record Linux `fmt`, `clippy`, `test`, GUI build, and headless smoke checks.
- [ ] Verify native Wayland startup in a real compositor and observe resize, tiling, fullscreen, focus and dialogs.
- [x] Redirect config and cache defaults to XDG user directories while retaining legacy reading and explicit overrides (code change; local verification pending).
- [ ] Inspect the PPU title-screen path, especially background tiles, scroll, screen base, sprites and palettes.
- [ ] Reach a verified, non-placeholder Super Mario World title scene with a user-supplied ROM.
- [ ] Reach input-driven title/menu transition and first controllable gameplay.
- [ ] Record audio continuity, latency and save/load behavior during real play.
- [ ] Publish an evidence-based compatibility matrix with reproducible milestone criteria.

## P1 — make the desktop genuinely cozy

- [x] Lower the forced window minimum and add compact-width pop-up panels (source change; native Wayland UX still unverified).
- [ ] Make the library fast, readable and keyboard/gamepad friendly; improve cover/metadata fallback and offline behavior.
- [ ] Add thoughtful pause/resume, recent games, per-game controls and quick save/load affordances.
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

- [ ] Stabilize JSON CLI surfaces: inspect, sessions, compatibility probes, library status and diagnostics.
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
