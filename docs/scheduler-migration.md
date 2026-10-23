# Emulation scheduler migration

**Status: in progress. Commit [`a0fbdd8`](https://github.com/sguzman/starbyte/commit/a0fbdd8cb10494352d6a782c447f814287d4c37a) passed [CI run 40007158503](https://github.com/sguzman/starbyte/actions/runs/40007158503): Linux, Windows, lint/Clippy and original-window native Weston acceptance, including worker-owned saves, battery SRAM, joypad integration, delayed screenshot receipts and async worker sound-diagnostics coverage. Default-app ownership transfer and physical Hyprland/audio validation remain outstanding.**
**Issue:** [#7](https://github.com/sguzman/starbyte/issues/7).
**Branch:** `main`. **Scope:** Rust/egui game-loop ownership and window-independent SNES timing.

## Verified original-window async status, sound modes and screenshot ownership — 2026-10-23

Commit [`a0fbdd8`](https://github.com/sguzman/starbyte/commit/a0fbdd8cb10494352d6a782c447f814287d4c37a)
passed [CI run 40007158503](https://github.com/sguzman/starbyte/actions/runs/40007158503)
with **Linux, Windows and lint/Clippy green**, including the existing
native Weston original-window tests. The new source/test coverage:

- **Delayed PNG completions:** an asynchronous screenshot retains its
  selected-cartridge generation through PNG encoding. A late completion
  still writes its requested image but cannot overwrite a newer game's UI
  status; the generation predicate has a focused regression.
- **Exclusive SRAM owner:** the original window skips its legacy UI-thread
  30-second SRAM flush whenever the independent worker owns the game.
  The worker already flushes on its own 30-second cadence, pause and
  shutdown; native battery-backed SRAM shutdown acceptance remains green.
- **Sound/save namespaces:** an actual worker-mode integration test
  switches the same built-in synthetic cartridge among persistent sound,
  compatibility and isolated diagnostic modes. Persistent and legacy
  disk-slot paths are distinct; switching back finds the appropriate
  slot; memory quick saves are invalidated; isolated diagnostic mode
  refuses durable saves.
- **SPC700 panel detail:** the original opt-in full-library sound panel
  no longer substitutes a generic placeholder for emulated SPC700
  diagnostics. It requests owned APU information asynchronously only
  while visible, at most once per second, and ignores replies from
  superseded ROM generations. A worker-side test verifies the typed
  query reflects sound-driver enablement across a mode replacement.

**Evidence boundary:** CI demonstrates compilation, unit/integration
coverage and native virtual-Wayland regression stability. It has *not*
visually exercised every open/close state of this sound panel or reproduced
a delayed PNG race during a native ROM replacement. Physical Hyprland
focus transitions, speaker audibility and full default runtime cutover
are still unverified. Source-task progress stays A 7/8 (88%), B 15/18
(83%), P 18/20 (90%), C 8/16 (50%), D 11/15 (73%). The established
legacy player remains the default.

## Verified worker input through SNES auto joypad — 2026-10-23

Commit [`f7114d9`](https://github.com/sguzman/starbyte/commit/f7114d9032356594e6250aab9a38d3ab1b42ed36)
passed [CI 40002095002](https://github.com/sguzman/starbyte/actions/runs/40002095002):
Linux, Windows and lint/Clippy green, including the native Weston GUI suite.

A new integration regression writes a copyright-free cartridge that enables
the SNES automatic joypad polling register (`$4200`). With emulation and its
input port owned by a separate worker thread, it submits Start+Right,
executes frames through the asynchronous one-frame worker command, then
checks **both host and actual emulated latched joypad bits**. It then submits
a released state and verifies both registers clear after subsequent frames.
This proves input reaches the actual virtual SNES hardware, rather than
only updating an egui diagnostic overlay or testing a bitset in isolation.

It does **not** verify physical keyboards/gamepads or Hyprland focus
behavior, and the default GUI has not switched owners.
Source counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%), C 8/16
(50%) and D 11/15 (73%).

## Verified battery-backed SRAM across original GUI shutdown — 2026-10-23

Commit [`66d7147`](https://github.com/sguzman/starbyte/commit/66d71471b7b7a1faf268ff8c75fd4205bab7e08c)
passed [CI 40000854342](https://github.com/sguzman/starbyte/actions/runs/40000854342):
**Linux, Windows and lint/Clippy all green**, including the original-library
native Weston smoke.

A new headless owner regression modifies a small copyright-free synthetic
SNES cartridge to expose battery-backed SRAM. The exclusive emulation worker
writes byte `0x5A` at bus address `0x700000`, with no GUI-side save call,
shuts down, flushes SRAM from the owner finalizer, then reopens the cartridge
to confirm the on-disk value survived.

A separate **native original Starbyte window** test exercises the real eframe
close path (not merely an isolated worker shutdown): the same synthetic
cartridge is launched through the opt-in complete library UI, a worker-owned
cheat writes the SRAM byte, the window closes, `StarbyteApp::drop` joins the
worker, and the executable reopens the persisted cartridge. Weston recorded
`STARBYTE_FULL_LIBRARY_SRAM_SMOKE_OK persisted_byte=5A`.

This verifies a real synthetic battery-backed SRAM write on *normal*
shutdown, not general behavior for all commercial games, sudden process
termination, power loss, storage failure or physical Hyprland. The source
ledger counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%), C 8/16
(50%), D 11/15 (73%); the original full-library worker path is still opt-in,
and the legacy player remains the default.

## Verified original-window owner failure reporting — 2026-10-23

Commit [`f201823`](https://github.com/sguzman/starbyte/commit/f2018235217838bff01647d8ff153dc6446b6f3b)
passed [CI 39998845470](https://github.com/sguzman/starbyte/actions/runs/39998845470)
with green Linux, Windows and lint/Clippy jobs. The opt-in **complete
original library window** now detects a finished emulation-owner thread,
reports a playback error even if the player was manually paused, and
does not repeatedly log the same fatal exit on subsequent redraws.

A new hidden native acceptance mode begins playback, injects a synthetic
panic into *only* the dedicated emulation owner, and waits for the ordinary
window to mark playback stopped. Linux Weston recorded
`STARBYTE_FULL_LIBRARY_WORKER_EXIT_SMOKE_OK playback_stopped=true`.
The earlier focus policy acceptance passed again in the same run:
`STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames=0 resumed_frames=11`.

This intentionally fatal test never runs in normal desktop launches.
It proves the original GUI sees an unexpected worker exit; it does **not**
prove recovery after an emulator crash, physical Hyprland off-workspace
sound quality, SRAM integrity during a panic, or default worker readiness.
Roadmap counts remain A 7/8 (88%), B 15/18 (83%), P 18/20 (90%),
C 8/16 (50%) and D 11/15 (73%). The legacy player remains the default.

## Verified native focus pause and background continuation — 2026-10-23

Commit [`88da6d7`](https://github.com/sguzman/starbyte/commit/88da6d7b4f542392ae670e346da39db6dac8af2a)
passed [CI run 39997413540](https://github.com/sguzman/starbyte/actions/runs/39997413540):
all three Linux, Windows and lint/Clippy jobs succeeded, including
the complete native Weston GUI smoke suite.

The scheduler publishes an atomic **effective running** flag from its sole
emulation-clock thread, after applying manual Play, focus state, background
pause preference and the GUI heartbeat policy. The publication resets even
when that worker unwinds from a panic. The original full-library window's
status bar, game overlay and Session diagnostics now read that effective
state rather than inferring it from UI frame cadence or stale FPS windows.

The newly added **original full-window** Weston focus-pause smoke starts
gameplay, applies the real worker focus-pause policy, confirms that the
owner is stopped, then deliberately withholds **every egui callback for two
seconds**. It resumes the owner through the same focus policy and demands
new game frames. The log records:

- `STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames=0 resumed_frames=11`
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121` in the separate background-allowed case

The focused scheduler unit regression additionally checks the effective
running flag across manual pause, focus loss and missing UI heartbeats. This
is **virtual Wayland/Weston evidence**, not physical Hyprland off-workspace
testing, and no claim about speaker audibility or device recovery follows
from it. All existing source-task counts remain A 7/8 (88%), B 15/18 (83%),
P 18/20 (90%), C 8/16 (50%) and D 11/15 (73%). The established synchronous
player remains the default; worker ownership in the complete library GUI
is still opt-in.

## Latest full-library worker integration — 2026-10-23

Commit [`08c5802`](https://github.com/sguzman/starbyte/commit/08c5802ee644b54e6327292212fb607e9615675a)
passed [CI 39994543237](https://github.com/sguzman/starbyte/actions/runs/39994543237)
with **all three jobs green** (Linux, Windows and lint/Clippy). Native Weston
logged:

- `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=31`
- `STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=83 slot=1 quick=true screenshot=true step_frames=60`
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121`

In the original library UI's opt-in worker path, the Session panel now uses
worker-produced FPS, frame gaps, emulation CPU, coalesced video frames and
audio-drop diagnostics rather than its unused legacy frame-loop counters.
The two frame-step buttons are disabled during exclusive cartridge
installation so their requests cannot be redirected to the newly installed
game. Library cheat refreshes no longer queue old cartridge patches behind
an in-flight replacement; successful and failed installation responses
resynchronize enabled cheats against whichever game is actually installed.

A real owner-thread regression queues a quick save and then reinstalls the
**same ROM path** with a different sound/save policy. It verifies that the
receipt from the earlier namespace must be filtered by the successfully
installed generation even though its ROM path still matches. The Windows
and Linux test suites passed this regression.

**Limits:** This is code and virtual-Wayland acceptance, not actual sound
audibility, physical Hyprland off-workspace behavior or production-default
cutover. Roadmap source-task counts stay at A 7/8 (88%), B 15/18 (83%),
P 18/20 (90%), C 8/16 (50%), D 11/15 (73%).

## Verified 60-step and save-generation checkpoint — 2026-10-23

Commit [`93d3720`](https://github.com/sguzman/starbyte/commit/93d3720b4abbb4d6720e5ef381dc66e0fff9d152)
passed all jobs in [CI 39991451457](https://github.com/sguzman/starbyte/actions/runs/39991451457).
The original library-window Weston acceptance advanced **60 contiguous, worker-owned,
paused steps**, checking each successive frame number and the restored still
texture before reporting
`STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=82 slot=1 quick=true screenshot=true step_frames=60`.
The independent no-egui-callback test separately advanced 122 frames in two
seconds. This is native virtual-Wayland evidence, not physical Hyprland validation.

Commit [`28fb308`](https://github.com/sguzman/starbyte/commit/28fb308244a0a5f0b89923b2d934ef7d95038ba7)
also passed Linux, Windows and lint in
[CI 39992289325](https://github.com/sguzman/starbyte/actions/runs/39992289325).
A save/restore receipt is now tagged with the **last successfully installed**
worker cartridge generation, not merely its ROM path. An acknowledgement from
an older cartridge or older sound/save namespace cannot repaint a newer worker
session, even when the path is the same. A failed replacement retains the active
generation, so receipts for the surviving original cartridge remain valid.
The generation guard has focused unit coverage and the full suite plus native
Weston passes; a deliberately interleaved same-ROM replacement/saving
**native race test is not yet available**.

The established player remains the default; the complete-window worker path
is still opt-in. The existing roadmap source counts remain A 7/8 (88%),
B 15/18 (83%), P 18/20 (90%), C 8/16 (50%) and D 11/15 (73%). Neither
virtual Wayland acceptance nor 60-frame stepping establishes real device
audio, physical Hyprland focus changes, or default-runtime cutover.

## Verified original-library worker parity checkpoint — 2026-10-23

The full native [CI run 39990152770](https://github.com/sguzman/starbyte/actions/runs/39990152770)
on source commit [`729aab9`](https://github.com/sguzman/starbyte/commit/729aab95024fb79a138a9cb4d1f93fb81bf0c43d)
**passed all Linux, Windows and lint jobs**, including the headless scheduler
and virtual-Wayland (Weston) integration. The original full-library window
(`--native-worker-preview`) emitted these native acceptance records:

- `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=30` — the original game view displayed worker-produced frames.
- `STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=23 slot=1 quick=true screenshot=true step=true` — worker-owned quick/disk save+restore, PNG screenshot and a **paused single-frame step** returned still video through the original window.
- `STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=121` — emulation continued for two seconds without egui callbacks.

The existing `Step Frame` / `Step 60 Frames` commands now submit asynchronous,
silent frame-step requests to the exclusive worker, one at a time. A unit
regression asserts a paused worker advances exactly one frame, returns an
RGBA still and produces no PCM submissions. Another worker-backed regression
associates installed direct and ZIP-extracted ROMs with library metadata even
though the opt-in full library window's legacy session is empty. An audio
control regression guards **pitch-only** changes at constant game speed.
Intermediate completed-frame mailboxes are drained after a newer explicit
launch/save/restore/step receipt to avoid repainting obsolete still video.

**Acceptance limits:** the new step native smoke exercises one paused frame;
it does not yet prove 60-frame sequence completion on physical Hyprland.
Emulation remains worker-owned only in the opt-in window. Audio audibility,
physical workspace switching, comprehensive gamepad/cheat/menu parity,
shutdown persistence under real gameplay, and the default-owner handoff
remain unverified. **Keep the source task counts unchanged** (A 7/8, B 15/18,
P 18/20, C 8/16, D 11/15); these additions are incremental evidence, not
the completion of their remaining acceptance gates.

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

## 2026-10-23 full-library worker integration (opt-in)

The **existing complete** `StarbyteApp` library shell now has a hidden
`--native-worker-preview` integration path. Unlike the smaller
`--threaded-preview`, it retains the original library, covers, menus, cheat
browser, recent games and status controls. Its own `FullLibraryWorker`
owns `OwnedSession`, accepts prepared typed launch requests, publishes
coalesced RGBA frames and menu snapshots, and handles worker input on the
independent clock. The original synchronous `FrontendSession` remains
present but does not execute game frames in that opt-in mode.

The native Weston test `STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK` reached 31
frames on [run 39981233316](https://github.com/sguzman/starbyte/actions/runs/39981233316).
That same run completed the original-window save sequence
`STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame=18 slot=1 quick=true`
for quick-save, quick-load, persistent disk slot save and disk slot restore.
Linux and Windows test jobs passed, but that commit's lint failed solely for
formatting. Its native Weston evidence is valid for the tested code at that
commit; it is **not** a clean full-pipeline result.

Further opt-in work includes worker-owned cheats, sound mode restart, audio
volume/mute controls, owner-side PCM diagnostics, a worker sound-check
receipt, and screenshot pixels captured by the emulator worker then encoded
to PNG by a separate I/O thread. A separate native Weston test verifies
PNG creation in the full-library path. CI verification for the newest
combined code remains tied to its specific commit and must not be inferred
from earlier passes.

**Acceptance boundary:** default runtime remains the original player.
These changes do not demonstrate physical Hyprland background audio, every
menu command, gamepad handling or a complete shutdown/SRAM regression across
real ROMs. Do not claim the Milestone C default cutover is complete until
the original full-library worker path and its remaining feature parity are
validated and the default switch is deliberately made.
