#!/usr/bin/env bash
# Bootless virtual GPU/keyboard QMP probe. Never uses the owner's desktop.
set -Eeuo pipefail
umask 077
fail() {
  printf 'STARBYTE_VIRTUAL_DEVICES_BLOCKED %s\n' "$1" >&2
  exit 3
}
mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --smoke ) ]] || fail 'usage: --check-only|--smoke'

# Only execute on ephemeral GitHub-hosted Linux runners, never the owner's
# desktop or a self-hosted runner connected to that desktop.
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted ]] || fail 'github_hosted_runner_only'
[[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" && -z "${WAYLAND_SOCKET:-}" ]] || fail 'graphical_session_present'
[[ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" && -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || fail 'desktop_session_present'
[[ -z "${XDG_SESSION_ID:-}" ]] || fail 'interactive_session_present'
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail 'linux_x86_64_required'

if [[ "$mode" == --check-only ]]; then
  echo 'STARBYTE_VIRTUAL_DEVICE_PREFLIGHT_OK environment=github_hosted display=none'
  exit 0
fi
for executable in qemu-system-x86_64 socat jq timeout; do
  command -v "$executable" >/dev/null 2>&1 || fail "missing_$executable"
done
root="$(mktemp -d "${TMPDIR:-/tmp}/starbyte-vgpu.XXXXXXXX")" || fail 'private_runtime_creation_failed'
chmod 700 "$root"
mkdir -p "$root/home"
qemu_pid=''
cleanup() {
  if [[ -n "$qemu_pid" ]]; then
    kill -TERM "$qemu_pid" 2>/dev/null || true
    wait "$qemu_pid" 2>/dev/null || true
  fi
  rm -rf -- "$root"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# QEMU runs under software TCG with CPUs paused (-S) and no guest disk.
# No host Wayland/X11, graphics window, input forwarding, KVM or networking.
HOME="$root/home" XDG_CONFIG_HOME="$root/home" \
  timeout --signal=TERM --kill-after=2s 15s qemu-system-x86_64 \
    -machine q35,accel=tcg -m 192M -smp 1 -S \
    -nodefaults -display none -vga none -net none \
    -monitor none -serial none -parallel none \
    -device virtio-gpu-pci,id=sb_gpu \
    -device virtio-keyboard-pci,id=sb_keyboard \
    -qmp "unix:$root/qmp.sock,server=on,wait=off" \
    >"$root/qemu.log" 2>&1 &
qemu_pid=$!
for _ in {1..80}; do
  kill -0 "$qemu_pid" 2>/dev/null || fail 'qemu_early_exit'
  [[ -S "$root/qmp.sock" ]] && break
  sleep 0.1
done
[[ -S "$root/qmp.sock" ]] || fail 'qmp_socket_not_ready'
# QMP quit may close the socket before socat finishes reading; socat then
# returns 1 (ECONNRESET) even if complete QMP acknowledgments were delivered.
# Treat only statuses 0 or 1 as eligible, and require both actual structured
# acknowledgments below. All timeouts and other transport failures remain fatal.
set +e
{
  printf '%s\n' '{"execute":"qmp_capabilities","id":"cap"}'
  printf '%s\n' '{"execute":"query-pci","id":"pci"}'
  printf '%s\n' '{"execute":"quit","id":"quit"}'
} | timeout --signal=TERM --kill-after=1s 4s \
    socat -T 2 STDIO "UNIX-CONNECT:$root/qmp.sock" \
    >"$root/qmp.jsonl" 2>"$root/qmp.err"
qmp_status=$?
set -e
[[ "$qmp_status" -eq 0 || "$qmp_status" -eq 1 ]]   || fail 'qmp_exchange_failed_or_timed_out'

jq -es 'any(.[]; .id == "cap" and (.return == {}))' "$root/qmp.jsonl" >/dev/null \
  || fail 'qmp_capability_ack_missing'
jq -es '
  [.[] | select(.id == "pci") | .return | .. | objects | .qdev_id? | select(. != null)] as $ids |
  ($ids | index("sb_gpu") != null) and ($ids | index("sb_keyboard") != null)
' "$root/qmp.jsonl" >/dev/null || fail 'virtual_gpu_or_keyboard_missing_from_pci'

wait "$qemu_pid" || fail 'qemu_exit_failed'
qemu_pid=''
[[ ! -S "$root/qmp.sock" ]] || fail 'qmp_socket_remains'
echo 'STARBYTE_VIRTUAL_DEVICE_MODELS_OK gpu=virtio-gpu-pci keyboard=virtio-keyboard-pci display=none network=none guest_booted=false'
