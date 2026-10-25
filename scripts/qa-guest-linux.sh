#!/usr/bin/env bash
# Disposable Linux guest PCI smoke; hosted CI only. Never uses host desktop.
set -Eeuo pipefail
umask 077
fail() { printf 'STARBYTE_GUEST_LINUX_BLOCKED %s\n' "$1" >&2; exit 3; }
mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --smoke || "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke ) ]] || fail 'usage: --check-only|--smoke|--driver-smoke|--gbm-smoke|--input-smoke|--wayland-smoke'
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted ]] || fail 'github_hosted_runner_only'
[[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" && -z "${WAYLAND_SOCKET:-}" ]] || fail 'graphical_session_present'
[[ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" && -z "${DBUS_SESSION_BUS_ADDRESS:-}" && -z "${XDG_SESSION_ID:-}" ]] || fail 'interactive_session_present'
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail 'linux_x86_64_required'
if [[ "$mode" == --check-only ]]; then
  echo 'STARBYTE_GUEST_PREFLIGHT_OK environment=github_hosted display=none'
  exit 0
fi
for tool in qemu-system-x86_64 busybox cpio gzip curl sha256sum timeout; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing_$tool"
done
root="$(mktemp -d "${TMPDIR:-/tmp}/starbyte-guest-linux.XXXXXXXX")" || fail 'private_runtime_failed'
chmod 700 "$root"
trap 'rm -rf -- "$root"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# Official Alpine v3.23 virt kernel, pinned to the first hosted
# download's SHA-256 to reject future silent release updates.
kernel_url='https://dl-cdn.alpinelinux.org/alpine/v3.23/releases/x86_64/netboot/vmlinuz-virt'
curl --fail --location --silent --show-error --retry 2 --max-time 120 \
  --proto '=https' --tlsv1.2 "$kernel_url" --output "$root/vmlinuz-virt" \
  || fail 'alpine_kernel_download_failed'
sha="$(sha256sum "$root/vmlinuz-virt")"
sha="${sha%% *}"
[[ "$sha" == 456e0691f095285a82533cc6335280e4b4991d630c5292ddf15e6320e71971c9 ]] || fail 'alpine_kernel_digest_mismatch'
printf 'STARBYTE_GUEST_KERNEL_SHA256 %s\n' "$sha"

mkdir -p "$root/initfs/bin" "$root/initfs/dev" "$root/initfs/proc" "$root/initfs/sys"
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke ]]; then
  # The matching official Alpine modloop is a SquashFS module tree, not
  # a host module mount. Extract it only in this disposable runner.
  for tool in unsquashfs depmod; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing_$tool"
  done
  modloop_url='https://dl-cdn.alpinelinux.org/alpine/v3.23/releases/x86_64/netboot/modloop-virt'
  curl --fail --location --silent --show-error --retry 2 --max-time 120 \
    --proto '=https' --tlsv1.2 "$modloop_url" --output "$root/modloop-virt" \
    || fail 'modloop_download_failed'
  modloop_sha="$(sha256sum "$root/modloop-virt")"
  modloop_sha="${modloop_sha%% *}"
  [[ "$modloop_sha" == 237f5e31e95071816565ffcc55bc028cbdeed960e8e54efd134c8cf13b101782 ]] \
    || fail 'modloop_digest_mismatch'
  printf 'STARBYTE_GUEST_MODLOOP_SHA256 %s\n' "$modloop_sha"
  unsquashfs -d "$root/modloop" "$root/modloop-virt" >/dev/null 2>&1 \
    || fail 'modloop_extract_failed'
  module_base="$root/modloop/modules"
  [[ -d "$module_base" ]] || fail 'modloop_module_tree_missing'
  shopt -s nullglob
  # Alpine modloop also has a firmware directory. Select only actual
  # kernel-release module directories, not every sibling directory.
  releases=("$module_base"/[0-9]*)
  shopt -u nullglob
  [[ "${#releases[@]}" -eq 1 && -d "${releases[0]}" ]] || fail 'modloop_kernel_release_ambiguous'
  release="${releases[0]##*/}"
  mkdir -p "$root/initfs/lib/modules" "$root/initfs/etc"
  cp -a -- "${releases[0]}" "$root/initfs/lib/modules/$release"
  while IFS= read -r -d '' module; do
    gzip -d -- "$module" || fail 'module_decompression_failed'
  done < <(find "$root/initfs/lib/modules/$release" -type f -name '*.ko.gz' -print0)
  depmod -b "$root/initfs" "$release" || fail 'modloop_depmod_failed'
  : >"$root/initfs/etc/starbyte-driver-smoke"
fi
if [[ "$mode" == --gbm-smoke || "$mode" == --wayland-smoke ]]; then
  # Compile Rust only on the hosted CI runner, then execute inside guest RAM.
  # Copy just the compiled binary's dynamic linker and shared libraries.
  for tool in rustc ldd awk sort; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing_$tool"
  done
  rustc --edition=2021 -C opt-level=1 scripts/qa-guest-gbm.rs \
    -o "$root/initfs/bin/gbm-probe" || fail 'gbm_probe_compilation_failed'
  ldd "$root/initfs/bin/gbm-probe" >"$root/gbm-dependencies" \
    || fail 'gbm_dependency_scan_failed'
  # The minimal guest has libgbm but initially NO Mesa DRI backend.
  # Carry only the hosted distribution's CPU-only kms_swrast renderer,
  # together with its exact dynamic dependencies, into disposable guest RAM.
  dri_driver="$(find /usr/lib -name 'kms_swrast_dri.so' -print -quit)"
  [[ -n "$dri_driver" && -f "$dri_driver" ]] || fail 'hosted_kms_swrast_backend_missing'
  mkdir -p "$root/initfs$(dirname "$dri_driver")"
  cp -L -- "$dri_driver" "$root/initfs$dri_driver" \
    || fail 'gbm_software_backend_copy_failed'
  ldd "$dri_driver" >>"$root/gbm-dependencies" \
    || fail 'gbm_software_backend_dependency_scan_failed'
  # Mesa >=25 loads its GBM implementation from a *separate* plugin.
  # Copying libgbm.so and kms_swrast_dri.so alone leaves
  # gbm_create_device() returning NULL even for a valid DRM node.
  gbm_backend="$(find /usr/lib -path '*/gbm/dri_gbm.so' -print -quit)"
  [[ -n "$gbm_backend" && -f "$gbm_backend" ]] \
    || fail 'hosted_mesa_gbm_backend_missing'
  mkdir -p "$root/initfs$(dirname "$gbm_backend")"
  cp -L -- "$gbm_backend" "$root/initfs$gbm_backend" \
    || fail 'gbm_backend_copy_failed'
  ldd "$gbm_backend" >>"$root/gbm-dependencies" \
    || fail 'gbm_backend_dependency_scan_failed'
  if grep -Fq 'not found' "$root/gbm-dependencies"; then
    fail 'gbm_dependency_missing'
  fi
  while IFS= read -r dependency; do
    [[ "$dependency" == /* ]] || fail 'gbm_dependency_untrusted_path'
    mkdir -p "$root/initfs$(dirname "$dependency")"
    cp -L -- "$dependency" "$root/initfs$dependency" \
      || fail 'gbm_dependency_copy_failed'
  done < <(awk '$3 ~ /^\// {print $3} $1 ~ /^\// {print $1}' "$root/gbm-dependencies" | sort -u)
  if [[ "$mode" == --gbm-smoke ]]; then : >"$root/initfs/etc/starbyte-gbm-smoke"; fi
fi
if [[ "$mode" == --input-smoke ]]; then
  for tool in rustc socat jq; do
    command -v "$tool" >/dev/null 2>&1 || fail "guest_input_missing_$tool"
  done
  rustc --edition=2021 -C opt-level=1 -C target-feature=+crt-static \
    scripts/qa-guest-input.rs -o "$root/initfs/bin/input-probe" \
    || fail 'guest_input_compilation_failed'
  : >"$root/initfs/etc/starbyte-input-smoke"
fi
if [[ "$mode" == --wayland-smoke ]]; then
  # Stage a real DRM/KMS Weston compositor, its dynamically loaded modules,
  # and an independent Rust Wayland wire client into private guest RAM.
  # This does not launch a host GUI.
  for tool in weston wayland-info ldd awk sort; do
    command -v "$tool" >/dev/null 2>&1 || fail "wayland_host_tool_missing_$tool"
  done
  rustc --edition=2021 -C opt-level=1 -C target-feature=+crt-static \
    scripts/qa-guest-wayland-wire.rs -o "$root/initfs/bin/wayland-wire" \
    || fail 'guest_wayland_wire_compilation_failed'
  stage_files=("$(command -v weston)" "$(command -v wayland-info)")
  for pattern in '*/libweston-*/drm-backend.so' '*/weston/kiosk-shell.so'; do
    match="$(find /usr/lib -type f -path "$pattern" -print -quit)"
    [[ -n "$match" && -f "$match" ]] || fail "wayland_module_missing_${pattern##*/}"
    stage_files+=("$match")
  done
  # Weston releases may load this renderer as a separate plugin.
  pixman_plugin="$(find /usr/lib -type f -name 'pixman-renderer.so' -print -quit)"
  if [[ -n "$pixman_plugin" ]]; then stage_files+=("$pixman_plugin"); fi
  : >"$root/wayland-dependencies"
  for artifact in "${stage_files[@]}"; do
    mkdir -p "$root/initfs$(dirname "$artifact")"
    cp -L -- "$artifact" "$root/initfs$artifact" || fail 'wayland_stage_failed'
    ldd "$artifact" >>"$root/wayland-dependencies" || fail 'wayland_dependency_scan_failed'
  done
  grep -Fq 'not found' "$root/wayland-dependencies" && fail 'wayland_dependency_missing'
  while IFS= read -r dependency; do
    [[ "$dependency" == /* ]] || fail 'wayland_dependency_untrusted_path'
    mkdir -p "$root/initfs$(dirname "$dependency")"
    cp -L -- "$dependency" "$root/initfs$dependency" || fail 'wayland_dependency_copy_failed'
  done < <(awk '$3 ~ /^\// {print $3} $1 ~ /^\// {print $1}' "$root/wayland-dependencies" | sort -u)
  mkdir -p "$root/initfs/run" "$root/initfs/tmp" "$root/initfs/usr/bin"
  : >"$root/initfs/etc/starbyte-wayland-smoke"
fi
cp "$(command -v busybox)" "$root/initfs/bin/busybox"
ln -s busybox "$root/initfs/bin/sh"
cat >"$root/initfs/init" <<'GUEST_INIT'
#!/bin/sh
PATH=/bin
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
echo 'STARBYTE_GUEST_LINUX_BOOT_OK initramfs=private console=serial' >/dev/console
gpu=0
keyboard=0
for device in /sys/bus/pci/devices/*; do
  [ -f "$device/vendor" ] && [ -f "$device/device" ] || continue
  vendor="$(/bin/busybox cat "$device/vendor")"
  model="$(/bin/busybox cat "$device/device")"
  if [ "$vendor" = 0x1af4 ] && [ "$model" = 0x1050 ]; then gpu=1; fi
  if [ "$vendor" = 0x1af4 ] && [ "$model" = 0x1052 ]; then keyboard=1; fi
done
echo "STARBYTE_GUEST_PCI_RESULT virtio_gpu=$gpu virtio_keyboard=$keyboard" >/dev/console
if [ -e /etc/starbyte-driver-smoke ]; then
  /bin/busybox modprobe virtio_gpu >/dev/null 2>&1
  gpu_load=$?
  /bin/busybox modprobe virtio_input >/dev/null 2>&1
  input_load=$?
  /bin/busybox modprobe evdev >/dev/null 2>&1
  evdev_load=$?
  /bin/busybox sleep 1
  render=0
  input_event=0
  for node in /dev/dri/renderD*; do
    [ -c "$node" ] && render=1
  done
  for node in /dev/input/event*; do
    [ -c "$node" ] && input_event=1
  done
  echo "STARBYTE_GUEST_DRIVER_RESULT render=$render input=$input_event gpu_load=$gpu_load input_load=$input_load evdev_load=$evdev_load" >/dev/console
  if [ -e /etc/starbyte-gbm-smoke ]; then
    guest_render_node=''
    for node in /dev/dri/renderD*; do
      if [ -c "$node" ]; then guest_render_node="$node"; break; fi
    done
    MESA_LOADER_DRIVER_OVERRIDE=kms_swrast LIBGL_ALWAYS_SOFTWARE=1 \
      GBM_BACKEND=dri GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm \
      LIBGL_DRIVERS_PATH=/usr/lib/x86_64-linux-gnu/dri \
      /bin/gbm-probe "$guest_render_node" >/dev/console 2>/dev/null
    echo "STARBYTE_GUEST_GBM_EXEC render_status=$?" >/dev/console
    guest_card_node=''
    for node in /dev/dri/card*; do
      if [ -c "$node" ]; then guest_card_node="$node"; break; fi
    done
    MESA_LOADER_DRIVER_OVERRIDE=kms_swrast LIBGL_ALWAYS_SOFTWARE=1 \
      GBM_BACKEND=dri GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm \
      LIBGL_DRIVERS_PATH=/usr/lib/x86_64-linux-gnu/dri \
      /bin/gbm-probe "$guest_card_node" >/dev/console 2>/dev/null
    echo "STARBYTE_GUEST_GBM_EXEC card_status=$?" >/dev/console
  fi
  if [ -e /etc/starbyte-wayland-smoke ]; then
    /bin/busybox mkdir -p /run/user/0 /tmp
    /bin/busybox chmod 700 /run/user/0
    export XDG_RUNTIME_DIR=/run/user/0
    export LIBSEAT_BACKEND=builtin
    export XDG_SEAT=seat0
    export HOME=/tmp
    export PATH=/usr/bin:/bin
    export MESA_LOADER_DRIVER_OVERRIDE=kms_swrast
    export LIBGL_ALWAYS_SOFTWARE=1
    export GBM_BACKEND=dri
    export GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm
    export LIBGL_DRIVERS_PATH=/usr/lib/x86_64-linux-gnu/dri
    /usr/bin/weston --backend=drm --renderer=pixman --shell=kiosk-shell.so --no-config \
      --drm-device=card0 --continue-without-input \
      --socket=starbyte-wayland-0 --log=/tmp/weston.log \
      >/tmp/weston.stdout 2>&1 &
    weston_pid=$!
    socket_ready=0
    for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      if [ -S /run/user/0/starbyte-wayland-0 ]; then socket_ready=1; break; fi
      if ! /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then break; fi
      /bin/busybox sleep 1
    done
    handshake=0
    compositor=0
    seat=0
    wire_done=0
    wire_connected=0
    wire_attempts=0
    wayland_attempts=0
    if [ "$socket_ready" = 1 ]; then
      # Socket creation can precede global registration. Require a live
      # registry on a bounded retry loop; never mistake an empty response
      # for a functioning graphical session.
      for retry in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        wayland_attempts=$retry
        WAYLAND_DEBUG=client WAYLAND_DISPLAY=starbyte-wayland-0 \
          /bin/busybox timeout 4 /usr/bin/wayland-info \
          >/tmp/wayland-info.log 2>/tmp/wayland-info.err
        if [ "$?" = 0 ]; then handshake=1; fi
        if /bin/busybox grep -q 'wl_compositor' /tmp/wayland-info.log ||
          /bin/busybox grep -E 'wl_registry.*global.*wl_compositor' /tmp/wayland-info.err >/dev/null; then compositor=1; fi
        if /bin/busybox grep -q 'wl_seat' /tmp/wayland-info.log ||
          /bin/busybox grep -E 'wl_registry.*global.*wl_seat' /tmp/wayland-info.err >/dev/null; then seat=1; fi
        if [ "$handshake" = 1 ] && [ "$compositor" = 1 ] && [ "$seat" = 1 ]; then break; fi
        if ! /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then break; fi
        /bin/busybox sleep 1
      done
    fi
    # The Rust wire client uses wl_display.get_registry followed by sync.
    # Its wl_callback.done is the registry barrier; a clean wayland-info exit
    # with no output is not proof of any actual protocol messages.
    if [ "$socket_ready" = 1 ]; then
      for retry in 1 2 3 4 5 6 7 8; do
        wire_attempts=$retry
        WAYLAND_DISPLAY=starbyte-wayland-0 \
          /bin/busybox timeout 7 /bin/wayland-wire \
          >/tmp/wayland-wire.log 2>/dev/null
        if /bin/busybox grep -Fqx \
          'STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor=1 seat=1' \
          /tmp/wayland-wire.log; then
          wire_connected=1
          wire_done=1
          compositor=1
          seat=1
          handshake=1
          break
        fi
        if /bin/busybox grep -q '^STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1' \
          /tmp/wayland-wire.log; then
          wire_connected=1
          wire_done=1
        fi
        if ! /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then break; fi
        /bin/busybox sleep 1
      done
    fi
    # Diagnose only fixed categories; raw compositor and client logs never
    # leave the disposable VM or appear in public CI.
    module_error=0
    shared_library_error=0
    option_error=0
    seat_error=0
    drm_error=0
    compositor_alive=0
    info_stdout=0
    info_stderr=0
    registry_trace=0
    wire_log=0
    weston_log=0
    weston_stdout=0
    weston_zombie=0
    if /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then compositor_alive=1; fi
    if [ -s /tmp/weston.log ]; then weston_log=1; fi
    if [ -s /tmp/weston.stdout ]; then weston_stdout=1; fi
    if [ -r "/proc/$weston_pid/stat" ]; then
      state="$(/bin/busybox awk '{print $3}' "/proc/$weston_pid/stat")"
      if [ "$state" = Z ]; then weston_zombie=1; fi
    fi
    if [ -s /tmp/wayland-info.log ]; then info_stdout=1; fi
    if [ -s /tmp/wayland-info.err ]; then info_stderr=1; fi
    if [ -s /tmp/wayland-wire.log ]; then wire_log=1; fi
    if /bin/busybox grep -q 'wl_registry' /tmp/wayland-info.err 2>/dev/null; then registry_trace=1; fi
    for log in /tmp/weston.log /tmp/weston.stdout /tmp/wayland-info.err; do
      [ -f "$log" ] || continue
      if /bin/busybox grep -Eiq 'failed to load module|unable to load module' "$log"; then module_error=1; fi
      if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object file' "$log"; then shared_library_error=1; fi
      if /bin/busybox grep -Eiq 'unrecognized option|unknown option|invalid option|unknown renderer|unknown shell' "$log"; then option_error=1; fi
      if /bin/busybox grep -Eiq 'failed to open seat|seatd|libseat|session control|cannot get tty|no seat' "$log"; then seat_error=1; fi
      if /bin/busybox grep -Eiq 'drm|kms|no outputs|no connectors|no usable devices' "$log"; then drm_error=1; fi
    done
    echo "STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=$wire_connected sync=$wire_done output=$wire_log attempts=$wire_attempts" >/dev/console
    echo "STARBYTE_GUEST_WAYLAND_RESULT socket=$socket_ready handshake=$handshake compositor=$compositor seat=$seat" >/dev/console
    echo "STARBYTE_GUEST_WAYLAND_DIAG alive=$compositor_alive zombie=$weston_zombie weston_log=$weston_log weston_stdout=$weston_stdout module=$module_error library=$shared_library_error option=$option_error seat=$seat_error drm=$drm_error stdout=$info_stdout stderr=$info_stderr registry=$registry_trace attempts=$wayland_attempts" >/dev/console
    /bin/busybox kill "$weston_pid" 2>/dev/null || :
    wait "$weston_pid" 2>/dev/null || :
  fi
  if [ -e /etc/starbyte-input-smoke ]; then
    /bin/input-probe >/dev/console 2>/dev/null
    echo "STARBYTE_GUEST_INPUT_EXEC status=$?" >/dev/console
  fi
fi
/bin/busybox reboot -f
while :; do /bin/busybox sleep 1; done
GUEST_INIT
chmod 755 "$root/initfs/init"
(cd "$root/initfs"; find . -print0 | cpio --null -o -H newc --quiet | gzip -9) >"$root/initramfs.gz"
[[ -s "$root/initramfs.gz" ]] || fail 'initramfs_creation_failed'

# Guest devices are entirely emulated. Never expose host DRM/input/files.
guest_ram=384M
if [[ "$mode" == --gbm-smoke || "$mode" == --wayland-smoke ]]; then guest_ram=768M; fi
# -cpu max remains TCG-only; QEMU's default qemu64 CPU may lack instructions
# needed by Ubuntu's x86-64-v3 static BusyBox, killing PID 1 before output.
# The only synthetic keyboard injection path is this guest's private QMP
# socket. Host input devices, displays and session IPC remain inaccessible.
qmp_args=()
injector_pid=''
if [[ "$mode" == --input-smoke ]]; then
  qmp_args=(-qmp "unix:$root/qmp.sock,server=on,wait=off")
  guest_input_injector() {
    local ready=false
    for _ in {1..350}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]] \
        && grep -Fq 'STARBYTE_GUEST_INPUT_READY evdev=present' "$root/serial.log"; then
        ready=true
        break
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_INPUT_QMP_BLOCKED guest_not_ready'; return 5; }
    local qmp_status=0
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]},"id":"key"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] \
      || { echo 'STARBYTE_GUEST_INPUT_QMP_BLOCKED transport'; return 5; }
    jq -es '
      any(.[]; .id == "cap" and .return == {}) and
      any(.[]; .id == "key" and .return == {})
    ' "$root/qmp.jsonl" >/dev/null \
      || { echo 'STARBYTE_GUEST_INPUT_QMP_BLOCKED event_rejected'; return 5; }
    echo 'STARBYTE_GUEST_INPUT_QMP_ACK key_a=sent'
  }
  guest_input_injector &
  injector_pid=$!
  trap 'kill "$injector_pid" 2>/dev/null || true; wait "$injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
display_args=(-display none)
if [[ "$mode" == --wayland-smoke ]]; then
  # Private Unix-only VNC display emulation advertises a KMS output.
  # No TCP listener, physical display, viewer, or host desktop socket.
  display_args=(-display "vnc=unix:$root/guest-vnc.sock")
fi
guest_timeout=125
if [[ "$mode" == --wayland-smoke ]]; then guest_timeout=200; fi
set +e
HOME="$root" XDG_CONFIG_HOME="$root" \
  timeout --signal=TERM --kill-after=2s "${guest_timeout}s" qemu-system-x86_64 \
    -machine q35,accel=tcg -cpu max -m "$guest_ram" -smp 1 -no-reboot \
    -nodefaults "${display_args[@]}" -vga none -net none \
    -monitor none -parallel none -serial "file:$root/serial.log" \
    -device virtio-gpu-pci,id=sb_gpu -device virtio-keyboard-pci,id=sb_keyboard \
    "${qmp_args[@]}" \
    -kernel "$root/vmlinuz-virt" -initrd "$root/initramfs.gz" \
    -append 'console=ttyS0,115200 loglevel=3 panic=5' \
    >"$root/qemu.log" 2>&1
result=$?
set -e
if [[ "$mode" == --input-smoke ]]; then
  wait "$injector_pid" || fail 'guest_private_qmp_injection_failed'
  injector_pid=''
fi
# Never emit raw guest logs. Expose only three bounded diagnostic bits,
# including on QEMU timeout, to distinguish boot from shutdown failure.
[[ -f "$root/serial.log" ]] || : >"$root/serial.log"
tr -d '\r' <"$root/serial.log" >"$root/serial-clean.log"
boot=false
pci=false
panic=false
if grep -Fqx 'STARBYTE_GUEST_LINUX_BOOT_OK initramfs=private console=serial' "$root/serial-clean.log"; then boot=true; fi
if grep -Fqx 'STARBYTE_GUEST_PCI_RESULT virtio_gpu=1 virtio_keyboard=1' "$root/serial-clean.log"; then pci=true; fi
if grep -Fq 'Kernel panic' "$root/serial-clean.log"; then panic=true; fi
cause=none
if [[ "$panic" == true ]]; then
  cause=unclassified
  if grep -Eq 'No working init found|Requested init /init failed|Failed to execute /init' "$root/serial-clean.log"; then
    cause=init_exec
  elif grep -Eq 'VFS: Unable to mount root fs|unable to mount root fs' "$root/serial-clean.log"; then
    cause=root_filesystem
  elif grep -Fq 'Attempted to kill init' "$root/serial-clean.log"; then
    cause=init_exited
  elif grep -Eq 'Fatal exception|Fatal signal' "$root/serial-clean.log"; then
    cause=kernel_exception
  fi
fi
printf 'STARBYTE_GUEST_LINUX_DIAG boot=%s pci=%s panic=%s cause=%s qemu_status=%s\n' "$boot" "$pci" "$panic" "$cause" "$result"
[[ "$result" -eq 0 ]] || fail "guest_boot_failed_or_timed_out status=$result"
[[ "$boot" == true ]] || fail 'boot_receipt_missing'
[[ "$pci" == true ]] || fail 'guest_virtio_pci_devices_missing'
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  if [[ "$mode" == --wayland-smoke ]]; then
    # These two lines are machine-authored, bounded 0/1 receipts.
    grep -E '^STARBYTE_GUEST_WAYLAND_(RESULT|DIAG|WIRE_DIAG) ' "$root/serial-clean.log" || true
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_RESULT socket=1 handshake=1 compositor=1 seat=1' "$root/serial-clean.log" \
      || fail 'guest_drm_wayland_handshake_or_seat_missing'
    grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=1 sync=1 output=1 attempts=[1-8]
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=wayland-info seat=present guest_only=true'
  elif [[ "$mode" == --gbm-smoke ]]; then
    render_node=false
    render_device=false
    render_buffer=false
    card_node=false
    card_device=false
    card_buffer=false
    grep -Fqx 'STARBYTE_GUEST_GBM_NODE_OPEN_OK node=render' "$root/serial-clean.log" && render_node=true
    grep -Fqx 'STARBYTE_GUEST_GBM_NODE_OPEN_OK node=card' "$root/serial-clean.log" && card_node=true
    grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=render device=1 buffer=0' "$root/serial-clean.log" && render_device=true
    grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=card device=1 buffer=0' "$root/serial-clean.log" && card_device=true
    if grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=render device=1 buffer=1' "$root/serial-clean.log"; then
      render_device=true
      render_buffer=true
    fi
    if grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=card device=1 buffer=1' "$root/serial-clean.log"; then
      card_device=true
      card_buffer=true
    fi
    printf 'STARBYTE_GUEST_GBM_DIAG render_open=%s render_device=%s render_buffer=%s card_open=%s card_device=%s card_buffer=%s\n' \
      "$render_node" "$render_device" "$render_buffer" "$card_node" "$card_device" "$card_buffer"
    grep -Fqx 'STARBYTE_GUEST_GBM_EXEC render_status=0' "$root/serial-clean.log" \
      || fail 'guest_gbm_render_probe_failed'
    grep -Fqx 'STARBYTE_GUEST_GBM_EXEC card_status=0' "$root/serial-clean.log" \
      || fail 'guest_gbm_card_probe_failed'
    [[ "$render_node" == true && "$card_node" == true ]] \
      || fail 'guest_gbm_drm_node_open_failed'
    if [[ "$render_buffer" == true ]]; then
      echo 'STARBYTE_GUEST_GBM_OK node=render device=initialized buffer=rendering guest_only=true'
    elif [[ "$card_buffer" == true ]]; then
      echo 'STARBYTE_GUEST_GBM_OK node=card device=initialized buffer=rendering guest_only=true'
    else
      fail 'guest_gbm_allocator_unavailable'
    fi
  elif [[ "$mode" == --input-smoke ]]; then
    grep -Fqx 'STARBYTE_GUEST_INPUT_READY evdev=present' "$root/serial-clean.log" \
      || fail 'guest_input_listener_missing'
    grep -Fqx 'STARBYTE_GUEST_INPUT_RESULT key_a=1' "$root/serial-clean.log" \
      || fail 'guest_virtual_keypress_missing'
    grep -Fqx 'STARBYTE_GUEST_INPUT_EXEC status=0' "$root/serial-clean.log" \
      || fail 'guest_virtual_keypress_reader_failed'
    echo 'STARBYTE_GUEST_INPUT_OK key_a=received source=private_qmp evdev=guest_only'
  else
    echo 'STARBYTE_GUEST_DRM_INPUT_OK drm_render_node=present evdev_input_node=present guest_only=true'
  fi
else
  echo 'STARBYTE_GUEST_LINUX_PCI_OK gpu=virtio keyboard=virtio serial_only=true drm_render_node=not_tested input_seat=not_tested'
fi
 "$root/serial-clean.log" \
      || fail 'guest_wayland_registry_sync_missing'
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=wayland-info seat=present guest_only=true'
  elif [[ "$mode" == --gbm-smoke ]]; then
    render_node=false
    render_device=false
    render_buffer=false
    card_node=false
    card_device=false
    card_buffer=false
    grep -Fqx 'STARBYTE_GUEST_GBM_NODE_OPEN_OK node=render' "$root/serial-clean.log" && render_node=true
    grep -Fqx 'STARBYTE_GUEST_GBM_NODE_OPEN_OK node=card' "$root/serial-clean.log" && card_node=true
    grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=render device=1 buffer=0' "$root/serial-clean.log" && render_device=true
    grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=card device=1 buffer=0' "$root/serial-clean.log" && card_device=true
    if grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=render device=1 buffer=1' "$root/serial-clean.log"; then
      render_device=true
      render_buffer=true
    fi
    if grep -Fqx 'STARBYTE_GUEST_GBM_RESULT node=card device=1 buffer=1' "$root/serial-clean.log"; then
      card_device=true
      card_buffer=true
    fi
    printf 'STARBYTE_GUEST_GBM_DIAG render_open=%s render_device=%s render_buffer=%s card_open=%s card_device=%s card_buffer=%s\n' \
      "$render_node" "$render_device" "$render_buffer" "$card_node" "$card_device" "$card_buffer"
    grep -Fqx 'STARBYTE_GUEST_GBM_EXEC render_status=0' "$root/serial-clean.log" \
      || fail 'guest_gbm_render_probe_failed'
    grep -Fqx 'STARBYTE_GUEST_GBM_EXEC card_status=0' "$root/serial-clean.log" \
      || fail 'guest_gbm_card_probe_failed'
    [[ "$render_node" == true && "$card_node" == true ]] \
      || fail 'guest_gbm_drm_node_open_failed'
    if [[ "$render_buffer" == true ]]; then
      echo 'STARBYTE_GUEST_GBM_OK node=render device=initialized buffer=rendering guest_only=true'
    elif [[ "$card_buffer" == true ]]; then
      echo 'STARBYTE_GUEST_GBM_OK node=card device=initialized buffer=rendering guest_only=true'
    else
      fail 'guest_gbm_allocator_unavailable'
    fi
  elif [[ "$mode" == --input-smoke ]]; then
    grep -Fqx 'STARBYTE_GUEST_INPUT_READY evdev=present' "$root/serial-clean.log" \
      || fail 'guest_input_listener_missing'
    grep -Fqx 'STARBYTE_GUEST_INPUT_RESULT key_a=1' "$root/serial-clean.log" \
      || fail 'guest_virtual_keypress_missing'
    grep -Fqx 'STARBYTE_GUEST_INPUT_EXEC status=0' "$root/serial-clean.log" \
      || fail 'guest_virtual_keypress_reader_failed'
    echo 'STARBYTE_GUEST_INPUT_OK key_a=received source=private_qmp evdev=guest_only'
  else
    echo 'STARBYTE_GUEST_DRM_INPUT_OK drm_render_node=present evdev_input_node=present guest_only=true'
  fi
else
  echo 'STARBYTE_GUEST_LINUX_PCI_OK gpu=virtio keyboard=virtio serial_only=true drm_render_node=not_tested input_seat=not_tested'
fi
