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

## Nested Hyprland allocator blocker (2026-10-25, confirmed in pinned source)

The previously proposed `--hyprland-nested-smoke` path is now **blocked
before any launch**, with the explicit receipt
`STARBYTE_ISOLATED_WAYLAND_BLOCKED nested_hyprland_requires_private_drm_allocator`.
The experimental nested compositor supervisor was removed rather than
pretending that a successful parent Weston socket was sufficient.

This is an upstream *capability incompatibility*, not a flaky startup
timeout. Aquamarine **v0.15.1** `CWaylandBackend::start()` requires the
`zwp_linux_dmabuf_v1` protocol and its `initDmabuf()` tries to open
the advertised DRM device. `CBackend::start()` then rejects any
non-null backend that does not supply a DRM-backed GBM allocator.
Starbyte's isolated namespace deliberately hides **all** host
`/dev/dri` and `/dev/input` devices, and the successful private
Weston baseline runs with headless/Pixman. A nested Wayland socket or
software-rendering environment flag does not satisfy Aquamarine's
render-node requirement.

Source references:
- [Wayland protocol requirements](https://github.com/hyprwm/aquamarine/blob/v0.15.1/src/backend/Wayland.cpp#L97-L150)
- [Opening the Wayland feedback DRM node](https://github.com/hyprwm/aquamarine/blob/v0.15.1/src/backend/Wayland.cpp#L370-L468)
- [GBM allocator hard gate](https://github.com/hyprwm/aquamarine/blob/v0.15.1/src/backend/Backend.cpp#L162-L179)

The next *genuine* prerequisite for an isolated Hyprland runtime is a
separately provisioned **virtual DRM/render device plus virtual input
seat** (such as in a fully isolated VM), or a future audited backend
with a different software allocator. This must never be implemented by
bind-mounting the owner's DRM devices into Bubblewrap. An isolated VM
itself still would not prove speaker audibility or physical gamepad
acceptance; those gates need separate device evidence. The Weston-only
`--weston-smoke` continues to work independently. CI verifies this
fail-before-launch path without requiring compositor packages.
Scheduler acceptance remains **71/77** with all six gates open.

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

## Isolated Wayland infrastructure (partially implemented)

The local investigation on 2026-10-24 did not start any experimental
compositor. Hyprland/Aquamarine are installed, Weston is not, and this host
has no independently assigned QA input seat. The presence of render-only DRM
nodes does not establish seat isolation. The `--check-only` wrapper completed
without workspace changes or GUI windows.

`scripts/qa-isolated-wayland.sh` is the bounded infrastructure entrypoint. Its
`--check-only` mode creates a private runtime directory and enters a
Bubblewrap process/mount namespace with a cleared environment, private `/run`
and `/tmp`, a minimal `/dev`, no network namespace access, and only read-only
system binaries. It checks that host display/session sockets and input/DRM
device directories are absent, host `DISPLAY` and `WAYLAND_SOCKET` are unset,
and the private Wayland socket is not present before compositor startup. It
does not alter or inspect the running compositor. The check passed locally on
2026-10-24. This verifies the namespace view, not compositor behavior.

The `--weston-smoke` path now has a local preliminary pass. The official Arch
`extra` Weston 15.0.1 package and `wayland-utils` package were signature
verified and extracted into a mode-700 temporary runtime directory; no system
package installation was performed. Set `STARBYTE_WESTON_PACKAGE_ROOT` to
that extracted package tree to run the same harness without installing it.
Only the package tree is mounted read-only inside Bubblewrap. Weston started
with the headless backend, Pixman renderer and kiosk shell. `wayland-info`
connected through the private socket and observed `wl_compositor`; the
packaged `weston-simple-shm` client ran under its bounded timeout. Weston was
reaped and the socket was absent afterward. Forced startup and client failures
and a SIGTERM interruption also left no compositor process, socket, or
temporary QA runtime directory.
This is preliminary Weston infrastructure evidence, not Hyprland or Starbyte
acceptance. Regression tests cover the source-level isolation and cleanup
guards. The existing wrapper and collector hard-stops remain authoritative.

For an unprivileged extraction, use pacman's configured official Arch mirror
and verify each detached signature before extracting:

```sh
private_dir="$(mktemp -d "$XDG_RUNTIME_DIR/starbyte-weston.XXXXXXXX")"
chmod 700 "$private_dir"
mkdir -p "$private_dir/pkg" "$private_dir/tree"
for package in weston wayland-utils; do
  url="$(pacman -Sp --print-format '%l' "$package" | awk -v name="$package" '$0 ~ "/" name "-[^/]+pkg[.]tar[.]zst$" {print; exit}')"
  archive="$private_dir/pkg/$package.pkg.tar.zst"
  curl --fail --location "$url" --output "$archive"
  curl --fail --location "${url}.sig" --output "$archive.sig"
  pacman-key --verify "$archive.sig" "$archive"
  bsdtar -xf "$archive" -C "$private_dir/tree"
done
STARBYTE_WESTON_PACKAGE_ROOT="$private_dir/tree" bash scripts/qa-isolated-wayland.sh --weston-smoke
```

Remove `private_dir` after the smoke. This does not install or update host
packages.

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

No nested Hyprland compositor or private virtual input seat has been
verified. In particular, there is no evidence yet that Hyprland can run
nested under the isolated Weston socket, select only the nested Wayland
backend, or accept virtual-keyboard input without host session interaction.
Starbyte GUI probes remain blocked.

### Standalone headless Hyprland blocker (2026-10-24)

Installed Hyprland 0.56.2 / Aquamarine 0.15.1 was attempted once in the
existing Bubblewrap sandbox with a disposable config, private runtime, no
host sockets, no `/dev/input` or `/dev/dri`, cleared session variables,
software-rendering environment, and a bounded startup wait. The process
aborted during startup (exit status 134); no readiness or private IPC marker
was produced. The host-side temporary QA runtime directory was removed, and
no core artifact was left in the checkout. The private raw log was removed
with that runtime before its cause could be classified, so the exit status
alone is not treated as proof of a particular runtime error.

The source-level blocker is specific: Aquamarine's
[`CHeadlessBackend::drmFD()` returns `-1`](https://github.com/hyprwm/aquamarine/blob/v0.15.1/src/backend/Headless.cpp#L112-L128),
and its
[`CBackend::start()` requires a render allocator](https://github.com/hyprwm/aquamarine/blob/v0.15.1/src/backend/Backend.cpp#L148-L164)
unless the first backend is Null. With all DRM nodes excluded and Hyprland
not requesting Null, the mandatory headless backend alone cannot provide the
allocator. The same build's OpenGL renderer derives its DRM FD from the
backend and then attempts EGL-device/GBM initialization from that FD
([`OpenGL.cpp`](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/render/OpenGL.cpp#L266-L327)).
Hyprland's [virtual-GPU documentation](https://wiki.hypr.land/configuring/extra/virtual-gpu/)
describes a `HEADLESS-0` output, but that output capability does not remove
the allocator/renderer requirement. No host DRM device may be exposed to
work around it.

The isolated-harness mode remains fail-closed and reports startup failure,
timeout, and teardown markers without changing native GUI hard-stops. Because
the real process aborted before readiness, a successful normal shutdown and
the timeout cleanup branch were not exercised against a live compositor.
Neither a private IPC identity nor a headless output was verified.

The isolation preflight now also asserts that the host Hyprland IPC runtime
and personal config path are unreachable and that session-manager / Hyprland
instance environment markers are absent. This strengthens namespace
regression coverage; it does not validate compositor or input-seat behavior.

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
