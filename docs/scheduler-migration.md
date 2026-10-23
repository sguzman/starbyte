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

## Milestone B: thread-owned frontend session — 15/18 source tasks (83%)

- [x] 🧠 Define `OwnedSession` wrapping `FrontendSession` in `session_engine.rs`.
- [x] 🎵 Produce PCM on the worker and enqueue it to the existing bounded Rodio worker.
- [x] 💾 Serialize quick saves, durable slots and SRAM operations on the worker.
- [x] ⏳ Flush SRAM periodically on the worker and at manual/background pause transitions, independent of UI redraws.
- [x] 🎮 Publish an atomic held-controller state with latched short-press edges.
- [x] 🖼️ Skip unnecessary RGBA copies after 300 ms without UI repaint heartbeats, even when Wayland never reports the window as minimized.
- [x] 📬 Expose typed ROM, save, cheat, audio and input command methods.
- [x] 🖼️ Build a renderer-only adapter that consumes published snapshots and updates egui textures.
- [x] ⌨️ Expire stale input heartbeat after missing window callbacks, preventing endlessly held keys.
- [x] 💾 Publish restored still frames after quick-save and disk-state loads, including while paused.
- [x] 🎮 Support nonblocking ROM replacement with old-game preservation after failed cartridge loads.
- [x] 📸 Share an owned screenshot payload with the established frontend instead of borrowing emulator pixels from the UI.
- [x] 🔊 Publish actual audio-device, submitted PCM, and dropped-buffer diagnostics alongside video frames.
- [x] 💾 Run final SRAM flush and PCM invalidation on the emulation owner thread before shutdown/join.
- [x] 📬 Process command bursts in bounded batches; surface fatal worker exits and queue cheat/save commands without stalling UI frames.
- [ ] 🔄 Replace all live `StarbyteApp.session` access with worker commands and snapshots.
- [ ] 📊 Move current emulation telemetry (FPS, frame gaps, overlays) to worker-produced truth.
- [ ] 🎚️ Preserve every current audio speed/pitch control and its per-mode behavior through the handoff.

## Milestone P: opt-in threaded preview parity — 15/18 source tasks (83%)

- [x] 🖼️ Display coalesced worker frames using egui textures without advancing the emulator from UI redraws.
- [x] 🎮 Reuse configured twelve-button keyboard bindings and support several gamepads with analog D-pad emulation.
- [x] ⏱️ Follow configured playback speed and independent audio-pitch defaults.
- [x] 💤 Apply background pause and compositor heartbeat policies separately from manual Pause.
- [x] 💾 Route memory quick saves/loads and three persistent disk slots through worker ownership.
- [x] 📬 Queue preview saves and loads asynchronously to avoid freezing menus.
- [x] 📸 Capture screenshots through the worker and write PNG files on a separate I/O thread.
- [x] 📊 Show actual worker FPS, hitches, CPU time and superseded video-frame counts.
- [x] 🎚️ Add worker-controlled mute, audio enable, volume and independent pitch-mode selectors.
- [x] 🧩 Honor the existing individual View → Status bar metric visibility choices.
- [x] 📂 Open a replacement cartridge asynchronously without blocking menus or discarding the old game on load failure.
- [x] 🔉 Add real device/sink diagnostics and a mute-aware 440 Hz speaker check.
- [x] 🎮 Show worker-produced hardware input and cheat-interception diagnostics.
- [x] ⏩ Use Tab for temporary configured fast-forward speed, with transition-only audio/scheduler updates.
- [x] 🛡️ Release stale gamepad holds on lost compositor focus and expose a worker-owned clear-cheats action.
- [ ] 🎼 Verify actual background audio quality and speaker/device recovery on Hyprland.
- [ ] 🧪 Verify preview menus, gamepads, screenshots and save slots natively on Hyprland.
- [ ] ✅ Confirm format, compile, Clippy and tests on the exact preview commit.

**Isolation:** The preview is explicitly selected with `--threaded-preview`. It is
not a separate repository, not the default UI, and not a substitute for the
complete Starbyte library application.

## Milestone C: native GUI migration — 6/14 source tasks (43%; default worker integration still 0%)

- [x] 🧱 Introduce frontend-neutral SessionControlsSnapshot and migrate the existing disk-slot menu to it; worker can serialize the same summary.
- [x] 🎮 Migrate the existing input overlay to frontend-neutral SessionInputDiagnostics that the worker can also publish.
- [x] 📸 Migrate the existing screenshot action to a shared owned SessionScreenshot payload that the worker can capture as well.
- [x] 🔊 Use shared frontend PCM metrics and sound diagnostics in both legacy app and worker, without direct emulator access from ordinary menus.
- [x] 🛡️ Isolate deep blackout-monitor emulator inspection behind the FrontendSession boundary for later migration into worker ownership.
- [x] 💾 Route the established F5/F8 and durable-slot actions through frontend-neutral SessionSaveAction; the worker uses the same action dispatcher for quick save/load, durable slots and SRAM.
- [ ] 🎬 Route game-launch success/error replies through the emulation worker.
- [ ] ⌨️ Send controller inputs without losing key-down/up edges; preserve F5/F8/F9/Escape.
- [ ] 🪟 Display the latest completed RGBA texture without blocking the worker.
- [ ] 🏠 Keep the full library, menus, recent covers and View status-bar toggles intact.
- [ ] ⏸️ Make manually paused / automatically paused / background-allowed states independent.
- [ ] 💾 Preserve original disk-slot, SRAM and sound-mode namespaces.
- [ ] 🧹 Complete full-application orderly shutdown handoff: stop clock, flush SRAM, clear audio and join exactly once (worker finalizer implemented and covered, not yet installed in normal StarbyteApp).
- [ ] 🚀 Make the worker the default runtime only after passing acceptance gates.

## Milestone D: validation — 3/9 test paths authored (33%), not yet verified

- [x] 🧪 Add a display-server-free, real built-in ROM smoke path (`--demo --scheduler-smoke`) to CI.
- [x] 🖥️ Add an opt-in `--threaded-preview` native egui shell and Weston smoke that requires a rendered texture.
- [x] 🧪 Add a `cargo test` built-in demo ROM integration test for background frames, neutral controls, screenshot capture and quick-save state.
- [ ] ✅ Confirm Linux CI format/Clippy and Linux+Windows tests pass.
- [ ] 🟩 Confirm the scheduler smoke actually completes on the exact commit.
- [ ] 🪟 Run Wayland window visible/focused and confirm accurate ~60.1 emulated FPS.
- [ ] 🌌 Switch to another Hyprland workspace; confirm continued emulation and sound when allowed.
- [ ] 💤 Repeat with background pause checked; verify true stop, no catch-up burst and clean resume.
- [ ] 🛡️ Confirm manual Pause, gamepad, cheats, screenshot, quick/disk saves and SRAM across restarts.

**Implementation note:** These source milestones measure API and preview
coverage, not demonstrated full-app readiness. Native StarbyteApp still owns
`FrontendSession` and renders emulated frames synchronously. The remaining
eight full-UI cutover tasks are blocking, not cosmetic. The workspace declares
Rust 1.85; newly introduced post-1.85 let-chain expressions were rewritten,
but a real MSRV build has not been performed in this environment.

## First confirmed CI evidence (2026-10-23)

The GitHub Actions run [39887087222](https://github.com/sguzman/starbyte/actions/runs/39887087222),
built from commit `6acc6d8`, compiled the entire workspace and passed the
Windows Rust tests, Linux Rust test suites, and the display-free
`--demo --scheduler-smoke`. Its actual test output included
`STARBYTE_SCHEDULER_SMOKE_OK background_frames=5 paused_at=5
total_frames=8 restored_frame=8`. This validates the experimental worker
clock, deliberate pause, resume, memory restore and PNG smoke **on that
specific commit**, not yet on the current tip of `main`.

That run also identified a failing **legacy** Weston smoke: the headless
window had no keyboard focus, so its default `pause_when_unfocused=true`
prevented progress until the 90-second timeout. The explicit bounded
`--exit-after-frames` test session now disables focus pause without changing
the default for real players. A newer green CI run is needed to establish the
fix. The source formatter failures in that run were addressed with exact
`rustfmt` diffs recovered from the lint logs.

The dedicated worker also now has a real lost-heartbeat test:
default focus pause must suspend frames when Wayland stops repainting,
while disabling focus pause must continue frames without any UI update.
Full Hyprland native performance and speaker audibility remain untested.

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

Developers can also launch the opt-in `--demo --threaded-preview` shell to
exercise real native input, video texture upload, frame execution and
background pause. It is deliberately **not** the full game library or the
default desktop app, and its normal functionality is not a substitute for
Milestone C. For Weston/headless acceptance, `--preview-background` explicitly
disables the preview's focus pause without mutating persisted preferences.

This headless path is a diagnostic of the new worker only. Ordinary
`cargo run --release -p starbyte-egui` **still runs the existing UI-owned
scheduler until Milestone C is complete**. Never describe this intermediate
state as a fixed background-pause regression.

## Source evidence

- `scheduler.rs`: dedicated clock, mailbox, command and transaction tests.
- `session_engine.rs`: worker-owned SNES session, audio, input port, periodic SRAM
  flush and framebuffer copying controlled by renderer-update heartbeats; built-in demo integration tests run without a compositor.
- `scheduler_video.rs`: renderer-only texture and telemetry adapter (not yet attached to the live GUI).
- `main.rs`: explicit headless worker smoke (including a memory quick-save/restore roundtrip, failed ROM load preservation and temporary PNG validation) and opt-in native-preview routes, no production default switch.
- `threaded_preview.rs`: opt-in worker-powered egui player with configured twelve-button keyboard/gamepad input, independent FPS, audio controls and speaker check, live clock policies, status visibility, asynchronous save/load, ROM replacement, disk slots and F12 screenshots.
- `preview_gamepads.rs`: multi-controller button and analog-axis input aggregation, independent of SNES frame pacing.
- `starbyte-frontend::SessionControlsSnapshot`: common typed summary of ROM, mode, quick-save, durable slots, and cheat-read diagnostics; adopted by the legacy slot menu as a low-risk UI migration seam.
- `starbyte-frontend::SessionInputDiagnostics`, `SessionSoundDiagnostics`, `SessionAudioFrameMetrics`, and `SessionScreenshot`: neutral hardware/audio readouts and owned RGBA capture shared by the established UI and the worker.
- `starbyte-frontend::SessionSaveAction`: one typed save/load/SRAM action dispatcher consumed synchronously by the current application and asynchronously by the worker. A built-in ROM test asserts compatibility and sound-mode disk slot isolation.
- `.github/workflows/ci.yml`: headless worker (including memory quick-save roundtrip) and Weston threaded-preview smoke tasks authored; completion not yet verified.
