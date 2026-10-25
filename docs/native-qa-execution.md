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
The older `--hyprland-headless-smoke` also fails before launching with
`headless_hyprland_requires_private_drm_allocator`, since the pinned
headless backend returns no DRM FD. The experimental nested supervisor and
the obsolete headless-launch code were removed rather than pretending
that a successful parent Weston socket was sufficient.

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
`--weston-smoke` remains independently available and is now bounded
by a 20-second parent timeout, including compositor startup/teardown.
Its `wayland-info` protocol handshake is separately limited to four
seconds; a timed-out handshake cannot hang unattended QA. The no-GUI
Bubblewrap `--check-only` namespace audit is also capped at ten seconds. Bubblewrap's
private PID namespace and die-with-parent guard remain in force, and
the outer runtime cleanup continues to remove temporary artifacts.
These timeout safeguards are CI source/syntax-tested; a live hung
Weston process was not induced on the owner's machine.
CI verifies both Hyprland fail-before-launch paths without requiring
compositor packages.
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

### Hosted CI virtual GPU and keyboard model smoke (2026-10-25)

A dedicated, path-scoped GitHub Actions workflow runs
`scripts/qa-virtual-devices.sh --smoke` only when the QEMU probe or
its workflow changes (avoiding four-minute package downloads on
unrelated development commits).
with QEMU and QMP utilities. It rejects self-hosted and graphical sessions,
creates a private runtime, configures a **TCG-only, paused, bootless** VM
with 2D `virtio-gpu-pci` and `virtio-keyboard-pci`, and confirms both
named devices on the emulated PCI bus through bounded private QMP.
No host display, network, disk, physical input forwarding, or KVM is used.
The QMP exchange validates both actual command acknowledgments, even
if socat reports the expected socket reset as QEMU quits; transport
timeouts remain fatal. QMP quit and private socket teardown are mandatory.
Receipt:
`STARBYTE_VIRTUAL_DEVICE_MODELS_OK ... guest_booted=false`.

**Scope:** Device *models* exist inside the QEMU process. This is not
evidence of guest Linux DRM/render nodes, guest input focus, guest Hyprland,
Starbyte behavior, or real physical equipment. Booting a disposable guest
and verifying `CONFIG_DRM_VIRTIO_GPU` is the next evidence gate.

### Hosted private Linux guest PCI boot (2026-10-25)

The existing path-scoped virtual-device workflow now also boots a stock
Alpine Linux v3.23 virt kernel with an independently built BusyBox
initramfs. The guest serial console provides receipts that Linux
started userspace and enumerated the virtio GPU and keyboard PCI IDs.
QEMU uses TCG and no host display/input, network, disk or KVM.
The guest powers off and the host deletes all disposable artifacts.
The kernel SHA-256 is reported for future exact-artifact pinning.

This is a **guest-kernel PCI enumeration** test, not proof of a loaded
DRM driver, render node, evdev input seat, guest compositor, or physical
Starbyte acceptance. Those require separate positive checks.

### Next route: independently provisioned virtual rendering and input seat

**Superseded:** the earlier two-stage *software-only nested Weston →
Hyprland* proposal. Aquamarine v0.15.1 cannot use a parent Wayland socket
as a substitute for its required DRM-backed GBM allocator. This is proven
from upstream source above; do not retry the old path or bind-mount host
`/dev/dri` or `/dev/input` into its sandbox to make it start.

A possible replacement to **investigate, not yet implemented or verified,**
is a disposable, headless virtual machine:

1. Give a Linux guest its **own** virtual GPU/DRM subsystem, e.g. a QEMU
   `virtio-gpu` 2D device with a guest Mesa software renderer. Verify that
   its guest kernel supports `CONFIG_DRM_VIRTIO_GPU`, that it actually
   exposes a usable DRM/render node inside the guest, and that Hyprland can
   create its required allocator and renderer. None is assumed or accepted
   before a measured startup. QEMU's [VirtIO GPU documentation](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html)
   describes its 2D software-rendered option and guest prerequisites.
2. Keep the **entire** guest graphical environment, audio and input seat
   disconnected from the owner's active desktop: no host Wayland/X11 socket,
   Hyprland IPC, keyboard/mouse injection, DRM passthrough, home/config mounts,
   session bus or active-display window. Use a private VM control/input
   channel and disposable guest filesystem; demonstrate the host's workspace,
   focus and input remain untouched during a complete start/stop cycle.
3. Only after the independent render and input devices work, run a bounded
   in-guest Starbyte demo and verify clock, video, focus, synthetic input
   and teardown with machine-readable evidence. Keep the existing native
   QA wrapper and collector hard-stopped on the owner's physical compositor.
   VM evidence is **virtual**: it cannot by itself satisfy real speaker,
   physical controller, host Hyprland focus, or GPU scanout acceptance.

Require a dedicated supervisor to prove the guest is isolated, its display
and control channels belong to the guest, and every child/VM is reaped after
success, timeout or failure. Do not install permanent services, expose host
input devices, or launch test windows on the user's active desktop.
No virtual GPU, virtual keyboard seat, VM, or Starbyte guest run has yet
been verified. Weston-only headless/Pixman results remain preliminary.
**All six native acceptance gates are still open.**

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
