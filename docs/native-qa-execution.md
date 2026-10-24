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

## Candidate isolated-compositor design (not implemented or verified)

The local investigation on 2026-10-24 did not start any experimental
compositor. Hyprland/Aquamarine are installed, Weston is not, and this host
has no independently assigned QA input seat. The presence of render-only DRM
nodes does not establish seat isolation. The `--check-only` wrapper completed
without workspace changes or GUI windows.

The next design to validate is a two-stage, software-rendered nested setup:

1. Start Weston with its documented `headless` backend and Pixman renderer in
   a dedicated process/mount namespace. Give it a fresh private
   `XDG_RUNTIME_DIR` and socket name. Do not expose the owner's Wayland socket,
   `/dev/input`, DRM primary nodes, systemd user-manager socket, or D-Bus
   sockets to that namespace. Weston documents the headless backend as having
   no input or output; this is preliminary infrastructure evidence only, not
   Hyprland or Starbyte acceptance.
2. Start Hyprland as a nested Wayland client of that private Weston socket,
   with an ephemeral config and private runtime path. Require startup evidence
   that Aquamarine selected the Wayland backend. Deny access to all host input
   and DRM devices. Set `HYPRLAND_NO_SD_VARS=1`, and keep systemd/D-Bus sockets
   inaccessible, so a child compositor cannot import or unset the owner's
   activation environment.
3. Launch Starbyte with only the nested Hyprland socket in its environment.
   Provide input only through a virtual-keyboard protocol client connected to
   that private socket. Confirm the focused Starbyte surface and key events
   through the nested compositor's private IPC, never through the owner's
   compositor. Ensure all compositor and test processes are reaped on failure.

The DRM kernel documentation says render nodes do not permit modesetting or
DRM-master operations, but they still share GPU execution resources. If the
Wayland-nested design cannot start without opening a host DRM or input device,
stop and require a separately provisioned VM or physically separate seat; do
not fall back to the owner's compositor. Hyprland's `AQ_DRM_DEVICES` and
`AQ_NO_KMS_REQUIREMENT` options alone are not an isolation boundary. The
`HYPRLAND_NO_SD_VARS` guard is necessary but not sufficient without hiding
session-manager sockets.

References for the proposed boundaries: [Weston's backend and renderer
documentation](https://wayland.pages.freedesktop.org/weston/toc/running-weston.html),
[Hyprland environment variables](https://wiki.hypr.land/configuring/core/environment-variables/),
[Aquamarine device selection](https://github.com/hyprwm/aquamarine/blob/main/docs/env.md),
[Linux DRM render-node guarantees](https://docs.kernel.org/6.11/gpu/drm-uapi.html),
and [Hyprland's backend and virtual-keyboard protocol registration](https://github.com/hyprwm/Hyprland/blob/main/CMakeLists.txt).

Before allowing any GUI probe, a dedicated supervisor and regression tests
must prove: the socket and environment are private; the sandbox cannot open
host input or DRM primary nodes; no host Hyprland IPC is reachable; compositor
startup selects only the nested Wayland backend; a virtual key focuses and
operates only Starbyte inside that compositor; and all child processes stop
on success, timeout, and failure. Weston results must remain labeled
preliminary. Even a verified nested Hyprland run would not establish physical
monitor, GPU scanout, audible-device, or hardware-gamepad acceptance.

No such sandbox or test was available in this checkout/session, so this
design is not permission to start a compositor. The existing wrapper and
collector hard-stops remain authoritative until the complete safety proof and
regression coverage exist.

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
