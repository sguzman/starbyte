# Emulation scheduler migration

**Status: in progress (source changes only; no successful CI/native acceptance verified).**
**Issue:** [#7](https://github.com/sguzman/starbyte/issues/7).
**Branch:** `main`. **Scope:** Rust/egui game-loop ownership and window-independent SNES timing.

## Why a worker, not another repaint timer

The current player runs `FrontendSession::run_frame_with_timeout` inside
`eframe::App::update`. When Hyprland/Wayland withholds a hidden window's
presentation callbacks, a queued egui repaint may not run the application
again. Disabling `pause_when_unfocused` alone does not create a game clock.
The correct long-term owner of ROM state, frame timing and save mutations is a
dedicated emulation thread. The renderer merely consumes completed frames.

## Milestone A: clock and ownership foundation — 5/7 source tasks (71%)

- [x] ⏱️ Move deadline pacing into `scheduler.rs` with a dedicated OS thread.
- [x] 🎮 Give the worker exclusive ownership of a generic machine object.
- [x] 🖼️ Replace an unbounded video queue with a one-frame mailbox.
- [x] 💤 Add independent Play, Focus, Background-pause and Speed commands.
- [x] 🛑 Make a failed emulation frame halt the worker and expose its error.
- [ ] 🧪 Confirm format, Clippy and full workspace tests on the exact commit.
- [ ] 🖥️ Verify worker scheduling while Hyprland is off-workspace or occluded.

## Milestone B: thread-owned frontend session — 7/10 source tasks (70%)

- [x] 🧠 Define `OwnedSession` wrapping `FrontendSession` in `session_engine.rs`.
- [x] 🎵 Produce PCM on the worker and enqueue it to the existing bounded Rodio worker.
- [x] 💾 Serialize quick saves, durable slots and SRAM operations on the worker.
- [x] ⏳ Flush SRAM periodically on the worker, independent of UI redraws.
- [x] 🎮 Publish an atomic held-controller state with latched short-press edges.
- [x] 🖼️ Skip unnecessary RGBA copies when a hidden renderer cannot consume them.
- [x] 📬 Expose typed ROM, save, cheat, audio and input command methods.
- [ ] 🔄 Replace all live `StarbyteApp.session` access with worker commands and snapshots.
- [ ] 📊 Move current emulation telemetry (FPS, frame gaps, overlays) to worker-produced truth.
- [ ] 🎚️ Preserve every current audio speed/pitch control and its per-mode behavior through the handoff.

## Milestone C: native GUI migration — 0/8 integrated tasks (0%)

- [ ] 🎬 Route game-launch success/error replies through the emulation worker.
- [ ] ⌨️ Send controller inputs without losing key-down/up edges; preserve F5/F8/F9/Escape.
- [ ] 🪟 Display the latest completed RGBA texture without blocking the worker.
- [ ] 🏠 Keep the full library, menus, recent covers and View status-bar toggles intact.
- [ ] ⏸️ Make manually paused / automatically paused / background-allowed states independent.
- [ ] 💾 Preserve original disk-slot, SRAM and sound-mode namespaces.
- [ ] 🧹 Shut down cleanly: stop clock, flush SRAM, clear audio and join exactly once.
- [ ] 🚀 Make the worker the default runtime only after passing acceptance gates.

## Milestone D: validation — 1/7 test paths authored (14%), not yet verified

- [x] 🧪 Add a display-server-free, real built-in ROM smoke path (`--demo --scheduler-smoke`) to CI.
- [ ] ✅ Confirm Linux CI format/Clippy and Linux+Windows tests pass.
- [ ] 🟩 Confirm the scheduler smoke actually completes on the exact commit.
- [ ] 🪟 Run Wayland window visible/focused and confirm accurate ~60.1 emulated FPS.
- [ ] 🌌 Switch to another Hyprland workspace; confirm continued emulation and sound when allowed.
- [ ] 💤 Repeat with background pause checked; verify true stop, no catch-up burst and clean resume.
- [ ] 🛡️ Confirm manual Pause, gamepad, cheats, screenshot, quick/disk saves and SRAM across restarts.

## Architectural invariants

1. **One mutable emulator owner.** Only the worker touches `FrontendSession`.
   No `Arc<Mutex<FrontendSession>>` held while drawing UI.
2. **Compositor-independent clock.** `Instant` deadlines and command-receive
   timeouts are enough to drive the worker. No `request_repaint` dependency.
3. **Bounded publication.** The UI sees the newest frame, never a growing video
   backlog; emulation progress must not wait for hidden windows.
4. **Independent audio.** PCM is delivered directly from worker to existing
   bounded audio thread; video is not an audio clock.
5. **Serialized disk state.** ROM load/cheats/save-state/SRAM mutations never race
   an executing SNES frame.
6. **Conservative defaults.** `pause_when_unfocused = true` and existing
   saved settings remain valid. A hidden status bar does not stop emulation.
7. **Falsifiable acceptance.** A successful synthetic clock test cannot prove
   Hyprland behavior, audio fidelity or commercial ROM compatibility.

## Reproduction / developer smoke

```fish
cargo run --locked -p starbyte-egui -- --demo --scheduler-smoke
```

This headless path is a diagnostic of the new worker only. Ordinary
`cargo run --release -p starbyte-egui` **still runs the existing UI-owned
scheduler until Milestone C is complete**. Never describe this intermediate
state as a fixed background-pause regression.

## Source evidence

- `scheduler.rs`: dedicated clock, mailbox, command and transaction tests.
- `session_engine.rs`: worker-owned SNES session, audio, input port, periodic SRAM
  flush and optional framebuffer copying.
- `main.rs`: explicit headless worker smoke route, no production default switch.
- `.github/workflows/ci.yml`: build and smoke task authored; completion not yet
  verified.
