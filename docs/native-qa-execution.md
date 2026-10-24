# Native Hyprland QA: focus-protected execution

## Current safety state (2026-10-24)

**GUI QA is suspended on the owner's active desktop.** Two local Codex runs
demonstrated that moving a test window to an isolated workspace still
interrupted the owner's work by *switching workspace and taking focus*.
That directly violates the owner's requirement. The second run also failed
its actual foreground/native-client acceptance check.

The wrapper `scripts/qa-native-hyprland.sh` now fails closed before any GUI
window, compositor command, or focus change. This applies to both its former
12-probe mode and `--focused-only`. The Rust collector itself has a separate
hard-stop, including for direct `starbyte-native-qa --physical-workspace`
invocations. **Do not bypass either guard** by invoking the game binary
directly or restoring the former code from git.

A dedicated workspace is *not* an isolated seat. A focused original-window
probe cannot simultaneously hold the current user's keyboard focus and
leave that user's single-seat interactive session uninterrupted. New
`workspace silent` / `no_initial_focus` rules might let a window open
without a focus change, but that would not satisfy the focused-window
requirement by itself.

## Safe QA today

An authorized local Codex executor may run:

`bash scripts/qa-native-hyprland.sh --check-only`

It builds the existing GUI/QA binaries and runs the collector's regression
tests and the scheduler-ledger test with private local logs, without spawning
a window, changing workspace, or requesting focus. The regular hosted CI
already checks native Weston virtual-compositor behavior, Linux and Windows
workspace tests, Rust formatting, Clippy and compiler floor. These results
are **not** physical Hyprland, audible-device or gamepad evidence.

A separate compositor **and separate input seat** are required before
unattended GUI acceptance can resume. That solution must be tested to prove
zero effect on the owner's active desktop before running a single probe.
Do not install permanent services, change Hyprland config, ask the owner to
manually test, or override the safeguard.

## Evidence already obtained

- Native full harness on `b1448b3`: 11/12 pass. Nine original-window
  demo smokes and two *real* Hyprland workspace transitions passed.
  A-08: 142 worker frames, 59.81 FPS over 2,374 ms; nine compositor
  observations for 2,289 ms. D-14: two frames away, one suspension, one
  resume, ten resumed frames in 166 ms. Original workspace restored;
  no residual windows or child processes.
- Targeted focused-clock retest on `fd522d4`: **0/1** verified.
  The unverified application log reported 60.006 FPS, 181 completed
  frames, 181 presented texture frames and 3,016 ms. The collector
  rejected active-window/native-client verification. The user also
  confirmed this run forcibly switched focus. Workspace was restored,
  no process/window remained, and checkout was clean.
- Accepted ledger remains **71/77 (92%)**. A-08 and D-14 are accepted;
  focused D-12, background sound, physical device/controller, GUI/save
  persistence and production cutover remain pending. These cannot be
  inferred from the measured 60 FPS alone.

## GitHub and privacy

`.github/workflows/native-hyprland.yml` historically used a temporary
self-hosted runner for physical probes. Until independent focus-safe
isolation exists it is intentionally blocked by the wrapper too.
Do **not** dispatch repeated jobs or attach runners to the user's live
desktop as a workaround. Physical raw logs, device routes, paths, ROMs,
window titles, and saves must remain private. An unassigned or blocked
job is not successful evidence.
