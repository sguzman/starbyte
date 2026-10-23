# Emulation scheduler migration

**Status: in progress. Native Weston worker playback and a two-second no-egui-callback interval (121 continued SNES frames) passed on commit `135cf2e` (Linux run 39962498642). Full original-app ownership transfer and physical Hyprland/audio validation remain outstanding.**
**Issue:** [#7](https://github.com/sguzman/starbyte/issues/7).
**Branch:** `main`. **Scope:** Rust/egui game-loop ownership and window-independent SNES timing.

## Why a worker, not another repaint timer

The current player runs `FrontendSession::run_frame_with_timeout` inside
`eframe::App::update`. When Hyprland/Wayland withholds a hidden window's
presentation callbacks, a queued egui repaint may not run the application
again. Disabling `pause_when_unfocused` alone does not create a game clock.
The correct long-term owner of ROM state, frame timing and save mutations is a
dedicated emulation thread. The renderer merely consumes completed frames.

## Milestone A: clock and ownership foundation — 7/8 source tasks (88%)

- [x] ⏱️ Move deadline pacing into `scheduler.rs` with a dedicated OS thread.
- [x] 🎮 Give the worker exclusive ownership of a generic machine object.
- [x] 🖼️ Replace an unbounded video queue with a one-frame mailbox.
- [x] 💤 Add independent Play, Focus, Background-pause and Speed commands.
- [x] 🛑 Make a failed emulation frame halt the worker and expose its error.
- [x] 📦 Bound outstanding asynchronous operations to 128 without blocking the GUI or losing separately ordered policy commands.
- [x] 🧪 Confirm format, Clippy and full workspace tests on code commit `d0254a7` (CI run 39951239026).
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

## Milestone P: opt-in threaded preview parity — 18/20 source tasks (90%)

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
- [x] 📚 Start the opt-in preview with no cartridge, browse installed games from the real asynchronous library index, and launch each selected ROM/ZIP plus matching cheats on the sole emulation owner.
- [x] 🔎 Search the installed library while rendering only visible rows; disable save/pause controls until a cartridge is installed.
- [ ] 🎼 Verify actual background audio quality and speaker/device recovery on Hyprland.
- [ ] 🧪 Verify preview menus, gamepads, screenshots and save slots natively on Hyprland.
- [x] ✅ Confirm format, compile, Clippy and native Weston preview smoke at code commit `d0254a7`.

**Isolation:** The preview is explicitly selected with `--threaded-preview`. It is
not a separate repository, not the default UI, and not a substitute for the
complete Starbyte library application.

## Milestone C: native GUI migration — 8/16 source tasks (50%; default worker integration still 0%)

- [x] 🧱 Introduce frontend-neutral SessionControlsSnapshot and migrate the existing disk-slot menu to it; worker can serialize the same summary.
- [x] 🎮 Migrate the existing input overlay to frontend-neutral SessionInputDiagnostics that the worker can also publish.
- [x] 📸 Migrate the existing screenshot action to a shared owned SessionScreenshot payload that the worker can capture as well.
- [x] 🔊 Use shared frontend PCM metrics and sound diagnostics in both legacy app and worker, without direct emulator access from ordinary menus.
- [x] 🛡️ Isolate deep blackout-monitor emulator inspection behind the FrontendSession boundary for later migration into worker ownership.
- [x] 💾 Route the established F5/F8 and durable-slot actions through frontend-neutral SessionSaveAction; the worker uses the same action dispatcher for quick save/load, durable slots and SRAM.
- [x] 🖼️ Share validated nearest-filtered RGBA texture uploads between the legacy player and threaded preview, without adding an intermediate copy to the legacy loop.
- [x] 🎬 Share one typed ROM launch request (resolved path, sound/save mode and matching cheats) across library, direct-file, startup, sound-mode restart and worker APIs, without executing a frame between installation and cheat selection.
- [ ] 🎬 Route game-launch success/error replies through the emulation worker.
- [ ] ⌨️ Send controller inputs without losing key-down/up edges; preserve F5/F8/F9/Escape.
- [ ] 🪟 Display the latest completed RGBA texture without blocking the worker.
- [ ] 🏠 Keep the full library, menus, recent covers and View status-bar toggles intact.
- [ ] ⏸️ Make manually paused / automatically paused / background-allowed states independent.
- [ ] 💾 Preserve original disk-slot, SRAM and sound-mode namespaces.
- [ ] 🧹 Complete full-application orderly shutdown handoff: stop clock, flush SRAM, clear audio and join exactly once (worker finalizer implemented and covered, not yet installed in normal StarbyteApp).
- [ ] 🚀 Make the worker the default runtime only after passing acceptance gates.

## Milestone D: validation — 11/15 checkpoints (73%; Linux no-redraw acceptance passed on `135cf2e`, complete run verification pending)

- [x] 🧪 Add a display-server-free, real built-in ROM smoke path (`--demo --scheduler-smoke`) to CI.
- [x] 🖥️ Add an opt-in `--threaded-preview` native egui shell and Weston smoke that requires a rendered texture.
- [x] 🧪 Add a `cargo test` built-in demo ROM integration test for background frames, hidden-window copy suppression and reappearance, neutral controls, screenshot capture and quick-save state.
- [x] 🎬 Add direct/worker launch-request tests for failed ROM preservation, restored save-mode namespaces and memory quick-save invalidation after a successful ROM replacement.
- [x] 🔗 Add a compositor-free integration test from an installed library entry through ROM materialization, exclusive worker launch, unfocused gameplay frames and a worker-owned quick save.
- [x] ✅ Confirm Linux CI format/Clippy and Linux+Windows tests pass on `d0254a7`.
- [x] 🟩 Confirm the headless scheduler smoke and both native Weston smokes complete on `d0254a7`.
- [x] 🎮 Test the opt-in library selection coordinator from installed entry through asynchronous ROM preparation, owned-session launch, still-frame upload and compositor-independent frame progress (Linux test run on `adddfa9`).
- [x] 🪟 Add and pass a dedicated native Weston smoke for `--threaded-preview --exit-after-library-scan` without a startup ROM, with a real indexed built-in SNES demo file (Linux CI run 39955310386).
- [x] 🎮 Add and pass native Weston indexed-library game launch: scan the local demo, select the installed entry through the ROM preparation worker, install it on the exclusive SNES worker, and upload 30 rendered egui frames (`STARBYTE_THREADED_LIBRARY_PLAY_SMOKE_OK`; run 39959433627).
- [x] 🌙 In native Weston, suppress egui callback work and heartbeats for two seconds with background pause disabled; verify the worker advances at least 45 independent frames (observed 121 on commit `135cf2e`, run 39962498642).
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
the default for real players. Source formatter failures were addressed with
exact `rustfmt` diffs recovered from the lint logs.

The subsequent [**green run 39887694659**](https://github.com/sguzman/starbyte/actions/runs/39887694659)
at commit `4be0094` passed `cargo fmt`, Clippy, Windows and Linux tests, the
headless worker smoke, and both native Weston checks. Its verified output
included `STARBYTE_NATIVE_SMOKE_OK frame=30` (legacy window) and
`STARBYTE_THREADED_PREVIEW_SMOKE_OK worker_frames=30 rendered_frame=30`
(threaded window + real texture upload). **This is tested implementation
evidence at that SHA**, not a claim that the default library has been
migrated, nor that the newest commit or a Hyprland desktop is validated.

The dedicated worker also now has a real lost-heartbeat test:
default focus pause must suspend frames when Wayland stops repainting,
while disabling focus pause must continue frames without any UI update.
Full Hyprland native performance and speaker audibility remain untested.

**Latest source note:** The normal application's `WorkerEvent::RomReady`,
file/ZIP open, sound-mode restart and startup ROM flows now use the same
`SessionLaunchRequest` type as `OwnedSession::queue_launch_with_frame`.
This reduces divergence in cheat timing and save namespaces but **does not**
move the legacy app's frame clock or `FrontendSession` to another thread.
Outstanding cutover tasks remain blocking. The command-burst test now
explicitly accepts quota-based backpressure (instead of assuming an
unbounded queue), and CI formatter corrections have been applied.

## Partial handoff interfaces (2026-10-23)

The independent session owner now exposes a single nonblocking save/restore
receipt with the authoritative `SessionControlsSnapshot` and a restored
still framebuffer when applicable. A successful launch can similarly return
`WorkerLaunchResponse` containing both the new cartridge's menu state and
a still frame. The opt-in threaded preview uses these APIs, disables
unavailable quick/disk loads, and refreshes availability from acknowledgements
without synchronous emulator inspection.

The existing full-library ROM-materialization worker now prepares
`SessionLaunchRequest` (resolved path, persistent sound/save namespace and
selected game's cheats) before returning it to `StarbyteApp`. This keeps
request formation out of the GUI event callback and prepares the same
request for the emulation owner. **The established `StarbyteApp` still applies
the request to its UI-owned `FrontendSession`; its frame loop has not
been moved.** No Milestone C checkbox is claimed for these preparatory
interfaces, and source percentages remain unchanged until the actual
worker-owned full application cutover passes acceptance.


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
- `video_upload.rs`: the same framebuffer upload boundary is already used by the established local game view and opt-in worker renderer.
- `main.rs`: explicit headless worker smoke (including a memory quick-save/restore roundtrip, failed ROM load preservation and temporary PNG validation) and opt-in native-preview routes, no production default switch.
- `threaded_preview.rs`: opt-in worker-powered egui player with configured twelve-button keyboard/gamepad input, independent FPS, audio controls and speaker check, live clock policies, status visibility, asynchronous save/load, ROM replacement, disk slots and F12 screenshots.
- `preview_gamepads.rs`: multi-controller button and analog-axis input aggregation, independent of SNES frame pacing.
- `starbyte-frontend::SessionControlsSnapshot`: common typed summary of ROM, mode, quick-save, durable slots, and cheat-read diagnostics; adopted by the legacy slot menu as a low-risk UI migration seam.
- `starbyte-frontend::SessionInputDiagnostics`, `SessionSoundDiagnostics`, `SessionAudioFrameMetrics`, and `SessionScreenshot`: neutral hardware/audio readouts and owned RGBA capture shared by the established UI and the worker.
- `starbyte-frontend::SessionSaveAction`: one typed save/load/SRAM action dispatcher consumed synchronously by the current application and asynchronously by the worker. A built-in ROM test asserts compatibility and sound-mode disk slot isolation.
- `.github/workflows/ci.yml`: headless worker (including memory quick-save roundtrip) and Weston threaded-preview smoke tasks; all passed on `d0254a7`.

## 2026-10-23 continuation — asynchronous normal-app ROM preparation

The established library UI now dispatches direct ROM/ZIP preparation through
its dedicated ROM worker, instead of extracting ZIP archives in egui callbacks.
Prepared `SessionLaunchRequest` values preserve the selected game's
cheats and persistent sound/save mode, and are applied only after a successful
worker reply. Outdated selection replies/errors cannot replace the newest
selected game; queued superseded ROM jobs are skipped before ZIP extraction.
The legacy `StarbyteApp` still owns and advances `FrontendSession` after
materialization. This is **not** the complete ownership handoff.

Worker-only blackout/CPU/PPU diagnostics now observe `FrontendSession` on the
emulation thread, including when Wayland does not repaint. Worker shutdown
finalizes SRAM and releases the audio output on that owning thread instead
of flushing SRAM a second time when the returned owner is dropped on the UI
thread. The headless library-to-worker integration test exercises the full
materialization, cartridge launch, background clock and quick-save command
chain using the built-in copyright-free demo. Bounded nonblocking diagnostic
publication now writes QA reports on a separate I/O thread, dropping reports
rather than stalling emulation if the two-item queue fills.

**Verified 2026-10-23:** [CI run 39951239026](https://github.com/sguzman/starbyte/actions/runs/39951239026)
at code commit `d0254a7` passed Rust formatting, Clippy, all Linux and Windows
workspace tests, benchmark compilation on both platforms, the independent
background scheduler smoke and native Weston legacy + threaded-preview checks.
This verifies the new worker-side integrations and existing opt-in native
preview under CI, but does **not** establish real Hyprland focus/background
audio behavior or worker-backed execution in the full `StarbyteApp`.

## 2026-10-23 opt-in library bridge

The threaded preview can now boot without a cartridge (`--threaded-preview`),
scan the same configured installed-library sources as Starbyte's regular UI
using `AppWorker`, display a filtered, virtualized list, and pass a selected
entry's typed ROM/cheat launch request directly to the exclusive emulation
worker. Normal `StarbyteApp` still owns its own `FrontendSession`: this
feature exercises a working library-to-worker boundary but does **not**
fulfill Milestone C's main-UI handoff. The `--exit-after-library-scan` flag
is reserved for bounded native Wayland CI acceptance, and is not a normal
player option. Native Hyprland scheduling, audio quality, and complete GUI
feature parity remain mandatory before default-runtime migration.

## Latest worker-owned preview startup change

Commit [`407f30f`](https://github.com/sguzman/starbyte/commit/407f30f637929309683e2a31bcde4b94ff87596f)
now queues preview startup `--rom` and direct File → Open ROM requests
through the dedicated ROM preparation worker, including ZIP handling, before
submitting the typed launch to the emulation owner. The native UI constructor
no longer synchronously installs a cartridge or loads its SRAM. This also adds
two regression tests for startup and direct-file preparation. **These new
source changes are pending their own CI result**; the verified expanded CI
checkpoint below applies specifically to the preceding `adddfa9` SHA.

## Verified expanded CI checkpoint (2026-10-23)

[Run 39955310386](https://github.com/sguzman/starbyte/actions/runs/39955310386)
for `adddfa9` passed format, Clippy, Linux/Windows tests and benchmark
compilation, independent-scheduler smoke, legacy and threaded-preview native
Weston smokes, and the no-startup-cartridge indexed-library Weston smoke.
This does **not** prove physical Hyprland off-workspace playback, speaker
quality or full original library-GUI parity/default readiness.

## 2026-10-23 native indexed-library playback evidence

The fully green [run 39959433627](https://github.com/sguzman/starbyte/actions/runs/39959433627)
from `3a808ff` passed Linux and Windows tests, formatting, Clippy,
headless worker scheduling, and native Weston smokes. In particular, the
`--threaded-preview --preview-background --exit-after-library-play` check
started with **no ROM**, scanned the built-in demo from `--rom-dir`, selected
that indexed game on the asynchronous ROM-preparation lane, and launched it
through the exclusive emulation worker. The log reported
`STARBYTE_THREADED_LIBRARY_PLAY_SMOKE_OK worker_frames=30 rendered_frame=30`.
This is stronger than the earlier index-only smoke: the worker ran and egui
uploaded actual resulting video in one native UI session. It is **not**
physical Hyprland/audio acceptance or proof that the original full-library
`StarbyteApp` has transferred ownership. The subsequent held/press input
sampling separation is preparatory source work and remains unverified until
its own CI completes; it does not close Milestone C's input handoff.

## 2026-10-23 worker install handoff and missing-redraw regression

A new preview launch request arriving while the worker is installing a
cartridge now replaces the pending-next slot rather than returning a busy
error. The preparation lane remains independent; acknowledgements and video
from superseded intermediate installations are not painted. Save/load,
screenshots, cheat changes and pause controls are unavailable while an
exclusive owner install is pending, avoiding cross-cartridge action races.
Regression tests exercise preparation during installation, back-to-back
worker launches, and the save guard. These changes do **not** transfer the
original full-library `StarbyteApp` to the worker.

A further native Weston test launches the built-in demo with background
pause explicitly disabled, then deliberately stops processing egui callbacks,
input and display polling for two seconds. The log on commit `135cf2e`
reported `STARBYTE_NO_REPAINT_SMOKE_OK advanced_frames=121`. That is
evidence for compositor-callback-independent game progression in virtual
Wayland, **not** proof of physical Hyprland focus transitions or speaker
output. The default legacy application is unchanged.
