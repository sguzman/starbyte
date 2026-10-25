#!/usr/bin/env bash
# Private Weston -> nested Hyprland backend/IPC probe. Invoke ONLY via the
# Bubblewrap supervisor: it cannot connect to the owner's compositor or seat.
set -Eeuo pipefail
umask 077

blocked() {
  printf 'STARBYTE_NESTED_HYPRLAND_BLOCKED reason=%s\n' "$1" >&2
  exit 4
}
[[ "${XDG_RUNTIME_DIR:-}" == /run/user/qa ]] || blocked 'outside_private_runtime'
[[ "${WAYLAND_DISPLAY:-}" == starbyte-qa-wayland ]] || blocked 'wrong_parent_wayland_socket'
[[ -z "${DISPLAY:-}" && -z "${WAYLAND_SOCKET:-}" ]] || blocked 'host_display_environment'
[[ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]] || blocked 'inherited_compositor_instance'
[[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]] || blocked 'inherited_session_bus'
[[ -z "${XDG_SESSION_ID:-}" && -z "${SYSTEMD_EXEC_PID:-}" ]] || blocked 'inherited_session_manager'
[[ ! -e /dev/input && ! -e /dev/dri ]] || blocked 'host_input_or_drm_device_visible'
[[ ! -e /run/user/qa/bus && ! -e /run/user/qa/systemd ]] || blocked 'host_session_bus_visible'
[[ ! -e /tmp/.X11-unix ]] || blocked 'host_x11_socket_visible'
[[ -x "${STARBYTE_WESTON_BIN:-}" && -x "${STARBYTE_HYPRLAND_BIN:-}" ]] || blocked 'compositor_missing'
[[ -x "${STARBYTE_WAYLAND_INFO:-}" && -x "${STARBYTE_HYPRCTL_BIN:-}" ]] || blocked 'ipc_tools_missing'
command -v jq >/dev/null 2>&1 || blocked 'jq_required_for_private_ipc_verification'

weston_pid=''
hypr_pid=''
stop_child() {
  local pid="$1"
  [[ -n "$pid" ]] || return 0
  kill -TERM "$pid" 2>/dev/null || true
  local state=''
  for _ in {1..40}; do
    state=''
    if [[ -r "/proc/$pid/stat" ]]; then
      read -r _ _ state _ <"/proc/$pid/stat" || true
    fi
    [[ -z "$state" || "$state" == Z ]] && break
    sleep 0.05
  done
  if [[ -n "$state" && "$state" != Z ]]; then
    kill -KILL "$pid" 2>/dev/null || true
  fi
  wait "$pid" 2>/dev/null || true
}
cleanup() {
  stop_child "$hypr_pid"
  stop_child "$weston_pid"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

"$STARBYTE_WESTON_BIN" --backend=headless --renderer=pixman --shell=kiosk-shell.so --no-config \
  --socket="$WAYLAND_DISPLAY" --idle-time=0 --log="$XDG_RUNTIME_DIR/weston.log" &
weston_pid=$!
for _ in {1..100}; do
  kill -0 "$weston_pid" 2>/dev/null || blocked 'private_weston_early_exit'
  [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
  sleep 0.05
done
[[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || blocked 'private_weston_timeout'
"$STARBYTE_WAYLAND_INFO" >"$XDG_RUNTIME_DIR/wayland-info.log" 2>&1 \
  || blocked 'private_weston_unreachable'
grep -q wl_compositor "$XDG_RUNTIME_DIR/wayland-info.log" \
  || blocked 'private_weston_missing_compositor'
printf 'STARBYTE_NESTED_PARENT_OK backend=weston_headless renderer=pixman\n'

# Hyprland 0.55+ uses Lua. Do not read the owner's existing config or
# change any systemd activation environment. XWayland remains disabled.
cat >"$XDG_RUNTIME_DIR/hyprland.lua" <<'LUA'
hl.config({
  misc = { disable_hyprland_guiutils_check = true },
  xwayland = { enabled = false },
})
LUA
"$STARBYTE_HYPRLAND_BIN" --config "$XDG_RUNTIME_DIR/hyprland.lua" \
  >"$XDG_RUNTIME_DIR/hyprland.log" 2>&1 &
hypr_pid=$!
instance_dir=''
for _ in {1..160}; do
  kill -0 "$hypr_pid" 2>/dev/null || blocked 'nested_hyprland_early_exit'
  if [[ -d "$XDG_RUNTIME_DIR/hypr" ]]; then
    for candidate in "$XDG_RUNTIME_DIR"/hypr/*; do
      if [[ -d "$candidate" && -S "$candidate/.socket.sock" ]]; then
        instance_dir="$candidate"
        break
      fi
    done
  fi
  [[ -n "$instance_dir" ]] && break
  sleep 0.05
done
[[ -n "$instance_dir" ]] || blocked 'nested_hyprland_start_timeout'
[[ -r "$instance_dir/hyprland.lock" ]] || blocked 'nested_instance_identity_missing'
{
  read -r instance_pid
  read -r instance_wayland_socket
} <"$instance_dir/hyprland.lock"
[[ "$instance_pid" == "$hypr_pid" ]] || blocked 'nested_process_identity_mismatch'
[[ -S "$XDG_RUNTIME_DIR/$instance_wayland_socket" ]] || blocked 'nested_wayland_socket_missing'

# A socket alone cannot establish the selected rendering backend. Fail
# closed if the startup log does not explicitly identify Wayland.
grep -Eqi 'wayland.{0,80}backend|backend.{0,80}wayland' "$XDG_RUNTIME_DIR/hyprland.log" \
  || blocked 'nested_wayland_backend_unverified'
export HYPRLAND_INSTANCE_SIGNATURE="${instance_dir##*/}"
"$STARBYTE_HYPRCTL_BIN" -j monitors all >"$XDG_RUNTIME_DIR/nested-monitors.json" 2>&1 \
  || blocked 'private_hyprland_ipc_failed'
jq -e 'type == "array" and length > 0' "$XDG_RUNTIME_DIR/nested-monitors.json" >/dev/null \
  || blocked 'nested_hyprland_output_missing'
printf 'STARBYTE_NESTED_HYPRLAND_READY backend=wayland private_ipc=reachable private_socket=present\n'
"$STARBYTE_HYPRCTL_BIN" dispatch exit >/dev/null 2>&1 \
  || blocked 'private_hyprland_exit_failed'
cleanup
trap - EXIT INT TERM HUP
[[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || blocked 'weston_socket_remains'
[[ ! -S "$instance_dir/.socket.sock" ]] || blocked 'hyprland_ipc_socket_remains'
printf 'STARBYTE_NESTED_HYPRLAND_TORN_DOWN children=reaped sockets=gone\n'
