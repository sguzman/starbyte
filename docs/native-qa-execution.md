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

## Dedicated Hyprland guest prerequisite (2026-10-25)

The virtual QEMU environment now supplies independently emulated DRM, a
GBM allocation path and a udev-classified keyboard seat. The opt-in
`--hyprland-smoke` probe stages a packaged Hyprland binary, matching
runtime libraries and public shaders into guest RAM, shuts down the
successful Weston baseline, and starts Hyprland using **only** the VM's
virtio DRM/input devices. It requires the native Wayland protocol to
advertise both `wl_compositor` and `wl_seat` and reports fixed diagnostic
categories on failure. No raw compositor logs or personal files are
uploaded, and native physical-QA fail-closed guards are unchanged.

This probe is **not accepted until its hosted CI job passes**.
[Run 40242678493](https://github.com/sguzman/starbyte/actions/runs/40242678493)
confirmed guest Weston, DRM, udev keyboard and focused Escape, but Hyprland
exited before publishing a Wayland socket. Its documented root privilege
override was applied **inside the VM only**.
[Run 40243105431](https://github.com/sguzman/starbyte/actions/runs/40243105431)
then observed a live Hyprland process and private Wayland socket, but no
confirmed `wl_compositor` / `wl_seat` registry roundtrip; a bounded
configuration-error category was also present. [Run 40243529869](https://github.com/sguzman/starbyte/actions/runs/40243529869)
then **passed**: guest Hyprland remained alive, its configuration
verified, and the second bounded registry roundtrip saw both
`wl_compositor` and `wl_seat`. That establishes an isolated Hyprland
virtual DRM/seat prerequisite. The follow-on test additionally requires
a real xdg-toplevel to receive private-QMP Escape while Hyprland is
focused; it remains unaccepted until the matching CI run passes.

### Original Starbyte-in-Hyprland integration: unresolved

[Run 40244047890](https://github.com/sguzman/starbyte/actions/runs/40244047890)
first attempted the original Starbyte GUI and exclusive emulator worker inside
the private Hyprland guest. The GUI exited with status 1 before any accepted
worker frames or single-owner receipt; the focused sample did not become ready.
[Run 40245510283](https://github.com/sguzman/starbyte/actions/runs/40245510283)
and [run 40246534678](https://github.com/sguzman/starbyte/actions/runs/40246534678)
classified frontend rendering/surface initialization failure, with neither
keyboard-enter nor XDG configure for the sample. Its generic successful exit
and "simple-shm exiting" string do not establish focused Escape delivery.

[Run 40247126854](https://github.com/sguzman/starbyte/actions/runs/40247126854)
confirmed that the compositor advertises wl_output, xdg_wm_base and wl_shm,
with no classified monitor error.
[Run 40247720651](https://github.com/sguzman/starbyte/actions/runs/40247720651)
narrowed the original application failure to initialization before
wl_surface.attach: no XDG configure, no DMA-BUF submission or buffer-import
rejection, and no worker frames. This does **not** conclusively identify the
underlying error; do not ascribe it to missing output or DMA-BUF import.
Standard Linux, Windows, lint and compiler-floor CI passed for this code in
[run 40247720696](https://github.com/sguzman/starbyte/actions/runs/40247720696).
The separate Weston full-GUI/worker workflow also previously passed.

[Run 40249400601](https://github.com/sguzman/starbyte/actions/runs/40249400601)
and [run 40250100222](https://github.com/sguzman/starbyte/actions/runs/40250100222)
narrowed the eframe return error to Glutin's `initialization_failed`, which
[pinned Glutin 0.32.3 maps to `EGL_NOT_INITIALIZED`](https://github.com/rust-windowing/glutin/blob/v0.32.3/glutin/src/api/egl/mod.rs#L106).
This is an EGL initialization failure, **not a Rust panic**: the earlier
panic category overcounted a returned error's diagnostic text. The
focused sample also exited without proof of XDG configure or keyboard enter.

[Run 40250916583](https://github.com/sguzman/starbyte/actions/runs/40250916583)
tested a DRM-matched `kms_swrast` client instead of `swrast`; EGL still
returned `EGL_NOT_INITIALIZED`. Mesa debug logging identified no narrower
DRI2 error category. That failed override was reverted to the original
guest-client setting. Neither the Wayland registry check nor the separate
successful Weston-based virtual regression implies native Hyprland GUI
acceptance. [Virtual-device run 40250916586](https://github.com/sguzman/starbyte/actions/runs/40250916586)
passed, as did [standard Linux, Windows, lint and MSRV CI
40250916608](https://github.com/sguzman/starbyte/actions/runs/40250916608)
for the driver experiment commit. The cleanup commit must be checked on
its own CI run before claiming verification of that revision.

The remaining **virtual** prerequisite is a configured and focused Hyprland
client surface, followed by the unchanged original Starbyte 30-frame
exclusive-worker smoke. Do not weaken the gate, expose the active host DRM
or input devices, or treat virtual proof as physical/cutover acceptance.

A runner-packaged Hyprland version different from the owner's 0.56.2
cannot establish exact-version parity; even a successful virtual compositor
cannot establish physical focus, audible audio, gamepad input or the
production-default worker cutover. All six scheduler gates remain open.

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
The guest uses a forced reboot and QEMU no-reboot exit before temporary
artifacts are deleted. The first CI guest run timed out after 75 seconds.
The Alpine kernel SHA-256 is now pinned to the observed download:
`456e0691f095285a82533cc6335280e4b4991d630c5292ddf15e6320e71971c9`.
After a panic classified as `init_exited`, changing the TCG CPU from
default `qemu64` to `max` allowed hosted Ubuntu's static BusyBox to run.
The guest reached userspace and enumerated both virtio PCI devices,
then exited successfully, as confirmed by [run 40219493430](https://github.com/sguzman/starbyte/actions/runs/40219493430).

The `--driver-smoke` mode downloads Alpine's matching
`modloop-virt` kernel module SquashFS image into the **hosted** runner's
private workspace, verifies its SHA-256
(`237f5e31e95071816565ffcc55bc028cbdeed960e8e54efd134c8cf13b101782`),
unpacks the kernel-version module tree (excluding firmware), decompresses
modules and rebuilds dependency metadata before building the disposable
guest initramfs. Guest userspace loads `virtio_gpu`, `virtio_input`
and `evdev` with zero module-load errors, then confirms a DRM render
character device and an evdev event device. **This passed** in
[hosted run 40220037441](https://github.com/sguzman/starbyte/actions/runs/40220037441)
at `8e868a8`; the serial-only receipt records
`STARBYTE_GUEST_DRM_INPUT_OK drm_render_node=present evdev_input_node=present guest_only=true`.
It never attaches physical input or grants access to the host GPU.
Subsequent hosted runs separately proved GBM buffer allocation, real Weston
Wayland protocol globals and focused sample-window keyboard delivery.
Guest-created device nodes alone were never counted as those proofs.
EGL-based Starbyte rendering and physical/production cutover remain separate.

This is a **guest-kernel PCI enumeration** test, not proof of a loaded
DRM driver, render node, evdev input seat, guest compositor, or physical
Starbyte acceptance. Those require separate positive checks.

### Guest GBM allocator and rendering-buffer probe (2026-10-25)

CI 40223076070 confirmed both guest render and card nodes could be
opened, but neither could initialize `gbm_create_device`, despite
staging `kms_swrast_dri.so`. Newer Mesa's `libgbm` dynamically loads
`dri_gbm.so` from a separate `gbm/` library directory. That backend
was absent from the disposable initramfs; the next CI attempt carries
the hosted distribution's `dri_gbm.so` and its exact transitive
dependencies and explicitly sets `GBM_BACKENDS_PATH` in the **guest**.
This is a concrete missing runtime component, not new host GPU access.
**Confirmed success:** [CI 40223398366](https://github.com/sguzman/starbyte/actions/runs/40223398366)
on commit `16c6a7b` reported both DRM nodes open, both GBM devices
initialized, render-node buffer creation unsupported, and primary
card **rendering-buffer creation successful**. The Rust GBM probe
created and destroyed the XRGB8888 64×64 guest buffer. It did
not create an EGL context or Wayland compositor, inject keyboard
events, or use the owner's GPU/input devices. Host-device isolation
and all six native/cutover acceptance gates remain enforced.


A dedicated `--gbm-smoke` mode builds a small Rust GBM FFI executable
on the disposable GitHub-hosted runner, copies the executable and its
resolved host-supplied GBM shared libraries into guest RAM, then uses
the already-verified guest virtio GPU/input drivers. The guest code
opens its **own** `/dev/dri/renderD*`, calls `gbm_create_device`
and attempts a 64×64 XRGB8888 `GBM_BO_USE_RENDERING` buffer.
Device/buffer creation and teardown are checked by explicit serial
receipts; library/bootstrap and GPU allocation failures are not
silently accepted. The guest has no host display/DRM/input passthrough,
network or storage, and the probe cannot operate on the owner's
compositor. No EGL surface or Hyprland window is tested by this probe.

**Status:** The first hosted run ([CI 40221293158](https://github.com/sguzman/starbyte/actions/runs/40221293158))
reached the GBM stage but failed with `device=false buffer=false`;
the prior guest DRM/input proof still passed. The next targeted
diagnostic confirmed `started=true node_open=true exec_ok=true`:
the Rust process, shared-library loader and guest DRM open worked;
`gbm_create_device` returned null. The Mesa `kms_swrast_dri.so` software backend and dependencies were
then staged successfully, but `gbm_create_device` still returned null
for the virtual render node (run 40222606494). The following bounded
probe tests both the guest render node and guest DRM primary card, with
separately attributable machine-readable receipts. A primary-only
success would demonstrate the virtual KMS path, **not** render-node
GBM support. The allocator gate remains open until at least one
rendering buffer passes;
even that would not prove compositor or physical host behavior.

### Virtual key injection and guest evdev event proof (2026-10-25)

`--input-smoke` starts a statically linked Rust reader on guest
`/dev/input/event*` and emits a readiness marker. The hosted runner
then sends a QMP `input-send-event` A-key press through the VM's
private Unix socket. A QMP command acknowledgment is necessary but
insufficient: the guest must independently observe Linux `EV_KEY`,
key code 30 (`KEY_A`), and key-down value 1. The guest exits cleanly,
and the host validates both bounded receipts while keeping its serial
transcript private.

**Verified:** [hosted CI 40224443269](https://github.com/sguzman/starbyte/actions/runs/40224443269)
on `149a003` accepted both distinct receipts:
`STARBYTE_GUEST_INPUT_QMP_ACK key_a=sent` and
`STARBYTE_GUEST_INPUT_OK key_a=received source=private_qmp evdev=guest_only`.
There was no kernel panic, and the previous GBM buffer test remained
green. No host input hardware, desktop display, Wayland socket or
external session IPC was used. Evdev delivery cannot substitute for a
separately proven focused compositor window or native physical testing.

### Guest Weston compositor and focused-window keyboard proof (2026-10-25)

The hosted QEMU TCG guest now has a virtual DRM/KMS device, a private virtual
keyboard and an independent Weston DRM compositor. To make the real input
seat available, its disposable initramfs starts udev, loads packaged udev
rules and copies libinput quirks and the canonical XKB layouts into guest
RAM; it uses a guest-only writable `/tmp` and `/run`. The protocol probe
uses native Wayland `wl_display.get_registry` and `sync` messages and
accepts only actual `wl_compositor` and `wl_seat` registry announcements.

**Verified hosted acceptance**:
[CI run 40234758710](https://github.com/sguzman/starbyte/actions/runs/40234758710)
on `e0224d7` passed the whole isolated workflow, including the compositor
registry barrier, guest GBM rendering-buffer creation and QMP-to-evdev A-key
delivery. [CI run 40235518684](https://github.com/sguzman/starbyte/actions/runs/40235518684)
on `0ae50e7` additionally accepted both
`STARBYTE_GUEST_FOCUS_QMP_ACK escape=sent source=private_qmp` and
`STARBYTE_GUEST_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 weston_alive=1`.
An official `weston-simple-shm` xdg-toplevel received Escape through the
guest's keyboard seat and exited cleanly while Weston remained alive;
the VM's former GBM and evdev checks also passed again.

**Evidence boundaries:** These receipts prove a *virtual* DRM compositor,
Wayland protocol registry and focused Wayland sample input on a disposable
QEMU keyboard, not physical owner-session Hyprland focus, gamepads, audible
speaker quality, Starbyte video frames or a production worker cutover.
The original native/physical QA hard-stops remain untouched. The
original worker-backed Starbyte app has now been accepted in this same
disposable QEMU guest: [hosted run 40240050599](https://github.com/sguzman/starbyte/actions/runs/40240050599)
on `4302db8` marked the **actual Starbyte window and worker** test successful.
The `--starbyte-smoke` acceptance requires the original full-library egui
window, 30 worker-produced frames, and the
`STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true`
receipt. The VM stages host-runner CPU-only Mesa software EGL plus Wayland
EGL/cursor libraries loaded at runtime, and never forwards a host GPU,
keyboard, display or user's files. This is original-app **virtual** acceptance,
not a physical Hyprland-focused FPS, speaker, gamepad or cutover pass.
The same [hosted run 40240050599](https://github.com/sguzman/starbyte/actions/runs/40240050599)
finished **successfully in full**, including the repeated virtual Weston registry,
focused Escape, guest DRM, GBM rendering-buffer and private QMP-to-evdev input
regressions. The guest reported
`STARBYTE_GUEST_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true`.
Standard Linux/Windows, Clippy and Rust 1.88 CI also passed on
[run 40240050619](https://github.com/sguzman/starbyte/actions/runs/40240050619).
The pending six physical/cutover gates remain open at 71/77 (92%).

### Virtual rendering/input strategy and remaining limits

**Superseded:** the earlier two-stage *software-only nested Weston →
Hyprland* proposal. Aquamarine v0.15.1 cannot use a parent Wayland socket
as a substitute for its required DRM-backed GBM allocator. This is proven
from upstream source above; do not retry the old path or bind-mount host
`/dev/dri` or `/dev/input` into its sandbox to make it start.

The earlier proposal below is retained as a requirements record. A
disposable QEMU/virtio GPU and keyboard guest is **now implemented and
verified with Weston**, including the original Starbyte worker-backed GUI
as described above. A guest Hyprland runtime, physical audio/gamepad
acceptance and the production-default cutover remain unverified:

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
The virtual GPU, keyboard seat, DRM-backed Weston, focused sample
keyboard, and original worker-backed Starbyte GUI are verified by hosted
CI (individual test-step evidence above). This infrastructure never
substitutes for physical Hyprland, speaker or gamepad acceptance.
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
