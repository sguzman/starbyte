#!/usr/bin/env bash
# Disposable Linux guest PCI smoke; hosted CI only. Never uses host desktop.
set -Eeuo pipefail
umask 077
fail() { printf 'STARBYTE_GUEST_LINUX_BLOCKED %s\n' "$1" >&2; exit 3; }
mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --smoke ) ]] || fail 'usage: --check-only|--smoke'
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
/bin/busybox reboot -f
while :; do /bin/busybox sleep 1; done
GUEST_INIT
chmod 755 "$root/initfs/init"
(cd "$root/initfs"; find . -print0 | cpio --null -o -H newc --quiet | gzip -9) >"$root/initramfs.gz"
[[ -s "$root/initramfs.gz" ]] || fail 'initramfs_creation_failed'

# Guest devices are entirely emulated. Never expose host DRM/input/files.
set +e
HOME="$root" XDG_CONFIG_HOME="$root" \
  timeout --signal=TERM --kill-after=2s 75s qemu-system-x86_64 \
    -machine q35,accel=tcg -m 384M -smp 1 -no-reboot \
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
echo 'STARBYTE_GUEST_LINUX_PCI_OK gpu=virtio keyboard=virtio serial_only=true drm_render_node=not_tested input_seat=not_tested'
