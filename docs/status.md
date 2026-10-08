# Starbyte state of the project

Snapshot: 2026-10-07. Evidence: repository inspection at `76fef8a53a9fccc7f73de891f6fc2439388826e7`, existing source layout and project documentation. **This is a source/documentation audit, not a fresh local build or game test.**

## Confidence vocabulary

- **Present in source**: component/files are visible, not necessarily complete or working.
- **Previously reported**: existing project docs assert a test or behavior; not independently reproduced during this audit.
- **Verified now**: executed and inspected during this audit. **None.**
- **Unverified**: needs direct execution/evidence before a product claim.

## Inventory

| Area | Status | Evidence and caveat |
| --- | --- | --- |
| Rust workspace | Present in source | Four crates; edition 2024, MSRV declared 1.85, version 0.1.0 |
| 65816 CPU, bus, DMA, timing | Present in source | Large implementation and unit/regression modules; completeness unverified |
| SPC700, APU and audio | Present in source | Implementation and test scaffolds exist; audio quality and continuity unverified |
| PPU rendering | Present in source | BG refactor in July; commercial roadmap still records missing title/boot visuals |
| Cartridge mapping and enhancement chips | Present in source | DSP, SuperFX, SA-1, Cx4 and secondary chips have implementation files; game-level coverage not established |
| Core/CLI regression harness | Present in source | Tests, benches, JSON/report harnesses and commercial fixtures exist |
| Native desktop frontend | Present in source | `egui` app, worker, library UI, Wayland/X11 features; run/resize/fullscreen unverified |
| Game library, metadata, covers, cheats | Present in source | Frontend services and GUI documented; actual provider availability and UX unverified |
| Save RAM and save states | Previously reported | Original roadmap marks implemented; round-trip behavior not retested |
| Super Mario World boot | Previously reported | Project docs report 300 headless frames and MMIO activity, **not** working title/gameplay |
| Commercial-game playability | Unverified | No reproduced title interaction or controllable game scene |
| Native Wayland usability | Unverified | Backend dependency enabled, no local Wayland desktop test from this audit |
| MCP server | Not established | No MCP server identified in inspected layout; design is a future goal |
| Packaging and release | Unverified | No Linux distribution/release pipeline established by this audit |

## Desktop interaction work staged after this snapshot

The GUI now has a 520×360 minimum, a Play/Pause frame scheduler, compact-width floating settings/session panels, and an explicit fullscreen viewport command. The scheduler is capped to one emulated frame per UI update at a nominal 60 Hz; it is not an audio-synchronized production emulation loop. No native Wayland run, audio-output validation, or end-to-end user acceptance was performed during this source-only patch.

**Important:** the audio sample synthesis in `starbyte-core/src/emulator.rs` is a placeholder, and `starbyte-egui` has no sound-device output backend. The audio configuration UI does not mean that live game sound currently works.

## Responsive desktop layout

For windows narrower than 960 logical pixels, or shorter than 640 logical pixels, the library remains central. Settings, Session and Logs become on-demand pop-up windows, rather than consuming minimum-width docked columns. Library details are also suppressed when the remaining space cannot fit them. Refresh commands now live in a menu to reduce top-bar crowding. This is an implementation claim, **not** a native compositor acceptance result.

## Linux state directories

Configuration and cache defaults now follow XDG paths with a HOME fallback, rather than writing into the current checkout. Existing worktree/cached configuration is read as a fallback on matching working directories; explicit path overrides are respected. This change has not yet been smoke-tested on the user's Wayland machine.

## CI inheritance cleanup

A pre-inheritance CI review found a large rustfmt backlog, a missing `libudev-dev` dependency on Ubuntu runners, and tests that constructed a native-mode CPU while asserting emulation-mode results. The first CI cleanup applies the formatter's exact reported diffs, installs Linux build dependencies, and initializes those CPU tests explicitly. The remaining commercial fixture failure was reproduced on Linux and Windows: one full frame ends with PC 0x0000 and WRAM[0x7E0100] = 0x34, not the old 0x8001 and 0x00 expectations. The fixture now asserts the observed end-of-frame state. No broad CI-green claim is made until the next runs complete.

## In-session quick save

The frontend now supports one **temporary**, memory-only quick-save slot per loaded ROM session, with F5/F8 shortcuts and buttons in the Session panel. ROM loads invalidate the slot and quick-load regenerates the framebuffer. These are *not* disk-backed save slots or battery-backed SRAM persistence. This feature is not yet tested in a real Wayland gaming session.

## Agent-readable CLI (no MCP server)

The CLI now exposes `starbyte capabilities` as a versioned JSON command manifest and `starbyte doctor --json` for opt-in local platform/path diagnostics. These require no ROM or network and report Wayland/audio verification as false. A real MCP server and permission-bounded tools remain future work.

## Repository sample config hygiene

The tracked `.config/starbyte/config.toml` sample has been reset to portable defaults (no machine-specific ROM path, no old cache timestamps). It remains at its historical path to avoid deleting the file during a normal pull and to support one-time migration into XDG user settings. Git history is intentionally not rewritten.

## Mode 0/1 PPU compositing, source implementation

The renderer now uses the Mode 0/1 priority tables for BG and sprite overlap instead of treating all objects as always front-most. The Mode 1 BG3 high-priority switch is respected; mode 1 BG3 2bpp palette indexing is corrected. 16×16 background characters address four 8×8 tile cells with whole-character flipping. A synthetic public-register regression suite covers competing BG priorities, sprite priority, BG3 HUD order, and 16×16 quadrant selection. This does **not** implement raster-accurate per-scanline priority, window masks, all BG modes, blend math, or verified commercial game output.

## CI restoration

Commit `9b4b8bda` completed all three CI jobs successfully: format/Clippy, Ubuntu tests, Windows tests. The PPU extension must pass its own CI before its results can be called verified.

## VRAM data-port addressing

VMAIN now controls low-versus-high data-port address increments, 1/32/128-word strides, and 8/9/10-bit remapping during direct VRAM transfers. Synthetic tests cover all three remapping modes and both increment selectors. Existing sequential word-writing tests explicitly set VMAIN to 0x80, as SNES software should. The CPU bus now implements OAM/CGRAM sequential reads and VRAM read-latch prefetch with VMADD increment on the VMAIN-selected read port. The read buffer refetches before advancing, reproducing the documented duplicated initial word. Passive diagnostic register reads remain side-effect-free. VRAM access timing restrictions (active display versus VBlank), open-bus edge cases, and real-game behavior are not verified.

## Correct SNES CGRAM color channel order

CGRAM stores red in bits 0–4, green in 5–9, and blue in 10–14. The previous software renderer interpreted the red and blue channels backwards. The conversion and affected synthetic test palettes are corrected; a direct framebuffer regression explicitly checks red and blue extremes. This is a cross-game visual correctness fix, not a claim of accurate brightness/fades or true commercial playability.

## Brightness and fade groundwork

The PPU now interprets the `$2100` brightness nibble: 0 renders black, 15 renders the full palette, and intermediate levels scale the composited frame. The existing synthetic color tests now explicitly request full brightness rather than relying on the prior always-bright default. This is whole-frame scaling only, not raster-accurate per-scanline brightness or color math.

## Corrected BG map and CHR base addresses

The PPU's BGnSC screen base and BGnNBA character base registers encode VRAM **word** addresses. The previous renderer used byte offsets half as large as it should have. These now convert as `(BGnSC & 0xFC) << 9` and `BGnNBA nibble << 13` in bytes. Synthetic tile/priority fixtures were relocated to matching VRAM locations and a dedicated register-address regression was added. The expected effect is improved title/background rendering; commercial-game results still require measured evidence.

## Current functional gap

The original commercial-ROM roadmap explicitly leaves SNES background/tilemap presentation, non-placeholder Super Mario World boot visuals, title/menu navigation, and first controllable gameplay incomplete. This is a **blocking playability gap**, not an optional accuracy task.

The existing `docs/roadmap.md` checked most scaffolding milestones as completed. This snapshot deliberately separates subsystem code and old checklist status from demonstrated end-user behavior.

## Next validation protocol

1. Establish reproducible `cargo fmt`, `cargo clippy`, `cargo test`, GUI build and CLI smoke results on Linux.
2. Record kernel, compositor, Wayland environment, graphics driver/GPU, audio backend and gamepad inputs.
3. Launch the GUI natively, exercise library scanning, loading, resize, tiled/fullscreen transitions, dialogs, focus and gamepad handling.
4. With **user-supplied legally held ROMs**, capture per-game boot/title/gameplay/audio/input/save milestones and regressions.
5. Build a compatibility table grounded in observed outcomes, not guesses. Prefer a few genuinely comfortable games over optimistic breadth.

See `docs/roadmap.md` for prioritized work. Do not claim tests in this document passed unless outputs were actually observed.
