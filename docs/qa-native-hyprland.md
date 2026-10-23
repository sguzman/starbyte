# Native Hyprland scheduler acceptance

**State:** acceptance procedure only. This document does **not** assert that the checks below have run or passed. It does not authorize switching the production runtime. Track accepted tasks in [scheduler-progress.json](scheduler-progress.json) and [scheduler-migration.md](scheduler-migration.md).

## Rules for the QA operator

- Execute in a real, GPU-backed Hyprland Wayland session. Headless Weston, XWayland and a green hosted CI job are not substitutes. Record the compositor, renderer, display, GPU, backend, Rust build and exact Starbyte commit.
- Use an isolated disposable config and cache. Do not edit the user's live settings or overwrite saves; never commit ROMs, game saves, screenshots from proprietary games or private machine paths.
- Automate the steps and collect logs/screenshots/metrics. Do not hand the operator's keyboard or terminal chores to the user. A real gamepad and audible-speaker checks must be classified **not tested** if no representative device or reliable listening evidence is available.
- The normal player remains the default. Run opt-in worker acceptance with `--native-worker-preview` until every prerequisite is accepted. Do not infer another item's success merely because one check passed.
- A passing synthetic demo proves only that demo and the tested paths; commercial-ROM behavior requires separately obtained local-only evidence.

## Environment receipt

Capture an evidence record *before* launching the application: Git SHA, build mode, `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, Hyprland version and monitor/workspace data from `hyprctl -j monitors` / `hyprctl -j workspaces`, GPU/renderer, audio server and device route, connected controller identification (no serial numbers), and the disposable config/cache locations. Record whether the process uses native Wayland. Redact personal filesystem paths before attaching evidence to a public issue.

## Automated native checks

| Gate | Execution | Required observation |
| --- | --- | --- |
| D-12: visible/focused clock | Launch the full original-window shell with `--demo --native-worker-preview` on physical Hyprland; measure the worker-produced frame count against monotonic elapsed time while the app is visible and focused | Stable rate near NTSC 60.1 FPS without recurrent long frame gaps; separately record texture-delivery age, GPU presentation and audio continuity. A fixed 60 Hz monitor is not itself proof of emulation-rate accuracy |
| A-08 / D-13: off-workspace | On a disposable workspace, run with background pause **disabled**, move focus to a different workspace, leave the emulation owner running without egui callbacks, then return | The owner gains approximately elapsed-time × game FPS frames without a burst or permanently held input. Verify video catches up on return and actual background sound is not silently dropped |
| D-14: background pause | Repeat the same workspace transition with pause-on-focus-loss **enabled**, then return | Frames stop while the policy suspends, audio is cleared, and resuming does not simulate the elapsed wall-clock interval in a burst; manual Pause remains independent |
| P-18: sound and device recovery | Use real output routing with game PCM, the shared-worker 440 Hz check, mute/unmute and speed/pitch changes; test a real device disconnect/reselection and restore | Record audio device state, submitted nonzero PCM, mixer drops/empty observations and an independent audibility/quality result. Opening the OS device or submitting PCM alone cannot establish speaker sound or recovery |
| P-19 / D-15: interaction and persistence | Exercise original-window menus, all mapped gamepad directions/buttons, cheats, keyboard shortcuts, screenshot capture, F5/F8, disk slots, mode namespace switches and battery SRAM across restart | Before/after evidence for each operation, including correct screenshot pixels, gamepad input press and release at the SNES port, slot contents, surviving save files, and no stale receipt replacing a newer game |
| C-16: production cutover | Only after the above and all other required acceptance checks pass, deliberately switch the default and repeat the entire user-facing regression suite | Old saved settings and saves remain compatible, native background behavior remains correct, and an intentional rollback is documented |

The existing deterministic flags such as `--exit-after-full-worker-no-redraw`, `--exit-after-full-worker-focus-pause`, `--exit-after-full-worker-sram`, `--exit-after-full-worker-shortcuts` and `--exit-after-full-worker-replacement` are useful **automated adjuncts** when launched inside the real Hyprland session. They are *not*, on their own, proof that physically switching workspaces, moving a real gamepad or listening through speakers succeeded. Preserve their distinct success markers and failure logs.

## Evidence and decision

For each gate, record `task_id`, `commit`, `time`, `environment`, `setup`, `action`, `observed`, `expected`, `result` (`pass` / `fail` / `not_tested`), and artifact pointers. Attribute native physical evidence and virtual Weston evidence separately. A fail or untested check leaves its checklist item pending. Update both the Markdown task and JSON ledger together only when the gate's direct evidence satisfies its wording; run the ledger parity test in CI. A passing run of unrelated tests cannot increase the accepted percentage.
