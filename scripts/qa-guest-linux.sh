#!/usr/bin/env bash
# Disposable Linux guest PCI smoke; hosted CI only. Never uses host desktop.
set -Eeuo pipefail
umask 077
fail() { printf 'STARBYTE_GUEST_LINUX_BLOCKED %s\n' "$1" >&2; exit 3; }
mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --smoke || "$mode" == --driver-smoke ) ]] || fail 'usage: --check-only|--smoke|--driver-smoke'
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
if [[ "$mode" == --driver-smoke ]]; then
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
fi
/bin/busybox reboot -f
while :; do /bin/busybox sleep 1; done
GUEST_INIT
chmod 755 "$root/initfs/init"
(cd "$root/initfs"; find . -print0 | cpio --null -o -H newc --quiet | gzip -9) >"$root/initramfs.gz"
[[ -s "$root/initramfs.gz" ]] || fail 'initramfs_creation_failed'

# Guest devices are entirely emulated. Never expose host DRM/input/files.
# -cpu max remains TCG-only; QEMU's default qemu64 CPU may lack instructions
# needed by Ubuntu's x86-64-v3 static BusyBox, killing PID 1 before output.
set +e
HOME="$root" XDG_CONFIG_HOME="$root" \
  timeout --signal=TERM --kill-after=2s 75s qemu-system-x86_64 \
    -machine q35,accel=tcg -cpu max -m 384M -smp 1 -no-reboot \
    -nodefaults -display none -vga none -net none \
    -monitor none -parallel none -serial "file:$root/serial.log" \
    -device virtio-gpu-pci,id=sb_gpu -device virtio-keyboard-pci,id=sb_keyboard \
    -kernel "$root/vmlinuz-virt" -initrd "$root/initramfs.gz" \
    -append 'console=ttyS0,115200 loglevel=3 panic=5' \
    >"$root/qemu.log" 2>&1
result=$?
set -e
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
if [[ "$mode" == --driver-smoke ]]; then
  driver=false
  if grep -Fqx 'STARBYTE_GUEST_DRIVER_RESULT render=1 input=1 gpu_load=0 input_load=0 evdev_load=0' "$root/serial-clean.log"; then
    driver=true
  fi
  printf 'STARBYTE_GUEST_DRIVER_DIAG render_and_input=%s\n' "$driver"
  [[ "$driver" == true ]] || fail 'guest_driver_or_device_nodes_missing'
  echo 'STARBYTE_GUEST_DRM_INPUT_OK drm_render_node=present evdev_input_node=present guest_only=true'
else
  echo 'STARBYTE_GUEST_LINUX_PCI_OK gpu=virtio keyboard=virtio serial_only=true drm_render_node=not_tested input_seat=not_tested'
fi
