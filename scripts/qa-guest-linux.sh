#!/usr/bin/env bash
# Disposable Linux guest PCI smoke; hosted CI only. Never uses host desktop.
set -Eeuo pipefail
umask 077
fail() { printf 'STARBYTE_GUEST_LINUX_BLOCKED %s\n' "$1" >&2; exit 3; }
mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --smoke || "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ) ]] || fail 'usage: --check-only|--smoke|--driver-smoke|--gbm-smoke|--input-smoke|--wayland-smoke|--starbyte-smoke|--hyprland-smoke'
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
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
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
if [[ "$mode" == --gbm-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
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
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Stage a real DRM/KMS Weston compositor, its dynamically loaded modules,
  # and an independent Rust Wayland wire client into private guest RAM.
  # This does not launch a host GUI.
  for tool in weston wayland-info ldd awk sort readelf; do
    command -v "$tool" >/dev/null 2>&1 || fail "wayland_host_tool_missing_$tool"
  done
  rustc --edition=2021 -C opt-level=1 -C target-feature=+crt-static \
    scripts/qa-guest-wayland-wire.rs -o "$root/initfs/bin/wayland-wire" \
    || fail 'guest_wayland_wire_compilation_failed'
  readelf -h "$root/initfs/bin/wayland-wire" >"$root/wire-elf-header" \
    || fail 'guest_wire_invalid_elf'
  readelf -l "$root/initfs/bin/wayland-wire" >"$root/wire-elf-programs" \
    || fail 'guest_wire_invalid_elf'
  grep -Fq 'Advanced Micro Devices X86-64' "$root/wire-elf-header" \
    || fail 'guest_wire_wrong_architecture'
  elf_interp=0
  if grep -Fq 'INTERP' "$root/wire-elf-programs"; then elf_interp=1; fi
  printf 'STARBYTE_GUEST_WIRE_ELF machine=x86_64 interp=%s\n' "$elf_interp"
  command -v weston-simple-shm >/dev/null 2>&1 || fail 'hosted_wayland_sample_client_missing'
  stage_files=("$(command -v weston)" "$(command -v wayland-info)" "$(command -v weston-simple-shm)")
  for pattern in '*/libweston-*/drm-backend.so' '*/weston/kiosk-shell.so'; do
    match="$(find /usr/lib -type f -path "$pattern" -print -quit)"
    [[ -n "$match" && -f "$match" ]] || fail "wayland_module_missing_${pattern##*/}"
    stage_files+=("$match")
  done
  # Ubuntu 26.04 installs keyboard layouts under xkeyboard-config-2.
  # The older X11/xkb path can be a symlink; copying only that symlink
  # leaves the actual layout files absent inside the guest.
  xkb_source=''
  for candidate in /usr/share/xkeyboard-config-* /usr/share/X11/xkb; do
    [[ -f "$candidate/rules/evdev" && -f "$candidate/symbols/us" ]] || continue
    xkb_source="$(readlink -f -- "$candidate")"
    break
  done
  [[ -n "$xkb_source" && -d "$xkb_source" ]] || fail 'hosted_xkb_dataset_missing'
  mkdir -p "$root/initfs/usr/share"
  cp -a -- "$xkb_source" "$root/initfs/usr/share/starbyte-xkb" \
    || fail 'guest_xkb_dataset_stage_failed'
  [[ -f "$root/initfs/usr/share/starbyte-xkb/rules/evdev" ]] \
    || fail 'guest_xkb_rules_missing'
  [[ -f "$root/initfs/usr/share/starbyte-xkb/symbols/us" ]] \
    || fail 'guest_xkb_symbols_missing'
  [[ -d /usr/share/libinput ]] || fail 'hosted_libinput_quirks_missing'
  cp -a -- /usr/share/libinput "$root/initfs/usr/share/libinput" \
    || fail 'guest_libinput_quirks_stage_failed'
  # Weston releases may load this renderer as a separate plugin.
  pixman_plugin="$(find /usr/lib -type f -name 'pixman-renderer.so' -print -quit)"
  if [[ -n "$pixman_plugin" ]]; then stage_files+=("$pixman_plugin"); fi
  mkdir -p "$root/initfs/usr/lib/udev"
  : >"$root/wayland-dependencies"
  # libinput's udev seat requires actual ID_INPUT/ID_INPUT_KEYBOARD
  # properties. The bare initramfs has devtmpfs nodes but no udev database.
  # Carry only system tools/rules into private guest RAM; no host devices.
  udev_daemon='/usr/lib/systemd/systemd-udevd'
  [[ -f "$udev_daemon" && -x "$udev_daemon" ]] || fail 'hosted_udev_daemon_missing'
  command -v udevadm >/dev/null 2>&1 || fail 'hosted_udevadm_missing'
  [[ -d /usr/lib/udev/rules.d ]] || fail 'hosted_udev_rules_missing'
  cp -a /usr/lib/udev/rules.d "$root/initfs/usr/lib/udev/" \
    || fail 'guest_udev_rules_stage_failed'
  for udev_binary in "$udev_daemon" "$(command -v udevadm)"; do
    mkdir -p "$root/initfs$(dirname "$udev_binary")"
    cp -L "$udev_binary" "$root/initfs$udev_binary" || fail 'guest_udev_binary_stage_failed'
    ldd "$udev_binary" >>"$root/wayland-dependencies" \
      || fail 'guest_udev_dependency_scan_failed'
  done
  for artifact in "${stage_files[@]}"; do
    mkdir -p "$root/initfs$(dirname "$artifact")"
    cp -L -- "$artifact" "$root/initfs$artifact" || fail 'wayland_stage_failed'
    ldd "$artifact" >>"$root/wayland-dependencies" || fail 'wayland_dependency_scan_failed'
  done
  if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
    if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
      # Original application build from the checked-out repository only.
      # The standalone Hyprland guest runs this same binary after Weston exits.
      app_binary=target/debug/starbyte-egui
      [[ -x "$app_binary" ]] || fail 'hosted_starbyte_binary_missing'
      cp -L "$app_binary" "$root/initfs/bin/starbyte-egui" \
        || fail 'guest_starbyte_binary_stage_failed'
      ldd "$app_binary" >>"$root/wayland-dependencies" \
        || fail 'guest_starbyte_dependencies_scan_failed'
    fi
    # eframe/glutin dynamically load EGL; ldd on the application itself
    # does not discover Mesa's GLVND vendor plugin or its DRM software DRI.
    # These are hosted-runner CPU rendering libraries, never the owner's GPU.
    # The glutin/winit Wayland EGL path loads these with dlopen, so
    # ldd of starbyte-egui cannot find them. Use the host distribution's
    # matching ABI libraries in guest RAM; not a host Wayland socket.
    for egl_name in libEGL.so.1 libEGL_mesa.so.0 libGLESv2.so.2 libwayland-egl.so.1 libwayland-cursor.so.0; do
      egl_binary="$(find /usr/lib -name "$egl_name" -print -quit)"
      [[ -n "$egl_binary" && -f "$egl_binary" ]] || fail "guest_egl_library_missing_$egl_name"
      mkdir -p "$root/initfs$(dirname "$egl_binary")"
      cp -L "$egl_binary" "$root/initfs$egl_binary" || fail 'guest_egl_copy_failed'
      ldd "$egl_binary" >>"$root/wayland-dependencies" || fail 'guest_egl_dependency_scan_failed'
    done
    [[ -d /usr/share/glvnd/egl_vendor.d ]] || fail 'guest_egl_vendor_manifest_missing'
    mkdir -p "$root/initfs/usr/share/glvnd"
    cp -a -- /usr/share/glvnd/egl_vendor.d "$root/initfs/usr/share/glvnd/" \
      || fail 'guest_egl_vendor_manifest_stage_failed'
    swrast_binary="$(find /usr/lib -name 'swrast_dri.so' -print -quit)"
    [[ -n "$swrast_binary" && -f "$swrast_binary" ]] || fail 'guest_swrast_driver_missing'
    mkdir -p "$root/initfs$(dirname "$swrast_binary")"
    cp -L "$swrast_binary" "$root/initfs$swrast_binary" || fail 'guest_swrast_driver_copy_failed'
    ldd "$swrast_binary" >>"$root/wayland-dependencies" || fail 'guest_swrast_dependency_scan_failed'
    if [[ "$mode" == --starbyte-smoke ]]; then : >"$root/initfs/etc/starbyte-app-smoke"; fi
  fi
  if [[ "$mode" == --hyprland-smoke ]]; then
    # Only distro-packaged binaries and their shared dependencies enter
    # guest RAM. Host devices, desktop sockets and personal files do not.
    command -v Hyprland >/dev/null 2>&1 || fail 'hosted_hyprland_binary_missing'
    hyprland_bin="$(command -v Hyprland)"
    mkdir -p "$root/initfs$(dirname "$hyprland_bin")"
    cp -L "$hyprland_bin" "$root/initfs$hyprland_bin" || fail 'guest_hyprland_binary_copy_failed'
    ldd "$hyprland_bin" >>"$root/wayland-dependencies" || fail 'guest_hyprland_dependencies_failed'
    if [[ -d /usr/share/hyprland ]]; then
      cp -a /usr/share/hyprland "$root/initfs/usr/share/" || fail 'guest_hyprland_assets_failed'
    fi
    : >"$root/initfs/etc/starbyte-hyprland-smoke"
    # Independently test the exact staged Mesa EGL implementation inside
    # the private VM, not a host display or the user's machine.
    command -v eglinfo >/dev/null 2>&1 || fail 'guest_eglinfo_package_missing'
    eglinfo_bin="$(readlink -f -- "$(command -v eglinfo)")"
    [[ -x "$eglinfo_bin" ]] || fail 'guest_eglinfo_binary_missing'
    cp -L "$eglinfo_bin" "$root/initfs/bin/eglinfo" \
      || fail 'guest_eglinfo_stage_failed'
    ldd "$eglinfo_bin" >>"$root/wayland-dependencies" \
      || fail 'guest_eglinfo_dependency_scan_failed'
    rustc --edition=2021 -C opt-level=1 scripts/qa-guest-egl.rs \
      -o "$root/initfs/bin/egl-attach-probe" \
      || fail 'guest_egl_attach_probe_compilation_failed'
    ldd "$root/initfs/bin/egl-attach-probe" >>"$root/wayland-dependencies" \
      || fail 'guest_egl_attach_probe_dependency_scan_failed'
    rustc --edition=2021 -C opt-level=1 scripts/qa-guest-xdg-surface.rs \
      -o "$root/initfs/bin/xdg-surface-probe" \
      || fail 'guest_xdg_surface_probe_compilation_failed'
    ldd "$root/initfs/bin/xdg-surface-probe" >>"$root/wayland-dependencies" \
      || fail 'guest_xdg_surface_probe_dependency_scan_failed'
  fi
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
    # Do not rely on free space or permissions in the unpacked initramfs.
    # All mounts belong solely to this disposable guest.
    /bin/busybox mkdir -p /tmp /run
    /bin/busybox mount -t tmpfs -o size=64m,mode=1777 tmpfs /tmp
    tmp_mount_status=$?
    /bin/busybox mount -t tmpfs -o size=32m,mode=0755 tmpfs /run
    run_mount_status=$?
    echo "STARBYTE_GUEST_RUN_MOUNT status=$run_mount_status" >/dev/console
    echo "STARBYTE_GUEST_TMP_MOUNT status=$tmp_mount_status" >/dev/console
    if [ "$tmp_mount_status" != 0 ] || [ "$run_mount_status" != 0 ]; then
      /bin/busybox reboot -f
    fi
    /bin/busybox mkdir -p /run/user/0
    /bin/busybox chmod 700 /run/user/0
    export XDG_RUNTIME_DIR=/run/user/0
    # No guest VT is attached to the TCG-only serial console; libseat's
    # builtin backend must not try to bind to a physical terminal.
    export LIBSEAT_BACKEND=builtin
    export SEATD_VTBOUND=0
    export XDG_SEAT=seat0
    export HOME=/tmp
    export PATH=/usr/bin:/bin
    export MESA_LOADER_DRIVER_OVERRIDE=kms_swrast
    export LIBGL_ALWAYS_SOFTWARE=1
    export GBM_BACKEND=dri
    export GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm
    export LIBGL_DRIVERS_PATH=/usr/lib/x86_64-linux-gnu/dri
    export XKB_CONFIG_ROOT=/usr/share/starbyte-xkb
    xkb_rules=0
    xkb_symbols=0
    if [ -f "$XKB_CONFIG_ROOT/rules/evdev" ]; then xkb_rules=1; fi
    if [ -f "$XKB_CONFIG_ROOT/symbols/us" ]; then xkb_symbols=1; fi
    echo "STARBYTE_GUEST_XKB_DATA rules=$xkb_rules symbols=$xkb_symbols" >/dev/console
    # Load classification rules for the guest's virtual input devices.
    # Never forward any physical host device or seat.
    /usr/lib/systemd/systemd-udevd --daemon >/tmp/udevd.log 2>&1
    udev_daemon_status=$?
    /usr/bin/udevadm trigger --action=add --subsystem-match=input \
      >/tmp/udev-trigger.log 2>&1
    udev_trigger_status=$?
    /usr/bin/udevadm settle --timeout=8 >/tmp/udev-settle.log 2>&1
    udev_settle_status=$?
    udev_keyboard=0
    udev_event=0
    for node in /dev/input/event*; do
      [ -c "$node" ] || continue
      udev_event=1
      /usr/bin/udevadm info --query=property --name="$node" \
        >/tmp/udev-device.log 2>/dev/null
      if /bin/busybox grep -Fqx 'ID_INPUT=1' /tmp/udev-device.log &&
        /bin/busybox grep -Fqx 'ID_INPUT_KEYBOARD=1' /tmp/udev-device.log; then
        udev_keyboard=1
      fi
    done
    echo "STARBYTE_GUEST_UDEV_RESULT daemon=$udev_daemon_status trigger=$udev_trigger_status settle=$udev_settle_status event=$udev_event keyboard=$udev_keyboard" >/dev/console
    # Prove both actual binaries execute before interpreting compositor
    # startup or protocol silence. No raw program output leaves guest RAM.
    guest_tmp_write_status=0
    echo guest-private-write >/tmp/starbyte-write-check || guest_tmp_write_status=$?
    guest_tmp=0
    if [ -s /tmp/starbyte-write-check ]; then guest_tmp=1; fi
    echo "STARBYTE_GUEST_TMP_IO_DIAG write_status=$guest_tmp_write_status" >/dev/console
    echo "STARBYTE_GUEST_TMP_WRITE_OK result=$guest_tmp" >/dev/console
    wire_executable=0
    weston_executable=0
    [ -x /bin/wayland-wire ] && wire_executable=1
    [ -x /usr/bin/weston ] && weston_executable=1
    echo "STARBYTE_GUEST_WAYLAND_BIN_DIAG wire=$wire_executable weston=$weston_executable" >/dev/console
    # An instantaneous self-check should not rely on BusyBox timeout's
    # process-launch semantics. The outer QEMU host timeout remains active.
    /bin/wayland-wire --self-check >/tmp/wayland-selfcheck.log 2>/tmp/wayland-selfcheck.err
    wire_self_status=$?
    wire_self_output=0
    if /bin/busybox grep -Fqx 'STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST_OK frames=valid' /tmp/wayland-selfcheck.log; then wire_self_output=1; fi
    wire_self_stderr=0
    wire_self_reason=none
    if [ -s /tmp/wayland-selfcheck.err ]; then wire_self_stderr=1; fi
    if /bin/busybox grep -Eiq 'Permission denied|Operation not permitted' /tmp/wayland-selfcheck.err; then
      wire_self_reason=permission
    elif /bin/busybox grep -Eiq 'Exec format error' /tmp/wayland-selfcheck.err; then
      wire_self_reason=exec_format
    elif /bin/busybox grep -Eiq 'No such file|not found' /tmp/wayland-selfcheck.err; then
      wire_self_reason=missing_interpreter
    elif /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object' /tmp/wayland-selfcheck.err; then
      wire_self_reason=library
    fi
    echo "STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST status=$wire_self_status output=$wire_self_output stderr=$wire_self_stderr reason=$wire_self_reason" >/dev/console
    if [ "$wire_self_status" != 0 ] || [ "$wire_self_output" != 1 ]; then
      /bin/busybox reboot -f
    fi
    /usr/bin/weston --version >/tmp/weston-version.log 2>/tmp/weston-version.err
    weston_version_status=$?
    weston_version_output=0
    weston_version_stderr=0
    if [ -s /tmp/weston-version.log ]; then weston_version_output=1; fi
    if [ -s /tmp/weston-version.err ]; then weston_version_stderr=1; fi
    echo "STARBYTE_GUEST_WESTON_EXEC status=$weston_version_status stdout=$weston_version_output stderr=$weston_version_stderr" >/dev/console
    if [ "$weston_version_status" != 0 ]; then
      /bin/busybox reboot -f
    fi
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
    wire_last_status=0
    wayland_attempts=0
    # Auxiliary human-readable client sample; authoritative acceptance
    # comes exclusively from the independent Rust wire registry barrier.
    if [ "$socket_ready" = 1 ]; then
      wayland_attempts=1
      WAYLAND_DEBUG=client WAYLAND_DISPLAY=starbyte-wayland-0 \
        /bin/busybox timeout 4 /usr/bin/wayland-info \
        >/tmp/wayland-info.log 2>/tmp/wayland-info.err
      if [ "$?" = 0 ]; then handshake=1; fi
    fi
    # The Rust wire client uses wl_display.get_registry followed by sync.
    # Its wl_callback.done is the registry barrier; a clean wayland-info exit
    # with no output is not proof of any actual protocol messages.
    if [ "$socket_ready" = 1 ]; then
      for retry in 1 2 3 4 5 6 7 8; do
        wire_attempts=$retry
        WAYLAND_DISPLAY=starbyte-wayland-0 \
          /bin/wayland-wire \
          >/tmp/wayland-wire.log 2>/tmp/wayland-wire.err
        wire_last_status=$?
        if /bin/busybox grep -Eq \
          '^STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor=1 seat=1 globals=[1-9][0-9]*$' \
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
    # Weston kiosk shell focuses its first fullscreen xdg-toplevel window.
    # This official sample exits on *received* wl_keyboard Escape-down.
    # Merely injecting QMP commands or seeing evdev events is insufficient.
    focus_ready=0
    focus_clean_exit=0
    focus_exit_message=0
    focus_weston_alive=0
    focus_sample_status=unknown
    if [ "$compositor" = 1 ] && [ "$seat" = 1 ] && [ "$wire_done" = 1 ]; then
      WAYLAND_DISPLAY=starbyte-wayland-0 \
        /usr/bin/weston-simple-shm >/tmp/focus-client.log 2>/tmp/focus-client.err &
      focus_pid=$!
      /bin/busybox sleep 3
      if /bin/busybox kill -0 "$focus_pid" 2>/dev/null; then
        focus_ready=1
        echo 'STARBYTE_GUEST_FOCUS_READY client=weston-simple-shm alive=1' >/dev/console
        for focus_retry in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
          if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/focus-client.err; then break; fi
          if ! /bin/busybox kill -0 "$focus_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
      fi
      if /bin/busybox kill -0 "$focus_pid" 2>/dev/null; then
        /bin/busybox kill "$focus_pid" 2>/dev/null || :
      fi
      wait "$focus_pid" 2>/dev/null
      focus_sample_status=$?
      if [ "$focus_sample_status" = 0 ]; then focus_clean_exit=1; fi
      if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/focus-client.err 2>/dev/null; then focus_exit_message=1; fi
      if /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then focus_weston_alive=1; fi
    fi
    echo "STARBYTE_GUEST_FOCUS_RESULT ready=$focus_ready clean_exit=$focus_clean_exit exit_message=$focus_exit_message weston_alive=$focus_weston_alive" >/dev/console
    if [ -e /etc/starbyte-app-smoke ]; then
      # The real original egui library + exclusive SNES worker, in the VM.
      # The built-in demo needs no copyrighted ROM or private user files.
      app_exit=unknown
      app_frames=0
      app_single_owner=0
      app_launcher=1
      # Weston needs its DRM GBM backend, but the Wayland client needs
      # an EGL software rasterizer. Do not force Weston's kms_swrast into
      # the guest application's EGL context.
      WAYLAND_DISPLAY=starbyte-wayland-0 \
        RUST_BACKTRACE=1 \
        EGL_LOG_LEVEL=debug \
        LIBGL_DEBUG=verbose \
        WINIT_UNIX_BACKEND=wayland \
        EGL_PLATFORM=wayland \
        MESA_LOADER_DRIVER_OVERRIDE=swrast \
        GALLIUM_DRIVER=llvmpipe \
        XDG_CONFIG_HOME=/tmp/starbyte-config \
        XDG_CACHE_HOME=/tmp/starbyte-xdg-cache \
        /bin/starbyte-egui --demo --native-worker-preview --exit-after-frames 30 \
          --config /tmp/starbyte-app-config.toml \
          --cache-dir /tmp/starbyte-app-cache \
          >/tmp/starbyte-app.log 2>/tmp/starbyte-app.err
      app_exit=$?
      if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=' /tmp/starbyte-app.log; then app_frames=1; fi
      if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=' /tmp/starbyte-app.log; then app_single_owner=1; fi
      app_stderr=0
      app_stdout=0
      app_error_egl=0
      app_error_winit=0
      app_error_loader=0
      app_error_config=0
      app_error_render=0
      app_error_panic=0
      app_egl_initialize=0
      app_egl_bad_match=0
      app_egl_bad_alloc=0
      app_egl_bad_display=0
      app_egl_unsupported=0
      app_gl_config=0
      app_gl_context=0
      app_wayland_egl=0
      app_wayland_display=0
      app_winit_eventloop=0
      app_gbm_device=0
      app_dri_driver=0
      app_panic_eframe=0
      app_panic_glutin=0
      app_panic_winit=0
      app_error_egl_init=0
      app_error_dri=0
      app_error_context=0
      app_error_wayland=0
      if [ -s /tmp/starbyte-app.err ]; then app_stderr=1; fi
      if [ -s /tmp/starbyte-app.log ]; then app_stdout=1; fi
      for log in /tmp/starbyte-app.err /tmp/starbyte-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl|glvnd|libGL|libGLES' "$log"; then app_error_egl=1; fi
        if /bin/busybox grep -Eiq 'winit|wayland.*error|event loop|display.*failed' "$log"; then app_error_winit=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then app_error_loader=1; fi
        if /bin/busybox grep -Eiq 'config.*error|config.*failed|permission denied|read.only|no space left' "$log"; then app_error_config=1; fi
        if /bin/busybox grep -Eiq 'glutin|OpenGL|glow|surface.*error|renderer.*failed|failed.*renderer' "$log"; then app_error_render=1; fi
        if /bin/busybox grep -Eiq 'panicked at|thread.*panicked|stack backtrace' "$log"; then app_error_panic=1; fi
        if /bin/busybox grep -Eiq 'eglInitialize|EGL_NOT_INITIALIZED|Failed to initialize EGL|egl initialization failed' "$log"; then app_egl_initialize=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_MATCH' "$log"; then app_egl_bad_match=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_ALLOC' "$log"; then app_egl_bad_alloc=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_DISPLAY' "$log"; then app_egl_bad_display=1; fi
        if /bin/busybox grep -Eiq 'EGL_NOT_SUPPORTED|EGL_BAD_CONFIG|EGL_BAD_ATTRIBUTE|EGL_BAD_PARAMETER' "$log"; then app_egl_unsupported=1; fi
        if /bin/busybox grep -Eiq 'no available pixel format|NoAvailablePixelFormat|no matching config|no suitable.*config|failed to.*config' "$log"; then app_gl_config=1; fi
        if /bin/busybox grep -Eiq 'eglCreateContext|eglMakeCurrent|failed to create.*context|Failed to make.*current' "$log"; then app_gl_context=1; fi
        if /bin/busybox grep -Eiq 'libwayland-egl|wayland-egl|wl_egl_window' "$log"; then app_wayland_egl=1; fi
        if /bin/busybox grep -Eiq 'wayland.*connection|wayland.*display|wayland.*backend|Failed to connect to.*Wayland' "$log"; then app_wayland_display=1; fi
        if /bin/busybox grep -Eiq 'EventLoopError|failed to create event loop|EventLoop::new' "$log"; then app_winit_eventloop=1; fi
        if /bin/busybox grep -Eiq 'gbm_create_device|failed to create.*GBM' "$log"; then app_gbm_device=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*_dri|MESA-LOADER' "$log"; then app_dri_driver=1; fi
        if /bin/busybox grep -Eiq '^ *[0-9]+:.*eframe::' "$log"; then app_panic_eframe=1; fi
        if /bin/busybox grep -Eiq '^ *[0-9]+:.*glutin::' "$log"; then app_panic_glutin=1; fi
        if /bin/busybox grep -Eiq '^ *[0-9]+:.*winit::' "$log"; then app_panic_winit=1; fi
        if /bin/busybox grep -Eiq 'EGL_NOT_INITIALIZED|EGL_BAD_DISPLAY|failed to initialize EGL|eglInitialize failed' "$log"; then app_error_egl_init=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*dri|swrast_dri\.so|dri.*not found' "$log"; then app_error_dri=1; fi
        if /bin/busybox grep -Eiq 'NoAvailablePixelFormat|failed to create.*context|could not create.*context|failed to find EGLConfig' "$log"; then app_error_context=1; fi
        if /bin/busybox grep -Eiq 'failed to connect.*wayland|WaylandError|wayland.*connection.*failed' "$log"; then app_error_wayland=1; fi
      done
      echo "STARBYTE_GUEST_APP_ROOT_DIAG egl_init=$app_egl_initialize egl_match=$app_egl_bad_match egl_alloc=$app_egl_bad_alloc egl_display=$app_egl_bad_display egl_unsupported=$app_egl_unsupported gl_config=$app_gl_config gl_context=$app_gl_context wayland_egl=$app_wayland_egl wayland_display=$app_wayland_display eventloop=$app_winit_eventloop gbm=$app_gbm_device dri=$app_dri_driver stack_eframe=$app_panic_eframe stack_glutin=$app_panic_glutin stack_winit=$app_panic_winit" >/dev/console
      echo "STARBYTE_GUEST_APP_DIAG stdout=$app_stdout stderr=$app_stderr egl=$app_error_egl winit=$app_error_winit loader=$app_error_loader config=$app_error_config render=$app_error_render panic=$app_error_panic egl_init=$app_error_egl_init dri=$app_error_dri context=$app_error_context wayland=$app_error_wayland" >/dev/console
      echo "STARBYTE_GUEST_APP_RESULT launched=$app_launcher status=$app_exit frames=$app_frames single_owner=$app_single_owner" >/dev/console
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
    info_compositor=0
    info_seat=0
    wire_globs=0
    weston_backend=0
    weston_seat_granted=0
    weston_card_open=0
    weston_input_failure=0
    weston_backend_failure=0
    weston_input_absent=0
    weston_exit_status=unknown
    weston_failed_any=0
    weston_error_renderer=0
    weston_error_kms=0
    weston_error_outputs=0
    weston_error_backend=0
    weston_error_input=0
    weston_error_udev=0
    weston_error_libinput=0
    weston_error_xkb=0
    weston_error_keymap=0
    weston_error_quirks=0
    weston_error_fatal=0
    weston_error_crash=0
    wire_log=0
    wire_stderr=0
    wire_reason=none
    weston_log=0
    weston_stdout=0
    weston_zombie=0
    if /bin/busybox kill -0 "$weston_pid" 2>/dev/null; then compositor_alive=1; fi
    if [ "$compositor_alive" = 0 ]; then
      wait "$weston_pid" 2>/dev/null
      weston_exit_status=$?
    fi
    if [ -s /tmp/weston.log ]; then weston_log=1; fi
    if [ -s /tmp/weston.stdout ]; then weston_stdout=1; fi
    if [ -r "/proc/$weston_pid/stat" ]; then
      state="$(/bin/busybox awk '{print $3}' "/proc/$weston_pid/stat")"
      if [ "$state" = Z ]; then weston_zombie=1; fi
    fi
    if [ -s /tmp/wayland-info.log ]; then info_stdout=1; fi
    if [ -s /tmp/wayland-info.err ]; then info_stderr=1; fi
    if [ -s /tmp/wayland-wire.log ]; then wire_log=1; fi
    if [ -s /tmp/wayland-wire.err ]; then wire_stderr=1; fi
    for reason in missing_runtime missing_display unsafe_display_name connect_failed timeout_setup_failed write_failed read_failed payload_failed invalid_frame_length invalid_registry_global display_protocol_error registry_event_limit; do
      if /bin/busybox grep -Fqx "STARBYTE_GUEST_WAYLAND_WIRE_BLOCKED $reason" /tmp/wayland-wire.log 2>/dev/null; then wire_reason=$reason; break; fi
    done
    if /bin/busybox grep -q 'wl_registry' /tmp/wayland-info.err 2>/dev/null; then registry_trace=1; fi
    if /bin/busybox grep -q 'wl_compositor' /tmp/wayland-info.log 2>/dev/null ||
      /bin/busybox grep -q 'wl_compositor' /tmp/wayland-info.err 2>/dev/null; then info_compositor=1; fi
    if /bin/busybox grep -q 'wl_seat' /tmp/wayland-info.log 2>/dev/null ||
      /bin/busybox grep -q 'wl_seat' /tmp/wayland-info.err 2>/dev/null; then info_seat=1; fi
    if /bin/busybox grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 .* globals=[1-9][0-9]*$' /tmp/wayland-wire.log 2>/dev/null; then wire_globs=1; fi
    echo "STARBYTE_GUEST_WAYLAND_REGISTRY_DIAG info_compositor=$info_compositor info_seat=$info_seat wire_globals=$wire_globs" >/dev/console
    for log in /tmp/weston.log /tmp/weston.stdout /tmp/wayland-info.err; do
      [ -f "$log" ] || continue
      if /bin/busybox grep -Eiq 'failed to load module|unable to load module' "$log"; then module_error=1; fi
      if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object file' "$log"; then shared_library_error=1; fi
      if /bin/busybox grep -Eiq 'unrecognized option|unknown option|invalid option|unknown renderer|unknown shell' "$log"; then option_error=1; fi
      if /bin/busybox grep -Eiq 'failed to open seat|seatd|libseat|session control|cannot get tty|no seat' "$log"; then seat_error=1; fi
      if /bin/busybox grep -Eiq 'drm|kms|no outputs|no connectors|no usable devices' "$log"; then drm_error=1; fi
    done
    for log in /tmp/weston.log /tmp/weston.stdout; do
      [ -f "$log" ] || continue
      if /bin/busybox grep -Eiq 'initializing drm backend|drm backend initialized' "$log"; then weston_backend=1; fi
      if /bin/busybox grep -Eiq 'session control granted|Seat opened with backend' "$log"; then weston_seat_granted=1; fi
      if /bin/busybox grep -Eq 'using /dev/dri/card[0-9]+' "$log"; then weston_card_open=1; fi
      if /bin/busybox grep -Eiq 'failed to create input devices|failed to initialize udev|udev_input_init' "$log"; then weston_input_failure=1; fi
      if /bin/busybox grep -Eiq 'failed to create compositor backend|failed to create backend|fatal:' "$log"; then weston_backend_failure=1; fi
      if /bin/busybox grep -Eiq 'no input devices found' "$log"; then weston_input_absent=1; fi
    done
    for log in /tmp/weston.log /tmp/weston.stdout; do
      [ -f "$log" ] || continue
      if /bin/busybox grep -Eiq 'failed|error|fatal|crash|segmentation fault|assertion' "$log"; then weston_failed_any=1; fi
      if /bin/busybox grep -Eiq 'renderer|pixman|EGL|GBM' "$log" &&
        /bin/busybox grep -Eiq 'failed|error|fatal' "$log"; then weston_error_renderer=1; fi
      if /bin/busybox grep -Eiq 'crtc|kms|atomic|page.flip|drmMode' "$log" &&
        /bin/busybox grep -Eiq 'failed|error|fatal' "$log"; then weston_error_kms=1; fi
      if /bin/busybox grep -Eiq 'no outputs|no heads|no connectors|failed to create heads|output.*failed|no outputs available' "$log"; then weston_error_outputs=1; fi
      if /bin/busybox grep -Eiq 'failed to create compositor backend|failed to load backend|backend initialization failed' "$log"; then weston_error_backend=1; fi
      if /bin/busybox grep -Eiq 'failed to create input devices|failed to open /dev/input|libinput.*failed' "$log"; then weston_error_input=1; fi
      if /bin/busybox grep -Eiq 'failed to initialize udev|udev.*failed|failed.*udev' "$log"; then weston_error_udev=1; fi
      if /bin/busybox grep -Eiq 'libinput.*error|libinput.*failed|quirks.*error|quirks.*failed' "$log"; then weston_error_libinput=1; fi
      if /bin/busybox grep -Eiq 'xkbcommon.*error|xkb.*failed|xkb.*error|keyboard.*rules.*not found' "$log"; then weston_error_xkb=1; fi
      if /bin/busybox grep -Eiq 'failed to create keymap|failed to compile keymap|keymap.*error|keymap.*failed' "$log"; then weston_error_keymap=1; fi
      if /bin/busybox grep -Eiq 'quirks.*failed|quirks.*error|failed to load.*quirks' "$log"; then weston_error_quirks=1; fi
      if /bin/busybox grep -Eiq 'fatal:' "$log"; then weston_error_fatal=1; fi
      if /bin/busybox grep -Eiq 'segmentation fault|assertion failed|core dumped' "$log"; then weston_error_crash=1; fi
    done
    echo "STARBYTE_GUEST_WESTON_EXIT status=$weston_exit_status failed=$weston_failed_any renderer=$weston_error_renderer kms=$weston_error_kms outputs=$weston_error_outputs backend=$weston_error_backend input=$weston_error_input udev=$weston_error_udev libinput=$weston_error_libinput xkb=$weston_error_xkb keymap=$weston_error_keymap quirks=$weston_error_quirks fatal=$weston_error_fatal crash=$weston_error_crash" >/dev/console
    echo "STARBYTE_GUEST_WESTON_STAGE backend_entered=$weston_backend seat_granted=$weston_seat_granted card_open=$weston_card_open input_failed=$weston_input_failure backend_failed=$weston_backend_failure input_absent=$weston_input_absent" >/dev/console
    echo "STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=$wire_connected sync=$wire_done output=$wire_log attempts=$wire_attempts status=$wire_last_status stderr=$wire_stderr reason=$wire_reason" >/dev/console
    echo "STARBYTE_GUEST_WAYLAND_RESULT socket=$socket_ready handshake=$handshake compositor=$compositor seat=$seat" >/dev/console
    echo "STARBYTE_GUEST_WAYLAND_DIAG alive=$compositor_alive zombie=$weston_zombie weston_log=$weston_log weston_stdout=$weston_stdout module=$module_error library=$shared_library_error option=$option_error seat=$seat_error drm=$drm_error stdout=$info_stdout stderr=$info_stderr registry=$registry_trace attempts=$wayland_attempts" >/dev/console
    /bin/busybox kill "$weston_pid" 2>/dev/null || :
    wait "$weston_pid" 2>/dev/null || :
    if [ -e /etc/starbyte-hyprland-smoke ]; then
      # Weston has been reaped. The Hyprland compositor uses only this VM's
      # DRM card and keyboard. Never use the owner's active display.
      /bin/busybox cat >/tmp/starbyte-hyprland.conf <<'HYPR_CONFIG'
monitor = , preferred, auto, 1
misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
}
xwayland {
    enabled = false
}
HYPR_CONFIG
      hypr_executable=0
      hypr_socket=0
      hypr_registry=0
      hypr_compositor=0
      hypr_seat=0
      hypr_alive=0
      hypr_version=0
      hypr_config_valid=0
      hypr_wire_attempts=0
      hypr_wire_connected=0
      hypr_wire_sync=0
      hypr_wire_reason=none
      hypr_egl_error=0
      hypr_gbm_error=0
      hypr_seat_error=0
      hypr_config_error=0
      hypr_library_error=0
      hypr_monitor_error=0
      hypr_root_guard=0
      if [ -x /usr/bin/Hyprland ]; then hypr_executable=1; fi
      /usr/bin/Hyprland --version >/tmp/hypr-version.log 2>/tmp/hypr-version.err
      if [ "$?" = 0 ]; then hypr_version=1; fi
      /usr/bin/Hyprland --i-am-really-stupid --verify-config \
        --config /tmp/starbyte-hyprland.conf \
        >/tmp/hypr-verify.log 2>/tmp/hypr-verify.err
      if [ "$?" = 0 ]; then hypr_config_valid=1; fi
      HYPRLAND_NO_RT=1 AQ_DRM_DEVICES=/dev/dri/card0 \
        /usr/bin/Hyprland --i-am-really-stupid --config /tmp/starbyte-hyprland.conf \
        >/tmp/hyprland.log 2>/tmp/hyprland.err &
      hypr_pid=$!
      hypr_display=''
      for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30; do
        for socket in /run/user/0/wayland-*; do
          [ -S "$socket" ] || continue
          hypr_display="${socket##*/}"
          hypr_socket=1
          break
        done
        if [ "$hypr_socket" = 1 ]; then break; fi
        if ! /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then break; fi
        /bin/busybox sleep 1
      done
      if [ "$hypr_socket" = 1 ]; then
        # Socket creation can precede global publication, especially under
        # 1-vCPU software-emulated QEMU. Require wl_callback.done itself.
        for wire_try in 1 2 3 4 5 6 7 8; do
          hypr_wire_attempts=$wire_try
          WAYLAND_DISPLAY="$hypr_display" /bin/wayland-wire \
            >/tmp/hyprland-wire.log 2>/tmp/hyprland-wire.err
          if /bin/busybox grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor=1 seat=1 globals=[1-9][0-9]*$' /tmp/hyprland-wire.log; then
            hypr_registry=1
            hypr_compositor=1
            hypr_seat=1
            hypr_wire_connected=1
            hypr_wire_sync=1
            break
          fi
          if /bin/busybox grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_RESULT connected=1 registry_done=1 compositor=[01] seat=[01] globals=[0-9]+$' /tmp/hyprland-wire.log; then
            hypr_wire_connected=1
            hypr_wire_sync=1
            if /bin/busybox grep -Eq 'compositor=1 seat=[01]' /tmp/hyprland-wire.log; then hypr_compositor=1; fi
            if /bin/busybox grep -Eq 'compositor=[01] seat=1' /tmp/hyprland-wire.log; then hypr_seat=1; fi
          fi
          if ! /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
        for reason in missing_runtime missing_display unsafe_display_name connect_failed timeout_setup_failed write_failed read_failed payload_failed invalid_frame_length invalid_registry_global display_protocol_error registry_event_limit; do
          if /bin/busybox grep -Fqx "STARBYTE_GUEST_WAYLAND_WIRE_BLOCKED $reason" /tmp/hyprland-wire.log 2>/dev/null; then hypr_wire_reason=$reason; break; fi
        done
      fi
      # Socket and wl_seat presence alone do not establish a configured
      # DRM output. Classify actual display globals before starting clients.
      hypr_info_status=not_run
      hypr_output=0
      hypr_xdg_shell=0
      hypr_wl_shm=0
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DISPLAY="$hypr_display" /bin/busybox timeout 6 /usr/bin/wayland-info \
          >/tmp/hyprland-info.log 2>/tmp/hyprland-info.err
        hypr_info_status=$?
        if /bin/busybox grep -q 'wl_output' /tmp/hyprland-info.log /tmp/hyprland-info.err; then hypr_output=1; fi
        if /bin/busybox grep -q 'xdg_wm_base' /tmp/hyprland-info.log /tmp/hyprland-info.err; then hypr_xdg_shell=1; fi
        if /bin/busybox grep -q 'wl_shm' /tmp/hyprland-info.log /tmp/hyprland-info.err; then hypr_wl_shm=1; fi
      fi
      echo "STARBYTE_GUEST_HYPR_OUTPUT_DIAG info_status=$hypr_info_status output=$hypr_output xdg_shell=$hypr_xdg_shell shm=$hypr_wl_shm" >/dev/console
      # A published wl_output may exist before the guest DRM connector is
      # actively scan-out enabled. Inspect only synthetic guest sysfs nodes.
      hypr_kms_connectors=0
      hypr_kms_enabled=0
      for state in /sys/class/drm/card*-*/enabled; do
        [ -r "$state" ] || continue
        hypr_kms_connectors=$((hypr_kms_connectors + 1))
        if /bin/busybox grep -Fqx enabled "$state"; then
          hypr_kms_enabled=$((hypr_kms_enabled + 1))
        fi
      done
      echo "STARBYTE_GUEST_HYPR_KMS_DIAG connectors=$hypr_kms_connectors enabled=$hypr_kms_enabled" >/dev/console
      # Read actual wl_shm format events and wl_seat keyboard capability over
      # the private guest Wayland connection. Registry names alone are insufficient.
      hypr_caps_status=not_run
      hypr_caps_parsed=0
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DISPLAY="$hypr_display" /bin/wayland-wire --capabilities \
          >/tmp/hyprland-capabilities.log 2>/tmp/hyprland-capabilities.err
        hypr_caps_status=$?
        if /bin/busybox grep -Eq '^STARBYTE_GUEST_WAYLAND_CAP_RESULT keyboard=[01] shm_argb=[01] shm_xrgb=[01] formats=[0-9]+$' /tmp/hyprland-capabilities.log; then
          hypr_caps_parsed=1
          /bin/busybox grep -E '^STARBYTE_GUEST_WAYLAND_CAP_RESULT keyboard=[01] shm_argb=[01] shm_xrgb=[01] formats=[0-9]+$' /tmp/hyprland-capabilities.log >/dev/console
        else
          for reason in missing_runtime missing_display unsafe_display_name connect_failed timeout_setup_failed write_failed read_failed invalid_frame_length payload_failed invalid_registry_global display_protocol_error registry_event_limit seat_global_missing shm_global_missing capability_event_limit; do
            if /bin/busybox grep -Fqx "STARBYTE_GUEST_WAYLAND_CAP_BLOCKED $reason" /tmp/hyprland-capabilities.log; then
              echo "STARBYTE_GUEST_HYPR_CAP_BLOCKED reason=$reason" >/dev/console
              break
            fi
          done
        fi
      fi
      echo "STARBYTE_GUEST_HYPR_CAP_DIAG status=$hypr_caps_status parsed=$hypr_caps_parsed" >/dev/console
      # Mesa eglinfo probes both platforms with the same guest software
      # renderer. Keep raw diagnostics inside the disposable VM.
      hypr_eglinfo_status=not_run
      hypr_eglinfo_wayland=0
      hypr_eglinfo_surfaceless=0
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DISPLAY="$hypr_display" EGL_PLATFORM=wayland \
          MESA_LOADER_DRIVER_OVERRIDE=swrast LIBGL_ALWAYS_SOFTWARE=1 \
          /bin/busybox timeout 20 /bin/eglinfo -B \
          >/tmp/hypr-eglinfo.log 2>/tmp/hypr-eglinfo.err
        hypr_eglinfo_status=$?
        /bin/busybox awk '
          /Wayland platform:/ { p = "wayland"; next }
          /Surfaceless platform:/ { p = "surfaceless"; next }
          /platform:/ { p = "other"; next }
          /EGL API version:|EGL version string:/ {
            if (p == "wayland") wayland = 1;
            if (p == "surfaceless") surfaceless = 1;
          }
          END { printf "STARBYTE_GUEST_EGLINFO_RESULT wayland=%d surfaceless=%d\n", wayland, surfaceless; }
        ' /tmp/hypr-eglinfo.log >/tmp/hypr-eglinfo-result.log
        if /bin/busybox grep -Fqx 'STARBYTE_GUEST_EGLINFO_RESULT wayland=1 surfaceless=1' /tmp/hypr-eglinfo-result.log; then
          hypr_eglinfo_wayland=1
          hypr_eglinfo_surfaceless=1
        elif /bin/busybox grep -Fqx 'STARBYTE_GUEST_EGLINFO_RESULT wayland=1 surfaceless=0' /tmp/hypr-eglinfo-result.log; then
          hypr_eglinfo_wayland=1
        elif /bin/busybox grep -Fqx 'STARBYTE_GUEST_EGLINFO_RESULT wayland=0 surfaceless=1' /tmp/hypr-eglinfo-result.log; then
          hypr_eglinfo_surfaceless=1
        fi
      fi
      echo "STARBYTE_GUEST_EGLINFO_DIAG status=$hypr_eglinfo_status wayland=$hypr_eglinfo_wayland surfaceless=$hypr_eglinfo_surfaceless" >/dev/console
      # Unlike eglinfo's EGL_DEFAULT_DISPLAY, pass a live wl_display pointer
      # to EGL directly. This matches the compositor-facing GUI integration.
      hypr_egl_attach_status=not_run
      hypr_egl_attach_parsed=0
      if [ "$hypr_registry" = 1 ]; then
        STARBYTE_GUEST_EGL_PROBE=1 WAYLAND_DISPLAY="$hypr_display" \
          MESA_LOADER_DRIVER_OVERRIDE=swrast LIBGL_ALWAYS_SOFTWARE=1 \
          /bin/busybox timeout 20 /bin/egl-attach-probe \
          >/tmp/hypr-egl-attach.log 2>/tmp/hypr-egl-attach.err
        hypr_egl_attach_status=$?
        if /bin/busybox grep -Eq '^STARBYTE_GUEST_EGL_ATTACHED_RESULT connected=[01] attached=[01] own=[01] surfaceless=[01] attached_error=0x[0-9a-f]{4} own_error=0x[0-9a-f]{4} surfaceless_error=0x[0-9a-f]{4}$' /tmp/hypr-egl-attach.log; then
          hypr_egl_attach_parsed=1
          /bin/busybox grep -E '^STARBYTE_GUEST_EGL_ATTACHED_RESULT connected=[01] attached=[01] own=[01] surfaceless=[01] attached_error=0x[0-9a-f]{4} own_error=0x[0-9a-f]{4} surfaceless_error=0x[0-9a-f]{4}$' /tmp/hypr-egl-attach.log >/dev/console
        fi
        # Test the EGL_KHR_display_reference attribute path used by Glutin.
        /bin/busybox grep -E '^STARBYTE_GUEST_EGL_NONE_RESULT initialized=[01] error=0x[0-9a-f]{4}$' /tmp/hypr-egl-attach.log >/dev/console || :
        /bin/busybox grep -E '^STARBYTE_GUEST_EGL_TRACKED_RESULT advertised=[01] initialized=[01] error=0x[0-9a-f]{4}$' /tmp/hypr-egl-attach.log >/dev/console || :
      fi
      echo "STARBYTE_GUEST_EGL_ATTACH_DIAG status=$hypr_egl_attach_status parsed=$hypr_egl_attach_parsed" >/dev/console
      # Prove that the virtual compositor actually sends xdg_surface.configure,
      # not merely that the registry advertises xdg_wm_base.
      hypr_xdg_status=not_run
      hypr_xdg_configured=0
      hypr_xdg_reason=none
      if [ "$hypr_registry" = 1 ]; then
        STARBYTE_GUEST_XDG_PROBE=1 WAYLAND_DISPLAY="$hypr_display" \
          /bin/busybox timeout 16 /bin/xdg-surface-probe \
          >/tmp/hypr-xdg-probe.log 2>/tmp/hypr-xdg-probe.err
        hypr_xdg_status=$?
        if /bin/busybox grep -Fqx 'STARBYTE_GUEST_XDG_SURFACE_RESULT connected=1 xdg=1 committed=1 configured=1 acked=1' /tmp/hypr-xdg-probe.log; then
          hypr_xdg_configured=1
        else
          for reason in private_guest_only missing_display unsafe_display_name connect_failed timeout_failed write_failed read_failed invalid_request invalid_event invalid_global protocol_error registry_timeout compositor_missing xdg_shell_missing configure_timeout ack_timeout; do
            if /bin/busybox grep -Fqx "STARBYTE_GUEST_XDG_SURFACE_BLOCKED $reason" /tmp/hypr-xdg-probe.log; then hypr_xdg_reason=$reason; break; fi
          done
        fi
      fi
      # Only bounded interface names and numeric protocol codes cross out of
      # this disposable VM; Wayland's arbitrary error message stays private.
      /bin/busybox grep -E '^STARBYTE_GUEST_XDG_ERROR_DIAG interface=(display|registry|sync|compositor|surface|xdg_wm_base|xdg_surface|xdg_toplevel|malformed|other) code=([0-9]|1[0-6]|other)
      hypr_focus_ready=0
      hypr_focus_clean=0
      hypr_focus_message=0
      hypr_focus_compositor_alive=0
      hypr_focus_status=not_run
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          /usr/bin/weston-simple-shm >/tmp/hypr-focus.log 2>/tmp/hypr-focus.err &
        hypr_focus_pid=$!
        # Only report readiness after wl_keyboard.enter, rather than a
        # fixed three-second liveness check on an unfocused window.
        for focus_probe in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
          if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' /tmp/hypr-focus.err 2>/dev/null &&
             /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
            hypr_focus_ready=1
            echo 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' >/dev/console
            break
          fi
          if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
        if [ "$hypr_focus_ready" = 1 ]; then
          for focus_wait in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
            if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err; then break; fi
            if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
            /bin/busybox sleep 1
          done
        fi
        if /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
          /bin/busybox kill "$hypr_focus_pid" 2>/dev/null || :
        fi
        wait "$hypr_focus_pid" 2>/dev/null
        hypr_focus_status=$?
        if [ "$hypr_focus_status" = 0 ]; then hypr_focus_clean=1; fi
        if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err 2>/dev/null; then hypr_focus_message=1; fi
        if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_focus_compositor_alive=1; fi
      fi
      hypr_focus_stdout=0
      hypr_focus_stderr=0
      hypr_focus_connect=0
      hypr_focus_library=0
      hypr_focus_protocol=0
      hypr_focus_xdg=0
      hypr_focus_shm=0
      hypr_focus_surface=0
      hypr_focus_keyboard_enter=0
      hypr_focus_keyboard_requested=0
      hypr_focus_xdg_configure=0
      hypr_focus_shm_advertised=0
      hypr_focus_registry_requested=0
      hypr_focus_create_surface=0
      hypr_focus_create_xdg=0
      hypr_focus_create_toplevel=0
      hypr_focus_initial_commit=0
      hypr_focus_display_error=0
      hypr_focus_toplevel_close=0
      if [ -s /tmp/hypr-focus.log ]; then hypr_focus_stdout=1; fi
      if [ -s /tmp/hypr-focus.err ]; then hypr_focus_stderr=1; fi
      for log in /tmp/hypr-focus.log /tmp/hypr-focus.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'failed to connect|connection refused|cannot connect|No such file.*wayland' "$log"; then hypr_focus_connect=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_focus_library=1; fi
        if /bin/busybox grep -Eiq 'protocol error|wl_display.*error|wayland.*error|display.*disconnected' "$log"; then hypr_focus_protocol=1; fi
        if /bin/busybox grep -Eiq 'xdg_wm_base|xdg_surface|xdg_toplevel|xdg-shell' "$log"; then hypr_focus_xdg=1; fi
        if /bin/busybox grep -Eiq 'wl_shm|shared memory|shm.*failed' "$log"; then hypr_focus_shm=1; fi
        if /bin/busybox grep -Eiq 'failed to.*(surface|window)|surface.*failed|failed.*buffer' "$log"; then hypr_focus_surface=1; fi
        if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' "$log"; then hypr_focus_keyboard_enter=1; fi
        if /bin/busybox grep -Eq 'wl_seat@[0-9]+\.get_keyboard\(' "$log"; then hypr_focus_keyboard_requested=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_focus_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'wl_registry@[0-9]+\.global.*wl_shm' "$log"; then hypr_focus_shm_advertised=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_focus_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_focus_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_focus_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_focus_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_focus_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_focus_display_error=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.close\(' "$log"; then hypr_focus_toplevel_close=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_FOCUS_DIAG status=$hypr_focus_status stdout=$hypr_focus_stdout stderr=$hypr_focus_stderr connect=$hypr_focus_connect library=$hypr_focus_library protocol=$hypr_focus_protocol xdg=$hypr_focus_xdg shm=$hypr_focus_shm surface=$hypr_focus_surface keyboard_enter=$hypr_focus_keyboard_enter keyboard_requested=$hypr_focus_keyboard_requested xdg_configure=$hypr_focus_xdg_configure shm_advertised=$hypr_focus_shm_advertised registry_requested=$hypr_focus_registry_requested create_surface=$hypr_focus_create_surface create_xdg=$hypr_focus_create_xdg create_toplevel=$hypr_focus_create_toplevel initial_commit=$hypr_focus_initial_commit display_error=$hypr_focus_display_error toplevel_close=$hypr_focus_toplevel_close" >/dev/console
      echo "STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=$hypr_focus_ready clean_exit=$hypr_focus_clean exit_message=$hypr_focus_message hyprland_alive=$hypr_focus_compositor_alive" >/dev/console
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_alive=1; fi
      for log in /tmp/hyprland.log /tmp/hyprland.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|egl_not_initialized' "$log"; then hypr_egl_error=1; fi
        if /bin/busybox grep -Eiq 'gbm.*failed|failed.*gbm|no allocator' "$log"; then hypr_gbm_error=1; fi
        if /bin/busybox grep -Eiq 'libseat.*failed|seat.*failed|session inactive' "$log"; then hypr_seat_error=1; fi
        if /bin/busybox grep -Eiq 'config.*failed|config.*error|parse.*error' "$log"; then hypr_config_error=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|failed.*shader' "$log"; then hypr_library_error=1; fi
        if /bin/busybox grep -Eiq 'no monitor|no output|no suitable mode|failed to commit.*output|create.*head.*failed|mode.*not available|output.*disabled' "$log"; then hypr_monitor_error=1; fi
        if /bin/busybox grep -Eiq 'superuser privileges|shall not be run as the root|privilege check is not omitted' "$log"; then hypr_root_guard=1; fi
      done
      # Reuse the original full-library GUI smoke with Hyprland as the
      # Wayland server, never the owner's display. One exclusive worker
      # must execute 30 demo frames without a shadow legacy session.
      hypr_app_status=not_run
      hypr_app_frames=0
      hypr_app_single_owner=0
      if [ "$hypr_registry" = 1 ] && [ "$hypr_seat" = 1 ]; then
        # The private Hyprland test compared swrast with kms_swrast:
        # both returned EGL_NOT_INITIALIZED. Retain the Weston-proven
        # swrast client setting; no accepted guest GUI cutover is implied.
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          RUST_BACKTRACE=1 \
          STARBYTE_LOG=starbyte=info,eframe=trace,glutin=trace,winit=trace \
          EGL_LOG_LEVEL=debug LIBGL_DEBUG=verbose \
          WINIT_UNIX_BACKEND=wayland \
          EGL_PLATFORM=wayland \
          MESA_LOADER_DRIVER_OVERRIDE=swrast \
          LIBGL_ALWAYS_SOFTWARE=1 \
          GALLIUM_DRIVER=llvmpipe \
          XDG_CONFIG_HOME=/tmp/starbyte-hyprland-app-config \
          XDG_CACHE_HOME=/tmp/starbyte-hyprland-app-cache \
          /bin/starbyte-egui --demo --native-worker-preview --exit-after-frames 30 \
            --config /tmp/starbyte-hyprland-app-config.toml \
            --cache-dir /tmp/starbyte-hyprland-cache \
            >/tmp/starbyte-hyprland-app.log 2>/tmp/starbyte-hyprland-app.err
        hypr_app_status=$?
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_frames=1; fi
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_single_owner=1; fi
      fi
      hypr_app_stdout=0
      hypr_app_stderr=0
      hypr_app_egl=0
      hypr_app_dri=0
      hypr_app_context=0
      hypr_app_wayland=0
      hypr_app_winit=0
      hypr_app_panic=0
      hypr_app_library=0
      hypr_app_config=0
      hypr_app_render=0
      hypr_app_permission=0
      hypr_app_protocol=0
      hypr_app_stack_eframe=0
      hypr_app_stack_glutin=0
      hypr_app_stack_winit=0
      hypr_app_stack_starbyte=0
      hypr_app_pixel_format=0
      hypr_app_surface=0
      hypr_app_missing_global=0
      hypr_app_egl_bad_surface=0
      hypr_app_egl_bad_native_window=0
      hypr_app_egl_create_window_surface=0
      hypr_app_surface_init_failure=0
      hypr_app_xdg_configure=0
      hypr_app_dmabuf_failed=0
      hypr_app_dmabuf_params=0
      hypr_app_wl_surface_attach=0
      hypr_app_wl_drm=0
      hypr_app_registry_requested=0
      hypr_app_create_surface=0
      hypr_app_create_xdg=0
      hypr_app_create_toplevel=0
      hypr_app_initial_commit=0
      hypr_app_display_error=0
      hypr_app_egl_bad_alloc=0
      hypr_app_egl_bad_match=0
      hypr_app_egl_not_initialized=0
      hypr_app_glutin_error=0
      hypr_app_surface_error=0
      hypr_app_backtrace=0
      hypr_app_panic_line=0
      if [ -s /tmp/starbyte-hyprland-app.log ]; then hypr_app_stdout=1; fi
      if [ -s /tmp/starbyte-hyprland-app.err ]; then hypr_app_stderr=1; fi
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|EGL_NOT_INITIALIZED|EGL_BAD_DISPLAY|eglInitialize' "$log"; then hypr_app_egl=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*dri|swrast_dri.so|MESA-LOADER' "$log"; then hypr_app_dri=1; fi
        if /bin/busybox grep -Eiq 'NoAvailablePixelFormat|failed to create.*context|no matching config|eglCreateContext|eglMakeCurrent' "$log"; then hypr_app_context=1; fi
        if /bin/busybox grep -Eiq 'wayland.*connection|failed to connect.*wayland|WaylandError|Wayland backend' "$log"; then hypr_app_wayland=1; fi
        if /bin/busybox grep -Eiq 'EventLoopError|failed to create event loop|winit::|event loop.*failed' "$log"; then hypr_app_winit=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_app_library=1; fi
        if /bin/busybox grep -Eiq 'config.*error|config.*failed|permission denied.*config|read.only|no space left' "$log"; then hypr_app_config=1; fi
        if /bin/busybox grep -Eiq 'glutin|OpenGL|surface.*error|renderer.*failed|failed.*renderer' "$log"; then hypr_app_render=1; fi
        if /bin/busybox grep -Eiq 'Permission denied|Operation not permitted' "$log"; then hypr_app_permission=1; fi
        if /bin/busybox grep -Eiq 'protocol error|xdg_wm_base|xdg_surface|xdg_toplevel|wl_display.*error' "$log"; then hypr_app_protocol=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*eframe::' "$log"; then hypr_app_stack_eframe=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*glutin::' "$log"; then hypr_app_stack_glutin=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*winit::' "$log"; then hypr_app_stack_winit=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*starbyte' "$log"; then hypr_app_stack_starbyte=1; fi
        if /bin/busybox grep -Eiq 'pixel.*format|no matching.*config|no EGL.*config|failed to find.*config' "$log"; then hypr_app_pixel_format=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_SURFACE|EGL_BAD_NATIVE_WINDOW|eglCreateWindowSurface|surface.*init.*fail' "$log"; then hypr_app_surface=1; fi
        if /bin/busybox grep -Eiq 'missing.*(global|extension)|does not support.*(protocol|interface)|required.*(global|extension)' "$log"; then hypr_app_missing_global=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_SURFACE' "$log"; then hypr_app_egl_bad_surface=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_NATIVE_WINDOW' "$log"; then hypr_app_egl_bad_native_window=1; fi
        if /bin/busybox grep -Fq 'eglCreateWindowSurface' "$log"; then hypr_app_egl_create_window_surface=1; fi
        if /bin/busybox grep -Eiq 'surface.*init.*fail' "$log"; then hypr_app_surface_init_failure=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_app_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.failed\(' "$log"; then hypr_app_dmabuf_failed=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.' "$log"; then hypr_app_dmabuf_params=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.attach\(' "$log"; then hypr_app_wl_surface_attach=1; fi
        if /bin/busybox grep -Fq 'wl_drm' "$log"; then hypr_app_wl_drm=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_app_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_app_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_app_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_app_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_app_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_app_display_error=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_ALLOC' "$log"; then hypr_app_egl_bad_alloc=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_MATCH' "$log"; then hypr_app_egl_bad_match=1; fi
        if /bin/busybox grep -Fq 'EGL_NOT_INITIALIZED' "$log"; then hypr_app_egl_not_initialized=1; fi
        if /bin/busybox grep -Eiq '(glutin|GL surface|GlSurface|gl_surface).*(error|fail)|Glutin\(' "$log"; then hypr_app_glutin_error=1; fi
        if /bin/busybox grep -Eiq '(surface|window).*(creation|initialization|configure).*(fail|error)|failed to initialize a window' "$log"; then hypr_app_surface_error=1; fi
        if /bin/busybox grep -Fq 'stack backtrace:' "$log"; then hypr_app_backtrace=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic_line=1; fi
      done
      # Map eframe 0.32's own GL launch messages to fixed stages. The
      # original logs stay private; these flags do not weaken acceptance.
      hypr_eframe_display_start=0
      hypr_eframe_display_ready=0
      hypr_eframe_context_start=0
      hypr_eframe_surface_start=0
      hypr_eframe_surface_ready=0
      hypr_eframe_context_current=0
      hypr_eframe_window_failure=0
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fq 'trying to create glutin Display with config:' "$log"; then hypr_eframe_display_start=1; fi
        if /bin/busybox grep -Fq 'successfully created GL Display with version' "$log"; then hypr_eframe_display_ready=1; fi
        if /bin/busybox grep -Fq 'creating gl context using raw window handle:' "$log"; then hypr_eframe_context_start=1; fi
        if /bin/busybox grep -Fq 'Creating a gl_surface for viewport' "$log"; then hypr_eframe_surface_start=1; fi
        if /bin/busybox grep -Fq 'surface created successfully:' "$log"; then hypr_eframe_surface_ready=1; fi
        if /bin/busybox grep -Fq 'made context current. setting swap interval' "$log"; then hypr_eframe_context_current=1; fi
        if /bin/busybox grep -Fq 'Failed to initialize a window for viewport' "$log"; then hypr_eframe_window_failure=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_EFRAME_DIAG display_start=$hypr_eframe_display_start display_ready=$hypr_eframe_display_ready context_start=$hypr_eframe_context_start surface_start=$hypr_eframe_surface_start surface_ready=$hypr_eframe_surface_ready context_current=$hypr_eframe_context_current window_failure=$hypr_eframe_window_failure" >/dev/console
      # Match glutin 0.32 ErrorKind's fixed public display strings.
      # Never print raw native errors, Wayland traces, paths or guest logs.
      hypr_app_glutin_kind=unclassified
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fqi 'argument does not refer to a valid native window' "$log"; then
          hypr_app_glutin_kind=bad_native_window
        elif /bin/busybox grep -Fqi 'argument does not name a valid surface' "$log"; then
          hypr_app_glutin_kind=bad_surface
        elif /bin/busybox grep -Fqi 'argument does not name a valid display' "$log"; then
          hypr_app_glutin_kind=bad_display
        elif /bin/busybox grep -Fqi 'argument does not name a valid config' "$log"; then
          hypr_app_glutin_kind=bad_config
        elif /bin/busybox grep -Fqi 'argument does not name a valid context' "$log"; then
          hypr_app_glutin_kind=bad_context
        elif /bin/busybox grep -Fqi 'arguments are inconsistent' "$log"; then
          hypr_app_glutin_kind=bad_match
        elif /bin/busybox grep -Fqi 'initialization failed' "$log"; then
          hypr_app_glutin_kind=initialization_failed
        elif /bin/busybox grep -Fqi 'access to the resource failed' "$log"; then
          hypr_app_glutin_kind=bad_access
        elif /bin/busybox grep -Fqi 'out of memory' "$log"; then
          hypr_app_glutin_kind=out_of_memory
        elif /bin/busybox grep -Fqi 'one or more argument values are invalid' "$log"; then
          hypr_app_glutin_kind=bad_parameter
        elif /bin/busybox grep -Fqi 'the context is in a bad state' "$log"; then
          hypr_app_glutin_kind=bad_context_state
        elif /bin/busybox grep -Fqi 'the current surface of the calling thread is no longer valid' "$log"; then
          hypr_app_glutin_kind=bad_current_surface
        elif /bin/busybox grep -Fqi 'context loss' "$log"; then
          hypr_app_glutin_kind=context_lost
        elif /bin/busybox grep -Fqi 'misc platform error' "$log"; then
          hypr_app_glutin_kind=misc
        elif /bin/busybox grep -Fqi 'glutin error:' "$log"; then
          hypr_app_glutin_kind=other_glutin_error
        elif /bin/busybox grep -Fqi 'winit error:' "$log"; then
          hypr_app_glutin_kind=winit_error
        elif /bin/busybox grep -Fqi 'app creation error:' "$log"; then
          hypr_app_glutin_kind=app_creation_error
        fi
        if [ "$hypr_app_glutin_kind" != unclassified ]; then break; fi
      done
      # Only fixed Mesa failure categories may leave private guest RAM.
      hypr_app_dri2_reason=none
      hypr_app_egl_debug=0
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'EGL debug|EGL warning|libEGL|DRI2:' "$log"; then hypr_app_egl_debug=1; fi
        if /bin/busybox grep -Eiq 'DRI2: failed to load driver|failed to load driver: kms_swrast|kms_swrast.*failed to load' "$log"; then
          hypr_app_dri2_reason=load_driver
        elif /bin/busybox grep -Eiq 'DRI2: failed to create screen|failed to create dri screen' "$log"; then
          hypr_app_dri2_reason=create_screen
        elif /bin/busybox grep -Eiq 'failed to setup EGLDevice|failed to set up EGLDevice' "$log"; then
          hypr_app_dri2_reason=setup_device
        elif /bin/busybox grep -Eiq 'failed to find required DRI extensions' "$log"; then
          hypr_app_dri2_reason=missing_extensions
        elif /bin/busybox grep -Eiq 'display does not support prime' "$log"; then
          hypr_app_dri2_reason=prime_unsupported
        elif /bin/busybox grep -Eiq 'failed to add configs|failed to.*EGL.*config' "$log"; then
          hypr_app_dri2_reason=configs
        elif /bin/busybox grep -Eiq 'failed to authenti(cate|cation)|failed to open.*render' "$log"; then
          hypr_app_dri2_reason=drm_access
        elif /bin/busybox grep -Eiq 'failed to load.*swrast_dri|swrast_dri.so.*not found' "$log"; then
          hypr_app_dri2_reason=missing_dri_library
        fi
        if [ "$hypr_app_dri2_reason" != none ]; then break; fi
      done
      echo "STARBYTE_GUEST_HYPRLAND_APP_DIAG stdout=$hypr_app_stdout stderr=$hypr_app_stderr egl=$hypr_app_egl dri=$hypr_app_dri context=$hypr_app_context wayland=$hypr_app_wayland winit=$hypr_app_winit panic=$hypr_app_panic library=$hypr_app_library config=$hypr_app_config render=$hypr_app_render permission=$hypr_app_permission protocol=$hypr_app_protocol stack_eframe=$hypr_app_stack_eframe stack_glutin=$hypr_app_stack_glutin stack_winit=$hypr_app_stack_winit stack_starbyte=$hypr_app_stack_starbyte pixel_format=$hypr_app_pixel_format surface=$hypr_app_surface missing_global=$hypr_app_missing_global egl_bad_surface=$hypr_app_egl_bad_surface egl_bad_native_window=$hypr_app_egl_bad_native_window egl_create_window_surface=$hypr_app_egl_create_window_surface surface_init_failure=$hypr_app_surface_init_failure xdg_configure=$hypr_app_xdg_configure dmabuf_failed=$hypr_app_dmabuf_failed dmabuf_params=$hypr_app_dmabuf_params wl_surface_attach=$hypr_app_wl_surface_attach wl_drm=$hypr_app_wl_drm registry_requested=$hypr_app_registry_requested create_surface=$hypr_app_create_surface create_xdg=$hypr_app_create_xdg create_toplevel=$hypr_app_create_toplevel initial_commit=$hypr_app_initial_commit display_error=$hypr_app_display_error egl_bad_alloc=$hypr_app_egl_bad_alloc egl_bad_match=$hypr_app_egl_bad_match egl_not_initialized=$hypr_app_egl_not_initialized glutin_error=$hypr_app_glutin_error surface_error=$hypr_app_surface_error backtrace=$hypr_app_backtrace panic_line=$hypr_app_panic_line glutin_kind=$hypr_app_glutin_kind mesa_driver=swrast egl_debug=$hypr_app_egl_debug dri2_reason=$hypr_app_dri2_reason" >/dev/console
      # kill -0 alone also succeeds for zombies; require a non-zombie
      # Hyprland process after the application has finished.
      hypr_alive=0
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null && [ -r "/proc/$hypr_pid/stat" ]; then
        hypr_state="$(/bin/busybox awk '{print $3}' "/proc/$hypr_pid/stat")"
        if [ "$hypr_state" != Z ]; then hypr_alive=1; fi
      fi
      echo "STARBYTE_GUEST_HYPRLAND_APP_RESULT status=$hypr_app_status frames=$hypr_app_frames single_owner=$hypr_app_single_owner" >/dev/console
      /bin/busybox kill "$hypr_pid" 2>/dev/null || :
      wait "$hypr_pid" 2>/dev/null || :
      echo "STARBYTE_GUEST_HYPRLAND_DIAG version=$hypr_version config_valid=$hypr_config_valid egl=$hypr_egl_error gbm=$hypr_gbm_error seat_error=$hypr_seat_error config_error=$hypr_config_error library_error=$hypr_library_error root_guard=$hypr_root_guard wire_attempts=$hypr_wire_attempts wire_connected=$hypr_wire_connected wire_sync=$hypr_wire_sync wire_reason=$hypr_wire_reason monitor_error=$hypr_monitor_error" >/dev/console
      echo "STARBYTE_GUEST_HYPRLAND_RESULT executable=$hypr_executable socket=$hypr_socket registry=$hypr_registry compositor=$hypr_compositor seat=$hypr_seat alive=$hypr_alive" >/dev/console
    fi
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
if [[ "$mode" == --gbm-smoke ]]; then guest_ram=768M; fi
if [[ "$mode" == --wayland-smoke ]]; then guest_ram=1536M; fi
if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then guest_ram=2048M; fi
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
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Entirely virtual keyboard, private guest-only QMP socket.
  qmp_args=(-qmp "unix:$root/qmp.sock,server=on,wait=off")
  guest_focus_injector() {
    local ready=false
    # VM startup under TCG can exceed one minute on hosted runners.
    # Remain bounded by the outer 200-second guest-QEMU deadline.
    for _ in {1..2100}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]] \
        && grep -Fq 'STARBYTE_GUEST_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
        ready=true
        break
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    local qmp_status=0
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_focus_injector &
  injector_pid=$!
  trap 'kill "$injector_pid" 2>/dev/null || true; wait "$injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
if [[ "$mode" == --hyprland-smoke ]]; then
  # A second QMP connection after the Weston Escape test has completed.
  # The acknowledgement is not enough: the focused client must exit.
  guest_hypr_focus_injector() {
    local ready=false qmp_status=0
    for _ in {1..2900}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]]; then
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
          ready=true
          break
        fi
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ' "$root/serial.log"; then
          echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_never_ready'
          return 5
        fi
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/hypr-focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/hypr-focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_hypr_focus_injector &
  hypr_injector_pid=$!
  trap 'kill "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; wait "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
display_args=(-display none)
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Private Unix-only VNC display emulation advertises a KMS output.
  # No TCP listener, physical display, viewer, or host desktop socket.
  display_args=(-display "vnc=unix:$root/guest-vnc.sock")
fi
guest_timeout=125
if [[ "$mode" == --wayland-smoke ]]; then guest_timeout=200; fi
if [[ "$mode" == --starbyte-smoke ]]; then guest_timeout=340; fi
if [[ "$mode" == --hyprland-smoke ]]; then guest_timeout=450; fi
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
focus_injector_ok=true
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  wait "$injector_pid" || focus_injector_ok=false
  injector_pid=''
fi
hypr_injector_ok=true
if [[ "$mode" == --hyprland-smoke ]]; then
  wait "$hypr_injector_pid" || hypr_injector_ok=false
  hypr_injector_pid=''
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
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
    # All receipts are bounded diagnostics; never disclose raw guest logs.
    grep -E '^STARBYTE_GUEST_(WAYLAND_(RESULT|DIAG|WIRE_DIAG|WIRE_SELFTEST|BIN_DIAG|REGISTRY_DIAG)|WESTON_(EXEC|STAGE|EXIT)|TMP_WRITE_OK|TMP_MOUNT|TMP_IO_DIAG|RUN_MOUNT|UDEV_RESULT|XKB_DATA|FOCUS_RESULT|APP_RESULT|APP_DIAG|APP_ROOT_DIAG) ' "$root/serial-clean.log" || true
    grep -Fqx 'STARBYTE_GUEST_TMP_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_RUN_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_run_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_UDEV_RESULT daemon=0 trigger=0 settle=0 event=1 keyboard=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_udev_classification_missing'
    grep -Fqx 'STARBYTE_GUEST_XKB_DATA rules=1 symbols=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_xkb_data_unavailable'
    grep -Fqx 'STARBYTE_GUEST_TMP_IO_DIAG write_status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_write_failed'
    grep -Fqx 'STARBYTE_GUEST_TMP_WRITE_OK result=1' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_io_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST status=0 output=1 stderr=0 reason=none' "$root/serial-clean.log" \
      || fail 'guest_wayland_wire_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WESTON_EXEC status=0 stdout=1 stderr=0' "$root/serial-clean.log" \
      || fail 'guest_weston_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_RESULT socket=1 handshake=1 compositor=1 seat=1' "$root/serial-clean.log" \
      || fail 'guest_drm_wayland_handshake_or_seat_missing'
    grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=1 sync=1 output=1 attempts=[1-8] status=0 stderr=0 reason=none$' "$root/serial-clean.log" \
      || fail 'guest_wayland_registry_sync_missing'
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=rust_wire seat=present guest_only=true'
    [[ "$focus_injector_ok" == true ]] || fail 'guest_private_focus_qmp_injection_failed'
    grep -Fqx 'STARBYTE_GUEST_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 weston_alive=1' "$root/serial-clean.log" \
      || fail 'guest_focused_wayland_keyboard_delivery_missing'
    echo 'STARBYTE_GUEST_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    if [[ "$mode" == --hyprland-smoke ]]; then
      grep -E '^STARBYTE_GUEST_(HYPRLAND_(RESULT|DIAG|APP_RESULT|APP_DIAG)|HYPR_FOCUS_(RESULT|DIAG)|HYPR_OUTPUT_DIAG|HYPR_KMS_DIAG|HYPR_EFRAME_DIAG|HYPR_CAP_(BLOCKED|DIAG)|WAYLAND_CAP_RESULT|EGLINFO_DIAG|EGL_ATTACH_DIAG|EGL_ATTACHED_RESULT|EGL_TRACKED_RESULT|EGL_NONE_RESULT|HYPR_XDG_DIAG) ' "$root/serial-clean.log" || true
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_RESULT executable=1 socket=1 registry=1 compositor=1 seat=1 alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_registry_or_seat_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_OK drm=virtio compositor=hyprland seat=virtual guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_APP_RESULT status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_hyprland_window_and_worker_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 hyprland_alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_focused_keyboard_missing'
      [[ "$hypr_injector_ok" == true ]] || fail 'guest_private_hyprland_focus_injector_failed'
      echo 'STARBYTE_GUEST_HYPR_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    fi
    if [[ "$mode" == --starbyte-smoke ]]; then
      grep -Fqx 'STARBYTE_GUEST_APP_RESULT launched=1 status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_window_and_worker_missing'
      echo 'STARBYTE_GUEST_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
    fi
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
 /tmp/hypr-xdg-probe.log >/dev/console || :
      echo "STARBYTE_GUEST_HYPR_XDG_DIAG status=$hypr_xdg_status configured=$hypr_xdg_configured reason=$hypr_xdg_reason" >/dev/console
      # Distinct from the Weston sample: prove focus under Hyprland itself.
      hypr_focus_ready=0
      hypr_focus_clean=0
      hypr_focus_message=0
      hypr_focus_compositor_alive=0
      hypr_focus_status=not_run
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          /usr/bin/weston-simple-shm >/tmp/hypr-focus.log 2>/tmp/hypr-focus.err &
        hypr_focus_pid=$!
        # Only report readiness after wl_keyboard.enter, rather than a
        # fixed three-second liveness check on an unfocused window.
        for focus_probe in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
          if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' /tmp/hypr-focus.err 2>/dev/null &&
             /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
            hypr_focus_ready=1
            echo 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' >/dev/console
            break
          fi
          if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
        if [ "$hypr_focus_ready" = 1 ]; then
          for focus_wait in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
            if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err; then break; fi
            if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
            /bin/busybox sleep 1
          done
        fi
        if /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
          /bin/busybox kill "$hypr_focus_pid" 2>/dev/null || :
        fi
        wait "$hypr_focus_pid" 2>/dev/null
        hypr_focus_status=$?
        if [ "$hypr_focus_status" = 0 ]; then hypr_focus_clean=1; fi
        if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err 2>/dev/null; then hypr_focus_message=1; fi
        if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_focus_compositor_alive=1; fi
      fi
      hypr_focus_stdout=0
      hypr_focus_stderr=0
      hypr_focus_connect=0
      hypr_focus_library=0
      hypr_focus_protocol=0
      hypr_focus_xdg=0
      hypr_focus_shm=0
      hypr_focus_surface=0
      hypr_focus_keyboard_enter=0
      hypr_focus_keyboard_requested=0
      hypr_focus_xdg_configure=0
      hypr_focus_shm_advertised=0
      hypr_focus_registry_requested=0
      hypr_focus_create_surface=0
      hypr_focus_create_xdg=0
      hypr_focus_create_toplevel=0
      hypr_focus_initial_commit=0
      hypr_focus_display_error=0
      hypr_focus_toplevel_close=0
      if [ -s /tmp/hypr-focus.log ]; then hypr_focus_stdout=1; fi
      if [ -s /tmp/hypr-focus.err ]; then hypr_focus_stderr=1; fi
      for log in /tmp/hypr-focus.log /tmp/hypr-focus.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'failed to connect|connection refused|cannot connect|No such file.*wayland' "$log"; then hypr_focus_connect=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_focus_library=1; fi
        if /bin/busybox grep -Eiq 'protocol error|wl_display.*error|wayland.*error|display.*disconnected' "$log"; then hypr_focus_protocol=1; fi
        if /bin/busybox grep -Eiq 'xdg_wm_base|xdg_surface|xdg_toplevel|xdg-shell' "$log"; then hypr_focus_xdg=1; fi
        if /bin/busybox grep -Eiq 'wl_shm|shared memory|shm.*failed' "$log"; then hypr_focus_shm=1; fi
        if /bin/busybox grep -Eiq 'failed to.*(surface|window)|surface.*failed|failed.*buffer' "$log"; then hypr_focus_surface=1; fi
        if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' "$log"; then hypr_focus_keyboard_enter=1; fi
        if /bin/busybox grep -Eq 'wl_seat@[0-9]+\.get_keyboard\(' "$log"; then hypr_focus_keyboard_requested=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_focus_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'wl_registry@[0-9]+\.global.*wl_shm' "$log"; then hypr_focus_shm_advertised=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_focus_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_focus_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_focus_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_focus_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_focus_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_focus_display_error=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.close\(' "$log"; then hypr_focus_toplevel_close=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_FOCUS_DIAG status=$hypr_focus_status stdout=$hypr_focus_stdout stderr=$hypr_focus_stderr connect=$hypr_focus_connect library=$hypr_focus_library protocol=$hypr_focus_protocol xdg=$hypr_focus_xdg shm=$hypr_focus_shm surface=$hypr_focus_surface keyboard_enter=$hypr_focus_keyboard_enter keyboard_requested=$hypr_focus_keyboard_requested xdg_configure=$hypr_focus_xdg_configure shm_advertised=$hypr_focus_shm_advertised registry_requested=$hypr_focus_registry_requested create_surface=$hypr_focus_create_surface create_xdg=$hypr_focus_create_xdg create_toplevel=$hypr_focus_create_toplevel initial_commit=$hypr_focus_initial_commit display_error=$hypr_focus_display_error toplevel_close=$hypr_focus_toplevel_close" >/dev/console
      echo "STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=$hypr_focus_ready clean_exit=$hypr_focus_clean exit_message=$hypr_focus_message hyprland_alive=$hypr_focus_compositor_alive" >/dev/console
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_alive=1; fi
      for log in /tmp/hyprland.log /tmp/hyprland.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|egl_not_initialized' "$log"; then hypr_egl_error=1; fi
        if /bin/busybox grep -Eiq 'gbm.*failed|failed.*gbm|no allocator' "$log"; then hypr_gbm_error=1; fi
        if /bin/busybox grep -Eiq 'libseat.*failed|seat.*failed|session inactive' "$log"; then hypr_seat_error=1; fi
        if /bin/busybox grep -Eiq 'config.*failed|config.*error|parse.*error' "$log"; then hypr_config_error=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|failed.*shader' "$log"; then hypr_library_error=1; fi
        if /bin/busybox grep -Eiq 'no monitor|no output|no suitable mode|failed to commit.*output|create.*head.*failed|mode.*not available|output.*disabled' "$log"; then hypr_monitor_error=1; fi
        if /bin/busybox grep -Eiq 'superuser privileges|shall not be run as the root|privilege check is not omitted' "$log"; then hypr_root_guard=1; fi
      done
      # Reuse the original full-library GUI smoke with Hyprland as the
      # Wayland server, never the owner's display. One exclusive worker
      # must execute 30 demo frames without a shadow legacy session.
      hypr_app_status=not_run
      hypr_app_frames=0
      hypr_app_single_owner=0
      if [ "$hypr_registry" = 1 ] && [ "$hypr_seat" = 1 ]; then
        # The private Hyprland test compared swrast with kms_swrast:
        # both returned EGL_NOT_INITIALIZED. Retain the Weston-proven
        # swrast client setting; no accepted guest GUI cutover is implied.
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          RUST_BACKTRACE=1 \
          STARBYTE_LOG=starbyte=info,eframe=trace,glutin=trace,winit=trace \
          EGL_LOG_LEVEL=debug LIBGL_DEBUG=verbose \
          WINIT_UNIX_BACKEND=wayland \
          EGL_PLATFORM=wayland \
          MESA_LOADER_DRIVER_OVERRIDE=swrast \
          LIBGL_ALWAYS_SOFTWARE=1 \
          GALLIUM_DRIVER=llvmpipe \
          XDG_CONFIG_HOME=/tmp/starbyte-hyprland-app-config \
          XDG_CACHE_HOME=/tmp/starbyte-hyprland-app-cache \
          /bin/starbyte-egui --demo --native-worker-preview --exit-after-frames 30 \
            --config /tmp/starbyte-hyprland-app-config.toml \
            --cache-dir /tmp/starbyte-hyprland-cache \
            >/tmp/starbyte-hyprland-app.log 2>/tmp/starbyte-hyprland-app.err
        hypr_app_status=$?
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_frames=1; fi
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_single_owner=1; fi
      fi
      hypr_app_stdout=0
      hypr_app_stderr=0
      hypr_app_egl=0
      hypr_app_dri=0
      hypr_app_context=0
      hypr_app_wayland=0
      hypr_app_winit=0
      hypr_app_panic=0
      hypr_app_library=0
      hypr_app_config=0
      hypr_app_render=0
      hypr_app_permission=0
      hypr_app_protocol=0
      hypr_app_stack_eframe=0
      hypr_app_stack_glutin=0
      hypr_app_stack_winit=0
      hypr_app_stack_starbyte=0
      hypr_app_pixel_format=0
      hypr_app_surface=0
      hypr_app_missing_global=0
      hypr_app_egl_bad_surface=0
      hypr_app_egl_bad_native_window=0
      hypr_app_egl_create_window_surface=0
      hypr_app_surface_init_failure=0
      hypr_app_xdg_configure=0
      hypr_app_dmabuf_failed=0
      hypr_app_dmabuf_params=0
      hypr_app_wl_surface_attach=0
      hypr_app_wl_drm=0
      hypr_app_registry_requested=0
      hypr_app_create_surface=0
      hypr_app_create_xdg=0
      hypr_app_create_toplevel=0
      hypr_app_initial_commit=0
      hypr_app_display_error=0
      hypr_app_egl_bad_alloc=0
      hypr_app_egl_bad_match=0
      hypr_app_egl_not_initialized=0
      hypr_app_glutin_error=0
      hypr_app_surface_error=0
      hypr_app_backtrace=0
      hypr_app_panic_line=0
      if [ -s /tmp/starbyte-hyprland-app.log ]; then hypr_app_stdout=1; fi
      if [ -s /tmp/starbyte-hyprland-app.err ]; then hypr_app_stderr=1; fi
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|EGL_NOT_INITIALIZED|EGL_BAD_DISPLAY|eglInitialize' "$log"; then hypr_app_egl=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*dri|swrast_dri.so|MESA-LOADER' "$log"; then hypr_app_dri=1; fi
        if /bin/busybox grep -Eiq 'NoAvailablePixelFormat|failed to create.*context|no matching config|eglCreateContext|eglMakeCurrent' "$log"; then hypr_app_context=1; fi
        if /bin/busybox grep -Eiq 'wayland.*connection|failed to connect.*wayland|WaylandError|Wayland backend' "$log"; then hypr_app_wayland=1; fi
        if /bin/busybox grep -Eiq 'EventLoopError|failed to create event loop|winit::|event loop.*failed' "$log"; then hypr_app_winit=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_app_library=1; fi
        if /bin/busybox grep -Eiq 'config.*error|config.*failed|permission denied.*config|read.only|no space left' "$log"; then hypr_app_config=1; fi
        if /bin/busybox grep -Eiq 'glutin|OpenGL|surface.*error|renderer.*failed|failed.*renderer' "$log"; then hypr_app_render=1; fi
        if /bin/busybox grep -Eiq 'Permission denied|Operation not permitted' "$log"; then hypr_app_permission=1; fi
        if /bin/busybox grep -Eiq 'protocol error|xdg_wm_base|xdg_surface|xdg_toplevel|wl_display.*error' "$log"; then hypr_app_protocol=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*eframe::' "$log"; then hypr_app_stack_eframe=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*glutin::' "$log"; then hypr_app_stack_glutin=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*winit::' "$log"; then hypr_app_stack_winit=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*starbyte' "$log"; then hypr_app_stack_starbyte=1; fi
        if /bin/busybox grep -Eiq 'pixel.*format|no matching.*config|no EGL.*config|failed to find.*config' "$log"; then hypr_app_pixel_format=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_SURFACE|EGL_BAD_NATIVE_WINDOW|eglCreateWindowSurface|surface.*init.*fail' "$log"; then hypr_app_surface=1; fi
        if /bin/busybox grep -Eiq 'missing.*(global|extension)|does not support.*(protocol|interface)|required.*(global|extension)' "$log"; then hypr_app_missing_global=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_SURFACE' "$log"; then hypr_app_egl_bad_surface=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_NATIVE_WINDOW' "$log"; then hypr_app_egl_bad_native_window=1; fi
        if /bin/busybox grep -Fq 'eglCreateWindowSurface' "$log"; then hypr_app_egl_create_window_surface=1; fi
        if /bin/busybox grep -Eiq 'surface.*init.*fail' "$log"; then hypr_app_surface_init_failure=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_app_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.failed\(' "$log"; then hypr_app_dmabuf_failed=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.' "$log"; then hypr_app_dmabuf_params=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.attach\(' "$log"; then hypr_app_wl_surface_attach=1; fi
        if /bin/busybox grep -Fq 'wl_drm' "$log"; then hypr_app_wl_drm=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_app_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_app_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_app_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_app_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_app_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_app_display_error=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_ALLOC' "$log"; then hypr_app_egl_bad_alloc=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_MATCH' "$log"; then hypr_app_egl_bad_match=1; fi
        if /bin/busybox grep -Fq 'EGL_NOT_INITIALIZED' "$log"; then hypr_app_egl_not_initialized=1; fi
        if /bin/busybox grep -Eiq '(glutin|GL surface|GlSurface|gl_surface).*(error|fail)|Glutin\(' "$log"; then hypr_app_glutin_error=1; fi
        if /bin/busybox grep -Eiq '(surface|window).*(creation|initialization|configure).*(fail|error)|failed to initialize a window' "$log"; then hypr_app_surface_error=1; fi
        if /bin/busybox grep -Fq 'stack backtrace:' "$log"; then hypr_app_backtrace=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic_line=1; fi
      done
      # Map eframe 0.32's own GL launch messages to fixed stages. The
      # original logs stay private; these flags do not weaken acceptance.
      hypr_eframe_display_start=0
      hypr_eframe_display_ready=0
      hypr_eframe_context_start=0
      hypr_eframe_surface_start=0
      hypr_eframe_surface_ready=0
      hypr_eframe_context_current=0
      hypr_eframe_window_failure=0
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fq 'trying to create glutin Display with config:' "$log"; then hypr_eframe_display_start=1; fi
        if /bin/busybox grep -Fq 'successfully created GL Display with version' "$log"; then hypr_eframe_display_ready=1; fi
        if /bin/busybox grep -Fq 'creating gl context using raw window handle:' "$log"; then hypr_eframe_context_start=1; fi
        if /bin/busybox grep -Fq 'Creating a gl_surface for viewport' "$log"; then hypr_eframe_surface_start=1; fi
        if /bin/busybox grep -Fq 'surface created successfully:' "$log"; then hypr_eframe_surface_ready=1; fi
        if /bin/busybox grep -Fq 'made context current. setting swap interval' "$log"; then hypr_eframe_context_current=1; fi
        if /bin/busybox grep -Fq 'Failed to initialize a window for viewport' "$log"; then hypr_eframe_window_failure=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_EFRAME_DIAG display_start=$hypr_eframe_display_start display_ready=$hypr_eframe_display_ready context_start=$hypr_eframe_context_start surface_start=$hypr_eframe_surface_start surface_ready=$hypr_eframe_surface_ready context_current=$hypr_eframe_context_current window_failure=$hypr_eframe_window_failure" >/dev/console
      # Match glutin 0.32 ErrorKind's fixed public display strings.
      # Never print raw native errors, Wayland traces, paths or guest logs.
      hypr_app_glutin_kind=unclassified
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fqi 'argument does not refer to a valid native window' "$log"; then
          hypr_app_glutin_kind=bad_native_window
        elif /bin/busybox grep -Fqi 'argument does not name a valid surface' "$log"; then
          hypr_app_glutin_kind=bad_surface
        elif /bin/busybox grep -Fqi 'argument does not name a valid display' "$log"; then
          hypr_app_glutin_kind=bad_display
        elif /bin/busybox grep -Fqi 'argument does not name a valid config' "$log"; then
          hypr_app_glutin_kind=bad_config
        elif /bin/busybox grep -Fqi 'argument does not name a valid context' "$log"; then
          hypr_app_glutin_kind=bad_context
        elif /bin/busybox grep -Fqi 'arguments are inconsistent' "$log"; then
          hypr_app_glutin_kind=bad_match
        elif /bin/busybox grep -Fqi 'initialization failed' "$log"; then
          hypr_app_glutin_kind=initialization_failed
        elif /bin/busybox grep -Fqi 'access to the resource failed' "$log"; then
          hypr_app_glutin_kind=bad_access
        elif /bin/busybox grep -Fqi 'out of memory' "$log"; then
          hypr_app_glutin_kind=out_of_memory
        elif /bin/busybox grep -Fqi 'one or more argument values are invalid' "$log"; then
          hypr_app_glutin_kind=bad_parameter
        elif /bin/busybox grep -Fqi 'the context is in a bad state' "$log"; then
          hypr_app_glutin_kind=bad_context_state
        elif /bin/busybox grep -Fqi 'the current surface of the calling thread is no longer valid' "$log"; then
          hypr_app_glutin_kind=bad_current_surface
        elif /bin/busybox grep -Fqi 'context loss' "$log"; then
          hypr_app_glutin_kind=context_lost
        elif /bin/busybox grep -Fqi 'misc platform error' "$log"; then
          hypr_app_glutin_kind=misc
        elif /bin/busybox grep -Fqi 'glutin error:' "$log"; then
          hypr_app_glutin_kind=other_glutin_error
        elif /bin/busybox grep -Fqi 'winit error:' "$log"; then
          hypr_app_glutin_kind=winit_error
        elif /bin/busybox grep -Fqi 'app creation error:' "$log"; then
          hypr_app_glutin_kind=app_creation_error
        fi
        if [ "$hypr_app_glutin_kind" != unclassified ]; then break; fi
      done
      # Only fixed Mesa failure categories may leave private guest RAM.
      hypr_app_dri2_reason=none
      hypr_app_egl_debug=0
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'EGL debug|EGL warning|libEGL|DRI2:' "$log"; then hypr_app_egl_debug=1; fi
        if /bin/busybox grep -Eiq 'DRI2: failed to load driver|failed to load driver: kms_swrast|kms_swrast.*failed to load' "$log"; then
          hypr_app_dri2_reason=load_driver
        elif /bin/busybox grep -Eiq 'DRI2: failed to create screen|failed to create dri screen' "$log"; then
          hypr_app_dri2_reason=create_screen
        elif /bin/busybox grep -Eiq 'failed to setup EGLDevice|failed to set up EGLDevice' "$log"; then
          hypr_app_dri2_reason=setup_device
        elif /bin/busybox grep -Eiq 'failed to find required DRI extensions' "$log"; then
          hypr_app_dri2_reason=missing_extensions
        elif /bin/busybox grep -Eiq 'display does not support prime' "$log"; then
          hypr_app_dri2_reason=prime_unsupported
        elif /bin/busybox grep -Eiq 'failed to add configs|failed to.*EGL.*config' "$log"; then
          hypr_app_dri2_reason=configs
        elif /bin/busybox grep -Eiq 'failed to authenti(cate|cation)|failed to open.*render' "$log"; then
          hypr_app_dri2_reason=drm_access
        elif /bin/busybox grep -Eiq 'failed to load.*swrast_dri|swrast_dri.so.*not found' "$log"; then
          hypr_app_dri2_reason=missing_dri_library
        fi
        if [ "$hypr_app_dri2_reason" != none ]; then break; fi
      done
      echo "STARBYTE_GUEST_HYPRLAND_APP_DIAG stdout=$hypr_app_stdout stderr=$hypr_app_stderr egl=$hypr_app_egl dri=$hypr_app_dri context=$hypr_app_context wayland=$hypr_app_wayland winit=$hypr_app_winit panic=$hypr_app_panic library=$hypr_app_library config=$hypr_app_config render=$hypr_app_render permission=$hypr_app_permission protocol=$hypr_app_protocol stack_eframe=$hypr_app_stack_eframe stack_glutin=$hypr_app_stack_glutin stack_winit=$hypr_app_stack_winit stack_starbyte=$hypr_app_stack_starbyte pixel_format=$hypr_app_pixel_format surface=$hypr_app_surface missing_global=$hypr_app_missing_global egl_bad_surface=$hypr_app_egl_bad_surface egl_bad_native_window=$hypr_app_egl_bad_native_window egl_create_window_surface=$hypr_app_egl_create_window_surface surface_init_failure=$hypr_app_surface_init_failure xdg_configure=$hypr_app_xdg_configure dmabuf_failed=$hypr_app_dmabuf_failed dmabuf_params=$hypr_app_dmabuf_params wl_surface_attach=$hypr_app_wl_surface_attach wl_drm=$hypr_app_wl_drm registry_requested=$hypr_app_registry_requested create_surface=$hypr_app_create_surface create_xdg=$hypr_app_create_xdg create_toplevel=$hypr_app_create_toplevel initial_commit=$hypr_app_initial_commit display_error=$hypr_app_display_error egl_bad_alloc=$hypr_app_egl_bad_alloc egl_bad_match=$hypr_app_egl_bad_match egl_not_initialized=$hypr_app_egl_not_initialized glutin_error=$hypr_app_glutin_error surface_error=$hypr_app_surface_error backtrace=$hypr_app_backtrace panic_line=$hypr_app_panic_line glutin_kind=$hypr_app_glutin_kind mesa_driver=swrast egl_debug=$hypr_app_egl_debug dri2_reason=$hypr_app_dri2_reason" >/dev/console
      # kill -0 alone also succeeds for zombies; require a non-zombie
      # Hyprland process after the application has finished.
      hypr_alive=0
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null && [ -r "/proc/$hypr_pid/stat" ]; then
        hypr_state="$(/bin/busybox awk '{print $3}' "/proc/$hypr_pid/stat")"
        if [ "$hypr_state" != Z ]; then hypr_alive=1; fi
      fi
      echo "STARBYTE_GUEST_HYPRLAND_APP_RESULT status=$hypr_app_status frames=$hypr_app_frames single_owner=$hypr_app_single_owner" >/dev/console
      /bin/busybox kill "$hypr_pid" 2>/dev/null || :
      wait "$hypr_pid" 2>/dev/null || :
      echo "STARBYTE_GUEST_HYPRLAND_DIAG version=$hypr_version config_valid=$hypr_config_valid egl=$hypr_egl_error gbm=$hypr_gbm_error seat_error=$hypr_seat_error config_error=$hypr_config_error library_error=$hypr_library_error root_guard=$hypr_root_guard wire_attempts=$hypr_wire_attempts wire_connected=$hypr_wire_connected wire_sync=$hypr_wire_sync wire_reason=$hypr_wire_reason monitor_error=$hypr_monitor_error" >/dev/console
      echo "STARBYTE_GUEST_HYPRLAND_RESULT executable=$hypr_executable socket=$hypr_socket registry=$hypr_registry compositor=$hypr_compositor seat=$hypr_seat alive=$hypr_alive" >/dev/console
    fi
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
if [[ "$mode" == --gbm-smoke ]]; then guest_ram=768M; fi
if [[ "$mode" == --wayland-smoke ]]; then guest_ram=1536M; fi
if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then guest_ram=2048M; fi
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
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Entirely virtual keyboard, private guest-only QMP socket.
  qmp_args=(-qmp "unix:$root/qmp.sock,server=on,wait=off")
  guest_focus_injector() {
    local ready=false
    # VM startup under TCG can exceed one minute on hosted runners.
    # Remain bounded by the outer 200-second guest-QEMU deadline.
    for _ in {1..2100}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]] \
        && grep -Fq 'STARBYTE_GUEST_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
        ready=true
        break
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    local qmp_status=0
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_focus_injector &
  injector_pid=$!
  trap 'kill "$injector_pid" 2>/dev/null || true; wait "$injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
if [[ "$mode" == --hyprland-smoke ]]; then
  # A second QMP connection after the Weston Escape test has completed.
  # The acknowledgement is not enough: the focused client must exit.
  guest_hypr_focus_injector() {
    local ready=false qmp_status=0
    for _ in {1..2900}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]]; then
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
          ready=true
          break
        fi
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ' "$root/serial.log"; then
          echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_never_ready'
          return 5
        fi
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/hypr-focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/hypr-focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_hypr_focus_injector &
  hypr_injector_pid=$!
  trap 'kill "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; wait "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
display_args=(-display none)
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Private Unix-only VNC display emulation advertises a KMS output.
  # No TCP listener, physical display, viewer, or host desktop socket.
  display_args=(-display "vnc=unix:$root/guest-vnc.sock")
fi
guest_timeout=125
if [[ "$mode" == --wayland-smoke ]]; then guest_timeout=200; fi
if [[ "$mode" == --starbyte-smoke ]]; then guest_timeout=340; fi
if [[ "$mode" == --hyprland-smoke ]]; then guest_timeout=450; fi
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
focus_injector_ok=true
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  wait "$injector_pid" || focus_injector_ok=false
  injector_pid=''
fi
hypr_injector_ok=true
if [[ "$mode" == --hyprland-smoke ]]; then
  wait "$hypr_injector_pid" || hypr_injector_ok=false
  hypr_injector_pid=''
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
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
    # All receipts are bounded diagnostics; never disclose raw guest logs.
    grep -E '^STARBYTE_GUEST_(WAYLAND_(RESULT|DIAG|WIRE_DIAG|WIRE_SELFTEST|BIN_DIAG|REGISTRY_DIAG)|WESTON_(EXEC|STAGE|EXIT)|TMP_WRITE_OK|TMP_MOUNT|TMP_IO_DIAG|RUN_MOUNT|UDEV_RESULT|XKB_DATA|FOCUS_RESULT|APP_RESULT|APP_DIAG|APP_ROOT_DIAG) ' "$root/serial-clean.log" || true
    grep -Fqx 'STARBYTE_GUEST_TMP_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_RUN_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_run_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_UDEV_RESULT daemon=0 trigger=0 settle=0 event=1 keyboard=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_udev_classification_missing'
    grep -Fqx 'STARBYTE_GUEST_XKB_DATA rules=1 symbols=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_xkb_data_unavailable'
    grep -Fqx 'STARBYTE_GUEST_TMP_IO_DIAG write_status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_write_failed'
    grep -Fqx 'STARBYTE_GUEST_TMP_WRITE_OK result=1' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_io_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST status=0 output=1 stderr=0 reason=none' "$root/serial-clean.log" \
      || fail 'guest_wayland_wire_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WESTON_EXEC status=0 stdout=1 stderr=0' "$root/serial-clean.log" \
      || fail 'guest_weston_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_RESULT socket=1 handshake=1 compositor=1 seat=1' "$root/serial-clean.log" \
      || fail 'guest_drm_wayland_handshake_or_seat_missing'
    grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=1 sync=1 output=1 attempts=[1-8] status=0 stderr=0 reason=none$' "$root/serial-clean.log" \
      || fail 'guest_wayland_registry_sync_missing'
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=rust_wire seat=present guest_only=true'
    [[ "$focus_injector_ok" == true ]] || fail 'guest_private_focus_qmp_injection_failed'
    grep -Fqx 'STARBYTE_GUEST_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 weston_alive=1' "$root/serial-clean.log" \
      || fail 'guest_focused_wayland_keyboard_delivery_missing'
    echo 'STARBYTE_GUEST_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    if [[ "$mode" == --hyprland-smoke ]]; then
      grep -E '^STARBYTE_GUEST_(HYPRLAND_(RESULT|DIAG|APP_RESULT|APP_DIAG)|HYPR_FOCUS_(RESULT|DIAG)|HYPR_OUTPUT_DIAG|HYPR_KMS_DIAG|HYPR_EFRAME_DIAG|HYPR_CAP_(BLOCKED|DIAG)|WAYLAND_CAP_RESULT|EGLINFO_DIAG|EGL_ATTACH_DIAG|EGL_ATTACHED_RESULT|EGL_TRACKED_RESULT|EGL_NONE_RESULT|HYPR_XDG_DIAG) ' "$root/serial-clean.log" || true
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_RESULT executable=1 socket=1 registry=1 compositor=1 seat=1 alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_registry_or_seat_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_OK drm=virtio compositor=hyprland seat=virtual guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_APP_RESULT status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_hyprland_window_and_worker_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 hyprland_alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_focused_keyboard_missing'
      [[ "$hypr_injector_ok" == true ]] || fail 'guest_private_hyprland_focus_injector_failed'
      echo 'STARBYTE_GUEST_HYPR_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    fi
    if [[ "$mode" == --starbyte-smoke ]]; then
      grep -Fqx 'STARBYTE_GUEST_APP_RESULT launched=1 status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_window_and_worker_missing'
      echo 'STARBYTE_GUEST_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
    fi
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
 /tmp/hypr-xdg-probe.log >/dev/console || :
      echo "STARBYTE_GUEST_HYPR_XDG_DIAG status=$hypr_xdg_status configured=$hypr_xdg_configured reason=$hypr_xdg_reason" >/dev/console
      # Distinct from the Weston sample: prove focus under Hyprland itself.
      hypr_focus_ready=0
      hypr_focus_clean=0
      hypr_focus_message=0
      hypr_focus_compositor_alive=0
      hypr_focus_status=not_run
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          /usr/bin/weston-simple-shm >/tmp/hypr-focus.log 2>/tmp/hypr-focus.err &
        hypr_focus_pid=$!
        # Only report readiness after wl_keyboard.enter, rather than a
        # fixed three-second liveness check on an unfocused window.
        for focus_probe in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
          if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' /tmp/hypr-focus.err 2>/dev/null &&
             /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
            hypr_focus_ready=1
            echo 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' >/dev/console
            break
          fi
          if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
        if [ "$hypr_focus_ready" = 1 ]; then
          for focus_wait in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
            if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err; then break; fi
            if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
            /bin/busybox sleep 1
          done
        fi
        if /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
          /bin/busybox kill "$hypr_focus_pid" 2>/dev/null || :
        fi
        wait "$hypr_focus_pid" 2>/dev/null
        hypr_focus_status=$?
        if [ "$hypr_focus_status" = 0 ]; then hypr_focus_clean=1; fi
        if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err 2>/dev/null; then hypr_focus_message=1; fi
        if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_focus_compositor_alive=1; fi
      fi
      hypr_focus_stdout=0
      hypr_focus_stderr=0
      hypr_focus_connect=0
      hypr_focus_library=0
      hypr_focus_protocol=0
      hypr_focus_xdg=0
      hypr_focus_shm=0
      hypr_focus_surface=0
      hypr_focus_keyboard_enter=0
      hypr_focus_keyboard_requested=0
      hypr_focus_xdg_configure=0
      hypr_focus_shm_advertised=0
      hypr_focus_registry_requested=0
      hypr_focus_create_surface=0
      hypr_focus_create_xdg=0
      hypr_focus_create_toplevel=0
      hypr_focus_initial_commit=0
      hypr_focus_display_error=0
      hypr_focus_toplevel_close=0
      if [ -s /tmp/hypr-focus.log ]; then hypr_focus_stdout=1; fi
      if [ -s /tmp/hypr-focus.err ]; then hypr_focus_stderr=1; fi
      for log in /tmp/hypr-focus.log /tmp/hypr-focus.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'failed to connect|connection refused|cannot connect|No such file.*wayland' "$log"; then hypr_focus_connect=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_focus_library=1; fi
        if /bin/busybox grep -Eiq 'protocol error|wl_display.*error|wayland.*error|display.*disconnected' "$log"; then hypr_focus_protocol=1; fi
        if /bin/busybox grep -Eiq 'xdg_wm_base|xdg_surface|xdg_toplevel|xdg-shell' "$log"; then hypr_focus_xdg=1; fi
        if /bin/busybox grep -Eiq 'wl_shm|shared memory|shm.*failed' "$log"; then hypr_focus_shm=1; fi
        if /bin/busybox grep -Eiq 'failed to.*(surface|window)|surface.*failed|failed.*buffer' "$log"; then hypr_focus_surface=1; fi
        if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' "$log"; then hypr_focus_keyboard_enter=1; fi
        if /bin/busybox grep -Eq 'wl_seat@[0-9]+\.get_keyboard\(' "$log"; then hypr_focus_keyboard_requested=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_focus_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'wl_registry@[0-9]+\.global.*wl_shm' "$log"; then hypr_focus_shm_advertised=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_focus_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_focus_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_focus_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_focus_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_focus_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_focus_display_error=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.close\(' "$log"; then hypr_focus_toplevel_close=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_FOCUS_DIAG status=$hypr_focus_status stdout=$hypr_focus_stdout stderr=$hypr_focus_stderr connect=$hypr_focus_connect library=$hypr_focus_library protocol=$hypr_focus_protocol xdg=$hypr_focus_xdg shm=$hypr_focus_shm surface=$hypr_focus_surface keyboard_enter=$hypr_focus_keyboard_enter keyboard_requested=$hypr_focus_keyboard_requested xdg_configure=$hypr_focus_xdg_configure shm_advertised=$hypr_focus_shm_advertised registry_requested=$hypr_focus_registry_requested create_surface=$hypr_focus_create_surface create_xdg=$hypr_focus_create_xdg create_toplevel=$hypr_focus_create_toplevel initial_commit=$hypr_focus_initial_commit display_error=$hypr_focus_display_error toplevel_close=$hypr_focus_toplevel_close" >/dev/console
      echo "STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=$hypr_focus_ready clean_exit=$hypr_focus_clean exit_message=$hypr_focus_message hyprland_alive=$hypr_focus_compositor_alive" >/dev/console
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_alive=1; fi
      for log in /tmp/hyprland.log /tmp/hyprland.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|egl_not_initialized' "$log"; then hypr_egl_error=1; fi
        if /bin/busybox grep -Eiq 'gbm.*failed|failed.*gbm|no allocator' "$log"; then hypr_gbm_error=1; fi
        if /bin/busybox grep -Eiq 'libseat.*failed|seat.*failed|session inactive' "$log"; then hypr_seat_error=1; fi
        if /bin/busybox grep -Eiq 'config.*failed|config.*error|parse.*error' "$log"; then hypr_config_error=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|failed.*shader' "$log"; then hypr_library_error=1; fi
        if /bin/busybox grep -Eiq 'no monitor|no output|no suitable mode|failed to commit.*output|create.*head.*failed|mode.*not available|output.*disabled' "$log"; then hypr_monitor_error=1; fi
        if /bin/busybox grep -Eiq 'superuser privileges|shall not be run as the root|privilege check is not omitted' "$log"; then hypr_root_guard=1; fi
      done
      # Reuse the original full-library GUI smoke with Hyprland as the
      # Wayland server, never the owner's display. One exclusive worker
      # must execute 30 demo frames without a shadow legacy session.
      hypr_app_status=not_run
      hypr_app_frames=0
      hypr_app_single_owner=0
      if [ "$hypr_registry" = 1 ] && [ "$hypr_seat" = 1 ]; then
        # The private Hyprland test compared swrast with kms_swrast:
        # both returned EGL_NOT_INITIALIZED. Retain the Weston-proven
        # swrast client setting; no accepted guest GUI cutover is implied.
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          RUST_BACKTRACE=1 \
          STARBYTE_LOG=starbyte=info,eframe=trace,glutin=trace,winit=trace \
          EGL_LOG_LEVEL=debug LIBGL_DEBUG=verbose \
          WINIT_UNIX_BACKEND=wayland \
          EGL_PLATFORM=wayland \
          MESA_LOADER_DRIVER_OVERRIDE=swrast \
          LIBGL_ALWAYS_SOFTWARE=1 \
          GALLIUM_DRIVER=llvmpipe \
          XDG_CONFIG_HOME=/tmp/starbyte-hyprland-app-config \
          XDG_CACHE_HOME=/tmp/starbyte-hyprland-app-cache \
          /bin/starbyte-egui --demo --native-worker-preview --exit-after-frames 30 \
            --config /tmp/starbyte-hyprland-app-config.toml \
            --cache-dir /tmp/starbyte-hyprland-cache \
            >/tmp/starbyte-hyprland-app.log 2>/tmp/starbyte-hyprland-app.err
        hypr_app_status=$?
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_frames=1; fi
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_single_owner=1; fi
      fi
      hypr_app_stdout=0
      hypr_app_stderr=0
      hypr_app_egl=0
      hypr_app_dri=0
      hypr_app_context=0
      hypr_app_wayland=0
      hypr_app_winit=0
      hypr_app_panic=0
      hypr_app_library=0
      hypr_app_config=0
      hypr_app_render=0
      hypr_app_permission=0
      hypr_app_protocol=0
      hypr_app_stack_eframe=0
      hypr_app_stack_glutin=0
      hypr_app_stack_winit=0
      hypr_app_stack_starbyte=0
      hypr_app_pixel_format=0
      hypr_app_surface=0
      hypr_app_missing_global=0
      hypr_app_egl_bad_surface=0
      hypr_app_egl_bad_native_window=0
      hypr_app_egl_create_window_surface=0
      hypr_app_surface_init_failure=0
      hypr_app_xdg_configure=0
      hypr_app_dmabuf_failed=0
      hypr_app_dmabuf_params=0
      hypr_app_wl_surface_attach=0
      hypr_app_wl_drm=0
      hypr_app_registry_requested=0
      hypr_app_create_surface=0
      hypr_app_create_xdg=0
      hypr_app_create_toplevel=0
      hypr_app_initial_commit=0
      hypr_app_display_error=0
      hypr_app_egl_bad_alloc=0
      hypr_app_egl_bad_match=0
      hypr_app_egl_not_initialized=0
      hypr_app_glutin_error=0
      hypr_app_surface_error=0
      hypr_app_backtrace=0
      hypr_app_panic_line=0
      if [ -s /tmp/starbyte-hyprland-app.log ]; then hypr_app_stdout=1; fi
      if [ -s /tmp/starbyte-hyprland-app.err ]; then hypr_app_stderr=1; fi
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|EGL_NOT_INITIALIZED|EGL_BAD_DISPLAY|eglInitialize' "$log"; then hypr_app_egl=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*dri|swrast_dri.so|MESA-LOADER' "$log"; then hypr_app_dri=1; fi
        if /bin/busybox grep -Eiq 'NoAvailablePixelFormat|failed to create.*context|no matching config|eglCreateContext|eglMakeCurrent' "$log"; then hypr_app_context=1; fi
        if /bin/busybox grep -Eiq 'wayland.*connection|failed to connect.*wayland|WaylandError|Wayland backend' "$log"; then hypr_app_wayland=1; fi
        if /bin/busybox grep -Eiq 'EventLoopError|failed to create event loop|winit::|event loop.*failed' "$log"; then hypr_app_winit=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_app_library=1; fi
        if /bin/busybox grep -Eiq 'config.*error|config.*failed|permission denied.*config|read.only|no space left' "$log"; then hypr_app_config=1; fi
        if /bin/busybox grep -Eiq 'glutin|OpenGL|surface.*error|renderer.*failed|failed.*renderer' "$log"; then hypr_app_render=1; fi
        if /bin/busybox grep -Eiq 'Permission denied|Operation not permitted' "$log"; then hypr_app_permission=1; fi
        if /bin/busybox grep -Eiq 'protocol error|xdg_wm_base|xdg_surface|xdg_toplevel|wl_display.*error' "$log"; then hypr_app_protocol=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*eframe::' "$log"; then hypr_app_stack_eframe=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*glutin::' "$log"; then hypr_app_stack_glutin=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*winit::' "$log"; then hypr_app_stack_winit=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*starbyte' "$log"; then hypr_app_stack_starbyte=1; fi
        if /bin/busybox grep -Eiq 'pixel.*format|no matching.*config|no EGL.*config|failed to find.*config' "$log"; then hypr_app_pixel_format=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_SURFACE|EGL_BAD_NATIVE_WINDOW|eglCreateWindowSurface|surface.*init.*fail' "$log"; then hypr_app_surface=1; fi
        if /bin/busybox grep -Eiq 'missing.*(global|extension)|does not support.*(protocol|interface)|required.*(global|extension)' "$log"; then hypr_app_missing_global=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_SURFACE' "$log"; then hypr_app_egl_bad_surface=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_NATIVE_WINDOW' "$log"; then hypr_app_egl_bad_native_window=1; fi
        if /bin/busybox grep -Fq 'eglCreateWindowSurface' "$log"; then hypr_app_egl_create_window_surface=1; fi
        if /bin/busybox grep -Eiq 'surface.*init.*fail' "$log"; then hypr_app_surface_init_failure=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_app_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.failed\(' "$log"; then hypr_app_dmabuf_failed=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.' "$log"; then hypr_app_dmabuf_params=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.attach\(' "$log"; then hypr_app_wl_surface_attach=1; fi
        if /bin/busybox grep -Fq 'wl_drm' "$log"; then hypr_app_wl_drm=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_app_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_app_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_app_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_app_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_app_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_app_display_error=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_ALLOC' "$log"; then hypr_app_egl_bad_alloc=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_MATCH' "$log"; then hypr_app_egl_bad_match=1; fi
        if /bin/busybox grep -Fq 'EGL_NOT_INITIALIZED' "$log"; then hypr_app_egl_not_initialized=1; fi
        if /bin/busybox grep -Eiq '(glutin|GL surface|GlSurface|gl_surface).*(error|fail)|Glutin\(' "$log"; then hypr_app_glutin_error=1; fi
        if /bin/busybox grep -Eiq '(surface|window).*(creation|initialization|configure).*(fail|error)|failed to initialize a window' "$log"; then hypr_app_surface_error=1; fi
        if /bin/busybox grep -Fq 'stack backtrace:' "$log"; then hypr_app_backtrace=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic_line=1; fi
      done
      # Map eframe 0.32's own GL launch messages to fixed stages. The
      # original logs stay private; these flags do not weaken acceptance.
      hypr_eframe_display_start=0
      hypr_eframe_display_ready=0
      hypr_eframe_context_start=0
      hypr_eframe_surface_start=0
      hypr_eframe_surface_ready=0
      hypr_eframe_context_current=0
      hypr_eframe_window_failure=0
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fq 'trying to create glutin Display with config:' "$log"; then hypr_eframe_display_start=1; fi
        if /bin/busybox grep -Fq 'successfully created GL Display with version' "$log"; then hypr_eframe_display_ready=1; fi
        if /bin/busybox grep -Fq 'creating gl context using raw window handle:' "$log"; then hypr_eframe_context_start=1; fi
        if /bin/busybox grep -Fq 'Creating a gl_surface for viewport' "$log"; then hypr_eframe_surface_start=1; fi
        if /bin/busybox grep -Fq 'surface created successfully:' "$log"; then hypr_eframe_surface_ready=1; fi
        if /bin/busybox grep -Fq 'made context current. setting swap interval' "$log"; then hypr_eframe_context_current=1; fi
        if /bin/busybox grep -Fq 'Failed to initialize a window for viewport' "$log"; then hypr_eframe_window_failure=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_EFRAME_DIAG display_start=$hypr_eframe_display_start display_ready=$hypr_eframe_display_ready context_start=$hypr_eframe_context_start surface_start=$hypr_eframe_surface_start surface_ready=$hypr_eframe_surface_ready context_current=$hypr_eframe_context_current window_failure=$hypr_eframe_window_failure" >/dev/console
      # Match glutin 0.32 ErrorKind's fixed public display strings.
      # Never print raw native errors, Wayland traces, paths or guest logs.
      hypr_app_glutin_kind=unclassified
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fqi 'argument does not refer to a valid native window' "$log"; then
          hypr_app_glutin_kind=bad_native_window
        elif /bin/busybox grep -Fqi 'argument does not name a valid surface' "$log"; then
          hypr_app_glutin_kind=bad_surface
        elif /bin/busybox grep -Fqi 'argument does not name a valid display' "$log"; then
          hypr_app_glutin_kind=bad_display
        elif /bin/busybox grep -Fqi 'argument does not name a valid config' "$log"; then
          hypr_app_glutin_kind=bad_config
        elif /bin/busybox grep -Fqi 'argument does not name a valid context' "$log"; then
          hypr_app_glutin_kind=bad_context
        elif /bin/busybox grep -Fqi 'arguments are inconsistent' "$log"; then
          hypr_app_glutin_kind=bad_match
        elif /bin/busybox grep -Fqi 'initialization failed' "$log"; then
          hypr_app_glutin_kind=initialization_failed
        elif /bin/busybox grep -Fqi 'access to the resource failed' "$log"; then
          hypr_app_glutin_kind=bad_access
        elif /bin/busybox grep -Fqi 'out of memory' "$log"; then
          hypr_app_glutin_kind=out_of_memory
        elif /bin/busybox grep -Fqi 'one or more argument values are invalid' "$log"; then
          hypr_app_glutin_kind=bad_parameter
        elif /bin/busybox grep -Fqi 'the context is in a bad state' "$log"; then
          hypr_app_glutin_kind=bad_context_state
        elif /bin/busybox grep -Fqi 'the current surface of the calling thread is no longer valid' "$log"; then
          hypr_app_glutin_kind=bad_current_surface
        elif /bin/busybox grep -Fqi 'context loss' "$log"; then
          hypr_app_glutin_kind=context_lost
        elif /bin/busybox grep -Fqi 'misc platform error' "$log"; then
          hypr_app_glutin_kind=misc
        elif /bin/busybox grep -Fqi 'glutin error:' "$log"; then
          hypr_app_glutin_kind=other_glutin_error
        elif /bin/busybox grep -Fqi 'winit error:' "$log"; then
          hypr_app_glutin_kind=winit_error
        elif /bin/busybox grep -Fqi 'app creation error:' "$log"; then
          hypr_app_glutin_kind=app_creation_error
        fi
        if [ "$hypr_app_glutin_kind" != unclassified ]; then break; fi
      done
      # Only fixed Mesa failure categories may leave private guest RAM.
      hypr_app_dri2_reason=none
      hypr_app_egl_debug=0
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'EGL debug|EGL warning|libEGL|DRI2:' "$log"; then hypr_app_egl_debug=1; fi
        if /bin/busybox grep -Eiq 'DRI2: failed to load driver|failed to load driver: kms_swrast|kms_swrast.*failed to load' "$log"; then
          hypr_app_dri2_reason=load_driver
        elif /bin/busybox grep -Eiq 'DRI2: failed to create screen|failed to create dri screen' "$log"; then
          hypr_app_dri2_reason=create_screen
        elif /bin/busybox grep -Eiq 'failed to setup EGLDevice|failed to set up EGLDevice' "$log"; then
          hypr_app_dri2_reason=setup_device
        elif /bin/busybox grep -Eiq 'failed to find required DRI extensions' "$log"; then
          hypr_app_dri2_reason=missing_extensions
        elif /bin/busybox grep -Eiq 'display does not support prime' "$log"; then
          hypr_app_dri2_reason=prime_unsupported
        elif /bin/busybox grep -Eiq 'failed to add configs|failed to.*EGL.*config' "$log"; then
          hypr_app_dri2_reason=configs
        elif /bin/busybox grep -Eiq 'failed to authenti(cate|cation)|failed to open.*render' "$log"; then
          hypr_app_dri2_reason=drm_access
        elif /bin/busybox grep -Eiq 'failed to load.*swrast_dri|swrast_dri.so.*not found' "$log"; then
          hypr_app_dri2_reason=missing_dri_library
        fi
        if [ "$hypr_app_dri2_reason" != none ]; then break; fi
      done
      echo "STARBYTE_GUEST_HYPRLAND_APP_DIAG stdout=$hypr_app_stdout stderr=$hypr_app_stderr egl=$hypr_app_egl dri=$hypr_app_dri context=$hypr_app_context wayland=$hypr_app_wayland winit=$hypr_app_winit panic=$hypr_app_panic library=$hypr_app_library config=$hypr_app_config render=$hypr_app_render permission=$hypr_app_permission protocol=$hypr_app_protocol stack_eframe=$hypr_app_stack_eframe stack_glutin=$hypr_app_stack_glutin stack_winit=$hypr_app_stack_winit stack_starbyte=$hypr_app_stack_starbyte pixel_format=$hypr_app_pixel_format surface=$hypr_app_surface missing_global=$hypr_app_missing_global egl_bad_surface=$hypr_app_egl_bad_surface egl_bad_native_window=$hypr_app_egl_bad_native_window egl_create_window_surface=$hypr_app_egl_create_window_surface surface_init_failure=$hypr_app_surface_init_failure xdg_configure=$hypr_app_xdg_configure dmabuf_failed=$hypr_app_dmabuf_failed dmabuf_params=$hypr_app_dmabuf_params wl_surface_attach=$hypr_app_wl_surface_attach wl_drm=$hypr_app_wl_drm registry_requested=$hypr_app_registry_requested create_surface=$hypr_app_create_surface create_xdg=$hypr_app_create_xdg create_toplevel=$hypr_app_create_toplevel initial_commit=$hypr_app_initial_commit display_error=$hypr_app_display_error egl_bad_alloc=$hypr_app_egl_bad_alloc egl_bad_match=$hypr_app_egl_bad_match egl_not_initialized=$hypr_app_egl_not_initialized glutin_error=$hypr_app_glutin_error surface_error=$hypr_app_surface_error backtrace=$hypr_app_backtrace panic_line=$hypr_app_panic_line glutin_kind=$hypr_app_glutin_kind mesa_driver=swrast egl_debug=$hypr_app_egl_debug dri2_reason=$hypr_app_dri2_reason" >/dev/console
      # kill -0 alone also succeeds for zombies; require a non-zombie
      # Hyprland process after the application has finished.
      hypr_alive=0
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null && [ -r "/proc/$hypr_pid/stat" ]; then
        hypr_state="$(/bin/busybox awk '{print $3}' "/proc/$hypr_pid/stat")"
        if [ "$hypr_state" != Z ]; then hypr_alive=1; fi
      fi
      echo "STARBYTE_GUEST_HYPRLAND_APP_RESULT status=$hypr_app_status frames=$hypr_app_frames single_owner=$hypr_app_single_owner" >/dev/console
      /bin/busybox kill "$hypr_pid" 2>/dev/null || :
      wait "$hypr_pid" 2>/dev/null || :
      echo "STARBYTE_GUEST_HYPRLAND_DIAG version=$hypr_version config_valid=$hypr_config_valid egl=$hypr_egl_error gbm=$hypr_gbm_error seat_error=$hypr_seat_error config_error=$hypr_config_error library_error=$hypr_library_error root_guard=$hypr_root_guard wire_attempts=$hypr_wire_attempts wire_connected=$hypr_wire_connected wire_sync=$hypr_wire_sync wire_reason=$hypr_wire_reason monitor_error=$hypr_monitor_error" >/dev/console
      echo "STARBYTE_GUEST_HYPRLAND_RESULT executable=$hypr_executable socket=$hypr_socket registry=$hypr_registry compositor=$hypr_compositor seat=$hypr_seat alive=$hypr_alive" >/dev/console
    fi
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
if [[ "$mode" == --gbm-smoke ]]; then guest_ram=768M; fi
if [[ "$mode" == --wayland-smoke ]]; then guest_ram=1536M; fi
if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then guest_ram=2048M; fi
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
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Entirely virtual keyboard, private guest-only QMP socket.
  qmp_args=(-qmp "unix:$root/qmp.sock,server=on,wait=off")
  guest_focus_injector() {
    local ready=false
    # VM startup under TCG can exceed one minute on hosted runners.
    # Remain bounded by the outer 200-second guest-QEMU deadline.
    for _ in {1..2100}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]] \
        && grep -Fq 'STARBYTE_GUEST_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
        ready=true
        break
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    local qmp_status=0
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_focus_injector &
  injector_pid=$!
  trap 'kill "$injector_pid" 2>/dev/null || true; wait "$injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
if [[ "$mode" == --hyprland-smoke ]]; then
  # A second QMP connection after the Weston Escape test has completed.
  # The acknowledgement is not enough: the focused client must exit.
  guest_hypr_focus_injector() {
    local ready=false qmp_status=0
    for _ in {1..2900}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]]; then
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
          ready=true
          break
        fi
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ' "$root/serial.log"; then
          echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_never_ready'
          return 5
        fi
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/hypr-focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/hypr-focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_hypr_focus_injector &
  hypr_injector_pid=$!
  trap 'kill "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; wait "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
display_args=(-display none)
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Private Unix-only VNC display emulation advertises a KMS output.
  # No TCP listener, physical display, viewer, or host desktop socket.
  display_args=(-display "vnc=unix:$root/guest-vnc.sock")
fi
guest_timeout=125
if [[ "$mode" == --wayland-smoke ]]; then guest_timeout=200; fi
if [[ "$mode" == --starbyte-smoke ]]; then guest_timeout=340; fi
if [[ "$mode" == --hyprland-smoke ]]; then guest_timeout=450; fi
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
focus_injector_ok=true
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  wait "$injector_pid" || focus_injector_ok=false
  injector_pid=''
fi
hypr_injector_ok=true
if [[ "$mode" == --hyprland-smoke ]]; then
  wait "$hypr_injector_pid" || hypr_injector_ok=false
  hypr_injector_pid=''
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
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
    # All receipts are bounded diagnostics; never disclose raw guest logs.
    grep -E '^STARBYTE_GUEST_(WAYLAND_(RESULT|DIAG|WIRE_DIAG|WIRE_SELFTEST|BIN_DIAG|REGISTRY_DIAG)|WESTON_(EXEC|STAGE|EXIT)|TMP_WRITE_OK|TMP_MOUNT|TMP_IO_DIAG|RUN_MOUNT|UDEV_RESULT|XKB_DATA|FOCUS_RESULT|APP_RESULT|APP_DIAG|APP_ROOT_DIAG) ' "$root/serial-clean.log" || true
    grep -Fqx 'STARBYTE_GUEST_TMP_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_RUN_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_run_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_UDEV_RESULT daemon=0 trigger=0 settle=0 event=1 keyboard=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_udev_classification_missing'
    grep -Fqx 'STARBYTE_GUEST_XKB_DATA rules=1 symbols=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_xkb_data_unavailable'
    grep -Fqx 'STARBYTE_GUEST_TMP_IO_DIAG write_status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_write_failed'
    grep -Fqx 'STARBYTE_GUEST_TMP_WRITE_OK result=1' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_io_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST status=0 output=1 stderr=0 reason=none' "$root/serial-clean.log" \
      || fail 'guest_wayland_wire_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WESTON_EXEC status=0 stdout=1 stderr=0' "$root/serial-clean.log" \
      || fail 'guest_weston_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_RESULT socket=1 handshake=1 compositor=1 seat=1' "$root/serial-clean.log" \
      || fail 'guest_drm_wayland_handshake_or_seat_missing'
    grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=1 sync=1 output=1 attempts=[1-8] status=0 stderr=0 reason=none$' "$root/serial-clean.log" \
      || fail 'guest_wayland_registry_sync_missing'
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=rust_wire seat=present guest_only=true'
    [[ "$focus_injector_ok" == true ]] || fail 'guest_private_focus_qmp_injection_failed'
    grep -Fqx 'STARBYTE_GUEST_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 weston_alive=1' "$root/serial-clean.log" \
      || fail 'guest_focused_wayland_keyboard_delivery_missing'
    echo 'STARBYTE_GUEST_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    if [[ "$mode" == --hyprland-smoke ]]; then
      grep -E '^STARBYTE_GUEST_(HYPRLAND_(RESULT|DIAG|APP_RESULT|APP_DIAG)|HYPR_FOCUS_(RESULT|DIAG)|HYPR_OUTPUT_DIAG|HYPR_KMS_DIAG|HYPR_EFRAME_DIAG|HYPR_CAP_(BLOCKED|DIAG)|WAYLAND_CAP_RESULT|EGLINFO_DIAG|EGL_ATTACH_DIAG|EGL_ATTACHED_RESULT|EGL_TRACKED_RESULT|EGL_NONE_RESULT|HYPR_XDG_DIAG) ' "$root/serial-clean.log" || true
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_RESULT executable=1 socket=1 registry=1 compositor=1 seat=1 alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_registry_or_seat_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_OK drm=virtio compositor=hyprland seat=virtual guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_APP_RESULT status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_hyprland_window_and_worker_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 hyprland_alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_focused_keyboard_missing'
      [[ "$hypr_injector_ok" == true ]] || fail 'guest_private_hyprland_focus_injector_failed'
      echo 'STARBYTE_GUEST_HYPR_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    fi
    if [[ "$mode" == --starbyte-smoke ]]; then
      grep -Fqx 'STARBYTE_GUEST_APP_RESULT launched=1 status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_window_and_worker_missing'
      echo 'STARBYTE_GUEST_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
    fi
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
 /tmp/hypr-xdg-probe.log >/dev/console || :
      echo "STARBYTE_GUEST_HYPR_XDG_DIAG status=$hypr_xdg_status configured=$hypr_xdg_configured reason=$hypr_xdg_reason" >/dev/console
      # Distinct from the Weston sample: prove focus under Hyprland itself.
      hypr_focus_ready=0
      hypr_focus_clean=0
      hypr_focus_message=0
      hypr_focus_compositor_alive=0
      hypr_focus_status=not_run
      if [ "$hypr_registry" = 1 ]; then
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          /usr/bin/weston-simple-shm >/tmp/hypr-focus.log 2>/tmp/hypr-focus.err &
        hypr_focus_pid=$!
        # Only report readiness after wl_keyboard.enter, rather than a
        # fixed three-second liveness check on an unfocused window.
        for focus_probe in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
          if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' /tmp/hypr-focus.err 2>/dev/null &&
             /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
            hypr_focus_ready=1
            echo 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' >/dev/console
            break
          fi
          if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
          /bin/busybox sleep 1
        done
        if [ "$hypr_focus_ready" = 1 ]; then
          for focus_wait in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25; do
            if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err; then break; fi
            if ! /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then break; fi
            /bin/busybox sleep 1
          done
        fi
        if /bin/busybox kill -0 "$hypr_focus_pid" 2>/dev/null; then
          /bin/busybox kill "$hypr_focus_pid" 2>/dev/null || :
        fi
        wait "$hypr_focus_pid" 2>/dev/null
        hypr_focus_status=$?
        if [ "$hypr_focus_status" = 0 ]; then hypr_focus_clean=1; fi
        if /bin/busybox grep -Fq 'simple-shm exiting' /tmp/hypr-focus.err 2>/dev/null; then hypr_focus_message=1; fi
        if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_focus_compositor_alive=1; fi
      fi
      hypr_focus_stdout=0
      hypr_focus_stderr=0
      hypr_focus_connect=0
      hypr_focus_library=0
      hypr_focus_protocol=0
      hypr_focus_xdg=0
      hypr_focus_shm=0
      hypr_focus_surface=0
      hypr_focus_keyboard_enter=0
      hypr_focus_keyboard_requested=0
      hypr_focus_xdg_configure=0
      hypr_focus_shm_advertised=0
      hypr_focus_registry_requested=0
      hypr_focus_create_surface=0
      hypr_focus_create_xdg=0
      hypr_focus_create_toplevel=0
      hypr_focus_initial_commit=0
      hypr_focus_display_error=0
      hypr_focus_toplevel_close=0
      if [ -s /tmp/hypr-focus.log ]; then hypr_focus_stdout=1; fi
      if [ -s /tmp/hypr-focus.err ]; then hypr_focus_stderr=1; fi
      for log in /tmp/hypr-focus.log /tmp/hypr-focus.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'failed to connect|connection refused|cannot connect|No such file.*wayland' "$log"; then hypr_focus_connect=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_focus_library=1; fi
        if /bin/busybox grep -Eiq 'protocol error|wl_display.*error|wayland.*error|display.*disconnected' "$log"; then hypr_focus_protocol=1; fi
        if /bin/busybox grep -Eiq 'xdg_wm_base|xdg_surface|xdg_toplevel|xdg-shell' "$log"; then hypr_focus_xdg=1; fi
        if /bin/busybox grep -Eiq 'wl_shm|shared memory|shm.*failed' "$log"; then hypr_focus_shm=1; fi
        if /bin/busybox grep -Eiq 'failed to.*(surface|window)|surface.*failed|failed.*buffer' "$log"; then hypr_focus_surface=1; fi
        if /bin/busybox grep -Eq 'wl_keyboard@[0-9]+\.enter\(' "$log"; then hypr_focus_keyboard_enter=1; fi
        if /bin/busybox grep -Eq 'wl_seat@[0-9]+\.get_keyboard\(' "$log"; then hypr_focus_keyboard_requested=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_focus_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'wl_registry@[0-9]+\.global.*wl_shm' "$log"; then hypr_focus_shm_advertised=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_focus_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_focus_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_focus_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_focus_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_focus_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_focus_display_error=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.close\(' "$log"; then hypr_focus_toplevel_close=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_FOCUS_DIAG status=$hypr_focus_status stdout=$hypr_focus_stdout stderr=$hypr_focus_stderr connect=$hypr_focus_connect library=$hypr_focus_library protocol=$hypr_focus_protocol xdg=$hypr_focus_xdg shm=$hypr_focus_shm surface=$hypr_focus_surface keyboard_enter=$hypr_focus_keyboard_enter keyboard_requested=$hypr_focus_keyboard_requested xdg_configure=$hypr_focus_xdg_configure shm_advertised=$hypr_focus_shm_advertised registry_requested=$hypr_focus_registry_requested create_surface=$hypr_focus_create_surface create_xdg=$hypr_focus_create_xdg create_toplevel=$hypr_focus_create_toplevel initial_commit=$hypr_focus_initial_commit display_error=$hypr_focus_display_error toplevel_close=$hypr_focus_toplevel_close" >/dev/console
      echo "STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=$hypr_focus_ready clean_exit=$hypr_focus_clean exit_message=$hypr_focus_message hyprland_alive=$hypr_focus_compositor_alive" >/dev/console
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null; then hypr_alive=1; fi
      for log in /tmp/hyprland.log /tmp/hyprland.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|egl_not_initialized' "$log"; then hypr_egl_error=1; fi
        if /bin/busybox grep -Eiq 'gbm.*failed|failed.*gbm|no allocator' "$log"; then hypr_gbm_error=1; fi
        if /bin/busybox grep -Eiq 'libseat.*failed|seat.*failed|session inactive' "$log"; then hypr_seat_error=1; fi
        if /bin/busybox grep -Eiq 'config.*failed|config.*error|parse.*error' "$log"; then hypr_config_error=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|failed.*shader' "$log"; then hypr_library_error=1; fi
        if /bin/busybox grep -Eiq 'no monitor|no output|no suitable mode|failed to commit.*output|create.*head.*failed|mode.*not available|output.*disabled' "$log"; then hypr_monitor_error=1; fi
        if /bin/busybox grep -Eiq 'superuser privileges|shall not be run as the root|privilege check is not omitted' "$log"; then hypr_root_guard=1; fi
      done
      # Reuse the original full-library GUI smoke with Hyprland as the
      # Wayland server, never the owner's display. One exclusive worker
      # must execute 30 demo frames without a shadow legacy session.
      hypr_app_status=not_run
      hypr_app_frames=0
      hypr_app_single_owner=0
      if [ "$hypr_registry" = 1 ] && [ "$hypr_seat" = 1 ]; then
        # The private Hyprland test compared swrast with kms_swrast:
        # both returned EGL_NOT_INITIALIZED. Retain the Weston-proven
        # swrast client setting; no accepted guest GUI cutover is implied.
        WAYLAND_DEBUG=client WAYLAND_DISPLAY="$hypr_display" \
          RUST_BACKTRACE=1 \
          STARBYTE_LOG=starbyte=info,eframe=trace,glutin=trace,winit=trace \
          EGL_LOG_LEVEL=debug LIBGL_DEBUG=verbose \
          WINIT_UNIX_BACKEND=wayland \
          EGL_PLATFORM=wayland \
          MESA_LOADER_DRIVER_OVERRIDE=swrast \
          LIBGL_ALWAYS_SOFTWARE=1 \
          GALLIUM_DRIVER=llvmpipe \
          XDG_CONFIG_HOME=/tmp/starbyte-hyprland-app-config \
          XDG_CACHE_HOME=/tmp/starbyte-hyprland-app-cache \
          /bin/starbyte-egui --demo --native-worker-preview --exit-after-frames 30 \
            --config /tmp/starbyte-hyprland-app-config.toml \
            --cache-dir /tmp/starbyte-hyprland-cache \
            >/tmp/starbyte-hyprland-app.log 2>/tmp/starbyte-hyprland-app.err
        hypr_app_status=$?
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_frames=1; fi
        if /bin/busybox grep -Fq 'STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true worker_frame=' /tmp/starbyte-hyprland-app.log; then hypr_app_single_owner=1; fi
      fi
      hypr_app_stdout=0
      hypr_app_stderr=0
      hypr_app_egl=0
      hypr_app_dri=0
      hypr_app_context=0
      hypr_app_wayland=0
      hypr_app_winit=0
      hypr_app_panic=0
      hypr_app_library=0
      hypr_app_config=0
      hypr_app_render=0
      hypr_app_permission=0
      hypr_app_protocol=0
      hypr_app_stack_eframe=0
      hypr_app_stack_glutin=0
      hypr_app_stack_winit=0
      hypr_app_stack_starbyte=0
      hypr_app_pixel_format=0
      hypr_app_surface=0
      hypr_app_missing_global=0
      hypr_app_egl_bad_surface=0
      hypr_app_egl_bad_native_window=0
      hypr_app_egl_create_window_surface=0
      hypr_app_surface_init_failure=0
      hypr_app_xdg_configure=0
      hypr_app_dmabuf_failed=0
      hypr_app_dmabuf_params=0
      hypr_app_wl_surface_attach=0
      hypr_app_wl_drm=0
      hypr_app_registry_requested=0
      hypr_app_create_surface=0
      hypr_app_create_xdg=0
      hypr_app_create_toplevel=0
      hypr_app_initial_commit=0
      hypr_app_display_error=0
      hypr_app_egl_bad_alloc=0
      hypr_app_egl_bad_match=0
      hypr_app_egl_not_initialized=0
      hypr_app_glutin_error=0
      hypr_app_surface_error=0
      hypr_app_backtrace=0
      hypr_app_panic_line=0
      if [ -s /tmp/starbyte-hyprland-app.log ]; then hypr_app_stdout=1; fi
      if [ -s /tmp/starbyte-hyprland-app.err ]; then hypr_app_stderr=1; fi
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'egl.*failed|failed.*egl|EGL_NOT_INITIALIZED|EGL_BAD_DISPLAY|eglInitialize' "$log"; then hypr_app_egl=1; fi
        if /bin/busybox grep -Eiq 'failed to load driver|failed to open.*dri|swrast_dri.so|MESA-LOADER' "$log"; then hypr_app_dri=1; fi
        if /bin/busybox grep -Eiq 'NoAvailablePixelFormat|failed to create.*context|no matching config|eglCreateContext|eglMakeCurrent' "$log"; then hypr_app_context=1; fi
        if /bin/busybox grep -Eiq 'wayland.*connection|failed to connect.*wayland|WaylandError|Wayland backend' "$log"; then hypr_app_wayland=1; fi
        if /bin/busybox grep -Eiq 'EventLoopError|failed to create event loop|winit::|event loop.*failed' "$log"; then hypr_app_winit=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic=1; fi
        if /bin/busybox grep -Eiq 'error while loading shared libraries|cannot open shared object|symbol lookup error' "$log"; then hypr_app_library=1; fi
        if /bin/busybox grep -Eiq 'config.*error|config.*failed|permission denied.*config|read.only|no space left' "$log"; then hypr_app_config=1; fi
        if /bin/busybox grep -Eiq 'glutin|OpenGL|surface.*error|renderer.*failed|failed.*renderer' "$log"; then hypr_app_render=1; fi
        if /bin/busybox grep -Eiq 'Permission denied|Operation not permitted' "$log"; then hypr_app_permission=1; fi
        if /bin/busybox grep -Eiq 'protocol error|xdg_wm_base|xdg_surface|xdg_toplevel|wl_display.*error' "$log"; then hypr_app_protocol=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*eframe::' "$log"; then hypr_app_stack_eframe=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*glutin::' "$log"; then hypr_app_stack_glutin=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*winit::' "$log"; then hypr_app_stack_winit=1; fi
        if /bin/busybox grep -Eq '^[[:space:]]*[0-9]+:.*starbyte' "$log"; then hypr_app_stack_starbyte=1; fi
        if /bin/busybox grep -Eiq 'pixel.*format|no matching.*config|no EGL.*config|failed to find.*config' "$log"; then hypr_app_pixel_format=1; fi
        if /bin/busybox grep -Eiq 'EGL_BAD_SURFACE|EGL_BAD_NATIVE_WINDOW|eglCreateWindowSurface|surface.*init.*fail' "$log"; then hypr_app_surface=1; fi
        if /bin/busybox grep -Eiq 'missing.*(global|extension)|does not support.*(protocol|interface)|required.*(global|extension)' "$log"; then hypr_app_missing_global=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_SURFACE' "$log"; then hypr_app_egl_bad_surface=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_NATIVE_WINDOW' "$log"; then hypr_app_egl_bad_native_window=1; fi
        if /bin/busybox grep -Fq 'eglCreateWindowSurface' "$log"; then hypr_app_egl_create_window_surface=1; fi
        if /bin/busybox grep -Eiq 'surface.*init.*fail' "$log"; then hypr_app_surface_init_failure=1; fi
        if /bin/busybox grep -Eq 'xdg_toplevel@[0-9]+\.configure\(' "$log"; then hypr_app_xdg_configure=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.failed\(' "$log"; then hypr_app_dmabuf_failed=1; fi
        if /bin/busybox grep -Eq 'zwp_linux_buffer_params_v1@[0-9]+\.' "$log"; then hypr_app_dmabuf_params=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.attach\(' "$log"; then hypr_app_wl_surface_attach=1; fi
        if /bin/busybox grep -Fq 'wl_drm' "$log"; then hypr_app_wl_drm=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.get_registry\(' "$log"; then hypr_app_registry_requested=1; fi
        if /bin/busybox grep -Eq 'wl_compositor@[0-9]+\.create_surface\(' "$log"; then hypr_app_create_surface=1; fi
        if /bin/busybox grep -Eq 'xdg_wm_base@[0-9]+\.get_xdg_surface\(' "$log"; then hypr_app_create_xdg=1; fi
        if /bin/busybox grep -Eq 'xdg_surface@[0-9]+\.get_toplevel\(' "$log"; then hypr_app_create_toplevel=1; fi
        if /bin/busybox grep -Eq 'wl_surface@[0-9]+\.commit\(' "$log"; then hypr_app_initial_commit=1; fi
        if /bin/busybox grep -Eq 'wl_display@[0-9]+\.error\(' "$log"; then hypr_app_display_error=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_ALLOC' "$log"; then hypr_app_egl_bad_alloc=1; fi
        if /bin/busybox grep -Fq 'EGL_BAD_MATCH' "$log"; then hypr_app_egl_bad_match=1; fi
        if /bin/busybox grep -Fq 'EGL_NOT_INITIALIZED' "$log"; then hypr_app_egl_not_initialized=1; fi
        if /bin/busybox grep -Eiq '(glutin|GL surface|GlSurface|gl_surface).*(error|fail)|Glutin\(' "$log"; then hypr_app_glutin_error=1; fi
        if /bin/busybox grep -Eiq '(surface|window).*(creation|initialization|configure).*(fail|error)|failed to initialize a window' "$log"; then hypr_app_surface_error=1; fi
        if /bin/busybox grep -Fq 'stack backtrace:' "$log"; then hypr_app_backtrace=1; fi
        if /bin/busybox grep -Eiq "thread '.*' panicked at" "$log"; then hypr_app_panic_line=1; fi
      done
      # Map eframe 0.32's own GL launch messages to fixed stages. The
      # original logs stay private; these flags do not weaken acceptance.
      hypr_eframe_display_start=0
      hypr_eframe_display_ready=0
      hypr_eframe_context_start=0
      hypr_eframe_surface_start=0
      hypr_eframe_surface_ready=0
      hypr_eframe_context_current=0
      hypr_eframe_window_failure=0
      for log in /tmp/starbyte-hyprland-app.log /tmp/starbyte-hyprland-app.err; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fq 'trying to create glutin Display with config:' "$log"; then hypr_eframe_display_start=1; fi
        if /bin/busybox grep -Fq 'successfully created GL Display with version' "$log"; then hypr_eframe_display_ready=1; fi
        if /bin/busybox grep -Fq 'creating gl context using raw window handle:' "$log"; then hypr_eframe_context_start=1; fi
        if /bin/busybox grep -Fq 'Creating a gl_surface for viewport' "$log"; then hypr_eframe_surface_start=1; fi
        if /bin/busybox grep -Fq 'surface created successfully:' "$log"; then hypr_eframe_surface_ready=1; fi
        if /bin/busybox grep -Fq 'made context current. setting swap interval' "$log"; then hypr_eframe_context_current=1; fi
        if /bin/busybox grep -Fq 'Failed to initialize a window for viewport' "$log"; then hypr_eframe_window_failure=1; fi
      done
      echo "STARBYTE_GUEST_HYPR_EFRAME_DIAG display_start=$hypr_eframe_display_start display_ready=$hypr_eframe_display_ready context_start=$hypr_eframe_context_start surface_start=$hypr_eframe_surface_start surface_ready=$hypr_eframe_surface_ready context_current=$hypr_eframe_context_current window_failure=$hypr_eframe_window_failure" >/dev/console
      # Match glutin 0.32 ErrorKind's fixed public display strings.
      # Never print raw native errors, Wayland traces, paths or guest logs.
      hypr_app_glutin_kind=unclassified
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Fqi 'argument does not refer to a valid native window' "$log"; then
          hypr_app_glutin_kind=bad_native_window
        elif /bin/busybox grep -Fqi 'argument does not name a valid surface' "$log"; then
          hypr_app_glutin_kind=bad_surface
        elif /bin/busybox grep -Fqi 'argument does not name a valid display' "$log"; then
          hypr_app_glutin_kind=bad_display
        elif /bin/busybox grep -Fqi 'argument does not name a valid config' "$log"; then
          hypr_app_glutin_kind=bad_config
        elif /bin/busybox grep -Fqi 'argument does not name a valid context' "$log"; then
          hypr_app_glutin_kind=bad_context
        elif /bin/busybox grep -Fqi 'arguments are inconsistent' "$log"; then
          hypr_app_glutin_kind=bad_match
        elif /bin/busybox grep -Fqi 'initialization failed' "$log"; then
          hypr_app_glutin_kind=initialization_failed
        elif /bin/busybox grep -Fqi 'access to the resource failed' "$log"; then
          hypr_app_glutin_kind=bad_access
        elif /bin/busybox grep -Fqi 'out of memory' "$log"; then
          hypr_app_glutin_kind=out_of_memory
        elif /bin/busybox grep -Fqi 'one or more argument values are invalid' "$log"; then
          hypr_app_glutin_kind=bad_parameter
        elif /bin/busybox grep -Fqi 'the context is in a bad state' "$log"; then
          hypr_app_glutin_kind=bad_context_state
        elif /bin/busybox grep -Fqi 'the current surface of the calling thread is no longer valid' "$log"; then
          hypr_app_glutin_kind=bad_current_surface
        elif /bin/busybox grep -Fqi 'context loss' "$log"; then
          hypr_app_glutin_kind=context_lost
        elif /bin/busybox grep -Fqi 'misc platform error' "$log"; then
          hypr_app_glutin_kind=misc
        elif /bin/busybox grep -Fqi 'glutin error:' "$log"; then
          hypr_app_glutin_kind=other_glutin_error
        elif /bin/busybox grep -Fqi 'winit error:' "$log"; then
          hypr_app_glutin_kind=winit_error
        elif /bin/busybox grep -Fqi 'app creation error:' "$log"; then
          hypr_app_glutin_kind=app_creation_error
        fi
        if [ "$hypr_app_glutin_kind" != unclassified ]; then break; fi
      done
      # Only fixed Mesa failure categories may leave private guest RAM.
      hypr_app_dri2_reason=none
      hypr_app_egl_debug=0
      for log in /tmp/starbyte-hyprland-app.err /tmp/starbyte-hyprland-app.log; do
        [ -f "$log" ] || continue
        if /bin/busybox grep -Eiq 'EGL debug|EGL warning|libEGL|DRI2:' "$log"; then hypr_app_egl_debug=1; fi
        if /bin/busybox grep -Eiq 'DRI2: failed to load driver|failed to load driver: kms_swrast|kms_swrast.*failed to load' "$log"; then
          hypr_app_dri2_reason=load_driver
        elif /bin/busybox grep -Eiq 'DRI2: failed to create screen|failed to create dri screen' "$log"; then
          hypr_app_dri2_reason=create_screen
        elif /bin/busybox grep -Eiq 'failed to setup EGLDevice|failed to set up EGLDevice' "$log"; then
          hypr_app_dri2_reason=setup_device
        elif /bin/busybox grep -Eiq 'failed to find required DRI extensions' "$log"; then
          hypr_app_dri2_reason=missing_extensions
        elif /bin/busybox grep -Eiq 'display does not support prime' "$log"; then
          hypr_app_dri2_reason=prime_unsupported
        elif /bin/busybox grep -Eiq 'failed to add configs|failed to.*EGL.*config' "$log"; then
          hypr_app_dri2_reason=configs
        elif /bin/busybox grep -Eiq 'failed to authenti(cate|cation)|failed to open.*render' "$log"; then
          hypr_app_dri2_reason=drm_access
        elif /bin/busybox grep -Eiq 'failed to load.*swrast_dri|swrast_dri.so.*not found' "$log"; then
          hypr_app_dri2_reason=missing_dri_library
        fi
        if [ "$hypr_app_dri2_reason" != none ]; then break; fi
      done
      echo "STARBYTE_GUEST_HYPRLAND_APP_DIAG stdout=$hypr_app_stdout stderr=$hypr_app_stderr egl=$hypr_app_egl dri=$hypr_app_dri context=$hypr_app_context wayland=$hypr_app_wayland winit=$hypr_app_winit panic=$hypr_app_panic library=$hypr_app_library config=$hypr_app_config render=$hypr_app_render permission=$hypr_app_permission protocol=$hypr_app_protocol stack_eframe=$hypr_app_stack_eframe stack_glutin=$hypr_app_stack_glutin stack_winit=$hypr_app_stack_winit stack_starbyte=$hypr_app_stack_starbyte pixel_format=$hypr_app_pixel_format surface=$hypr_app_surface missing_global=$hypr_app_missing_global egl_bad_surface=$hypr_app_egl_bad_surface egl_bad_native_window=$hypr_app_egl_bad_native_window egl_create_window_surface=$hypr_app_egl_create_window_surface surface_init_failure=$hypr_app_surface_init_failure xdg_configure=$hypr_app_xdg_configure dmabuf_failed=$hypr_app_dmabuf_failed dmabuf_params=$hypr_app_dmabuf_params wl_surface_attach=$hypr_app_wl_surface_attach wl_drm=$hypr_app_wl_drm registry_requested=$hypr_app_registry_requested create_surface=$hypr_app_create_surface create_xdg=$hypr_app_create_xdg create_toplevel=$hypr_app_create_toplevel initial_commit=$hypr_app_initial_commit display_error=$hypr_app_display_error egl_bad_alloc=$hypr_app_egl_bad_alloc egl_bad_match=$hypr_app_egl_bad_match egl_not_initialized=$hypr_app_egl_not_initialized glutin_error=$hypr_app_glutin_error surface_error=$hypr_app_surface_error backtrace=$hypr_app_backtrace panic_line=$hypr_app_panic_line glutin_kind=$hypr_app_glutin_kind mesa_driver=swrast egl_debug=$hypr_app_egl_debug dri2_reason=$hypr_app_dri2_reason" >/dev/console
      # kill -0 alone also succeeds for zombies; require a non-zombie
      # Hyprland process after the application has finished.
      hypr_alive=0
      if /bin/busybox kill -0 "$hypr_pid" 2>/dev/null && [ -r "/proc/$hypr_pid/stat" ]; then
        hypr_state="$(/bin/busybox awk '{print $3}' "/proc/$hypr_pid/stat")"
        if [ "$hypr_state" != Z ]; then hypr_alive=1; fi
      fi
      echo "STARBYTE_GUEST_HYPRLAND_APP_RESULT status=$hypr_app_status frames=$hypr_app_frames single_owner=$hypr_app_single_owner" >/dev/console
      /bin/busybox kill "$hypr_pid" 2>/dev/null || :
      wait "$hypr_pid" 2>/dev/null || :
      echo "STARBYTE_GUEST_HYPRLAND_DIAG version=$hypr_version config_valid=$hypr_config_valid egl=$hypr_egl_error gbm=$hypr_gbm_error seat_error=$hypr_seat_error config_error=$hypr_config_error library_error=$hypr_library_error root_guard=$hypr_root_guard wire_attempts=$hypr_wire_attempts wire_connected=$hypr_wire_connected wire_sync=$hypr_wire_sync wire_reason=$hypr_wire_reason monitor_error=$hypr_monitor_error" >/dev/console
      echo "STARBYTE_GUEST_HYPRLAND_RESULT executable=$hypr_executable socket=$hypr_socket registry=$hypr_registry compositor=$hypr_compositor seat=$hypr_seat alive=$hypr_alive" >/dev/console
    fi
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
if [[ "$mode" == --gbm-smoke ]]; then guest_ram=768M; fi
if [[ "$mode" == --wayland-smoke ]]; then guest_ram=1536M; fi
if [[ "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then guest_ram=2048M; fi
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
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Entirely virtual keyboard, private guest-only QMP socket.
  qmp_args=(-qmp "unix:$root/qmp.sock,server=on,wait=off")
  guest_focus_injector() {
    local ready=false
    # VM startup under TCG can exceed one minute on hosted runners.
    # Remain bounded by the outer 200-second guest-QEMU deadline.
    for _ in {1..2100}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]] \
        && grep -Fq 'STARBYTE_GUEST_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
        ready=true
        break
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    local qmp_status=0
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_focus_injector &
  injector_pid=$!
  trap 'kill "$injector_pid" 2>/dev/null || true; wait "$injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
if [[ "$mode" == --hyprland-smoke ]]; then
  # A second QMP connection after the Weston Escape test has completed.
  # The acknowledgement is not enough: the focused client must exit.
  guest_hypr_focus_injector() {
    local ready=false qmp_status=0
    for _ in {1..2900}; do
      if [[ -S "$root/qmp.sock" && -f "$root/serial.log" ]]; then
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_READY client=weston-simple-shm alive=1' "$root/serial.log"; then
          ready=true
          break
        fi
        if grep -Fq 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ' "$root/serial.log"; then
          echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_never_ready'
          return 5
        fi
      fi
      sleep 0.1
    done
    [[ "$ready" == true ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED focused_client_not_ready'; return 5; }
    {
      printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
      printf '%s\n' '{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]},"id":"escape"}'
    } | timeout --signal=TERM --kill-after=1s 5s \
      socat -T 3 STDIO "UNIX-CONNECT:$root/qmp.sock" \
      >"$root/hypr-focus-qmp.jsonl" 2>/dev/null || qmp_status=$?
    [[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]] || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED transport'; return 5; }
    jq -es 'any(.[]; .id == "cap" and .return == {}) and any(.[]; .id == "escape" and .return == {})' \
      "$root/hypr-focus-qmp.jsonl" >/dev/null || { echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_BLOCKED injection_rejected'; return 5; }
    echo 'STARBYTE_GUEST_HYPR_FOCUS_QMP_ACK escape=sent source=private_qmp'
  }
  guest_hypr_focus_injector &
  hypr_injector_pid=$!
  trap 'kill "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; wait "$injector_pid" "$hypr_injector_pid" 2>/dev/null || true; rm -rf -- "$root"' EXIT
fi
display_args=(-display none)
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  # Private Unix-only VNC display emulation advertises a KMS output.
  # No TCP listener, physical display, viewer, or host desktop socket.
  display_args=(-display "vnc=unix:$root/guest-vnc.sock")
fi
guest_timeout=125
if [[ "$mode" == --wayland-smoke ]]; then guest_timeout=200; fi
if [[ "$mode" == --starbyte-smoke ]]; then guest_timeout=340; fi
if [[ "$mode" == --hyprland-smoke ]]; then guest_timeout=450; fi
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
focus_injector_ok=true
if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  wait "$injector_pid" || focus_injector_ok=false
  injector_pid=''
fi
hypr_injector_ok=true
if [[ "$mode" == --hyprland-smoke ]]; then
  wait "$hypr_injector_pid" || hypr_injector_ok=false
  hypr_injector_pid=''
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
if [[ "$mode" == --driver-smoke || "$mode" == --gbm-smoke || "$mode" == --input-smoke || "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  if [[ "$mode" == --wayland-smoke || "$mode" == --starbyte-smoke || "$mode" == --hyprland-smoke ]]; then
    # All receipts are bounded diagnostics; never disclose raw guest logs.
    grep -E '^STARBYTE_GUEST_(WAYLAND_(RESULT|DIAG|WIRE_DIAG|WIRE_SELFTEST|BIN_DIAG|REGISTRY_DIAG)|WESTON_(EXEC|STAGE|EXIT)|TMP_WRITE_OK|TMP_MOUNT|TMP_IO_DIAG|RUN_MOUNT|UDEV_RESULT|XKB_DATA|FOCUS_RESULT|APP_RESULT|APP_DIAG|APP_ROOT_DIAG) ' "$root/serial-clean.log" || true
    grep -Fqx 'STARBYTE_GUEST_TMP_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_RUN_MOUNT status=0' "$root/serial-clean.log" \
      || fail 'guest_run_mount_unavailable'
    grep -Fqx 'STARBYTE_GUEST_UDEV_RESULT daemon=0 trigger=0 settle=0 event=1 keyboard=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_udev_classification_missing'
    grep -Fqx 'STARBYTE_GUEST_XKB_DATA rules=1 symbols=1' "$root/serial-clean.log" \
      || fail 'guest_keyboard_xkb_data_unavailable'
    grep -Fqx 'STARBYTE_GUEST_TMP_IO_DIAG write_status=0' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_write_failed'
    grep -Fqx 'STARBYTE_GUEST_TMP_WRITE_OK result=1' "$root/serial-clean.log" \
      || fail 'guest_temporary_file_io_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_WIRE_SELFTEST status=0 output=1 stderr=0 reason=none' "$root/serial-clean.log" \
      || fail 'guest_wayland_wire_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WESTON_EXEC status=0 stdout=1 stderr=0' "$root/serial-clean.log" \
      || fail 'guest_weston_executable_unavailable'
    grep -Fqx 'STARBYTE_GUEST_WAYLAND_RESULT socket=1 handshake=1 compositor=1 seat=1' "$root/serial-clean.log" \
      || fail 'guest_drm_wayland_handshake_or_seat_missing'
    grep -Eq '^STARBYTE_GUEST_WAYLAND_WIRE_DIAG connected=1 sync=1 output=1 attempts=[1-8] status=0 stderr=0 reason=none$' "$root/serial-clean.log" \
      || fail 'guest_wayland_registry_sync_missing'
    echo 'STARBYTE_GUEST_WAYLAND_OK drm=virtio compositor=weston client=rust_wire seat=present guest_only=true'
    [[ "$focus_injector_ok" == true ]] || fail 'guest_private_focus_qmp_injection_failed'
    grep -Fqx 'STARBYTE_GUEST_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 weston_alive=1' "$root/serial-clean.log" \
      || fail 'guest_focused_wayland_keyboard_delivery_missing'
    echo 'STARBYTE_GUEST_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    if [[ "$mode" == --hyprland-smoke ]]; then
      grep -E '^STARBYTE_GUEST_(HYPRLAND_(RESULT|DIAG|APP_RESULT|APP_DIAG)|HYPR_FOCUS_(RESULT|DIAG)|HYPR_OUTPUT_DIAG|HYPR_KMS_DIAG|HYPR_EFRAME_DIAG|HYPR_CAP_(BLOCKED|DIAG)|WAYLAND_CAP_RESULT|EGLINFO_DIAG|EGL_ATTACH_DIAG|EGL_ATTACHED_RESULT|EGL_TRACKED_RESULT|EGL_NONE_RESULT|HYPR_XDG_DIAG) ' "$root/serial-clean.log" || true
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_RESULT executable=1 socket=1 registry=1 compositor=1 seat=1 alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_registry_or_seat_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_OK drm=virtio compositor=hyprland seat=virtual guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPRLAND_APP_RESULT status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_hyprland_window_and_worker_missing'
      echo 'STARBYTE_GUEST_HYPRLAND_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
      grep -Fqx 'STARBYTE_GUEST_HYPR_FOCUS_RESULT ready=1 clean_exit=1 exit_message=1 hyprland_alive=1' "$root/serial-clean.log" \
        || fail 'guest_private_hyprland_focused_keyboard_missing'
      [[ "$hypr_injector_ok" == true ]] || fail 'guest_private_hyprland_focus_injector_failed'
      echo 'STARBYTE_GUEST_HYPR_FOCUS_OK client=weston-simple-shm escape=received source=private_qmp guest_only=true'
    fi
    if [[ "$mode" == --starbyte-smoke ]]; then
      grep -Fqx 'STARBYTE_GUEST_APP_RESULT launched=1 status=0 frames=1 single_owner=1' "$root/serial-clean.log" \
        || fail 'guest_real_starbyte_window_and_worker_missing'
      echo 'STARBYTE_GUEST_APP_OK shell=original_full_library emulator_owner=worker frames=30 guest_only=true'
    fi
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
