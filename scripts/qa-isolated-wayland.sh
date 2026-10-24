#!/usr/bin/env bash
# Private, software-rendered Wayland infrastructure probe. This never connects
# to the owner's compositor and does not authorize the physical QA collector.
set -Eeuo pipefail
umask 077

fail() {
  printf 'STARBYTE_ISOLATED_WAYLAND_BLOCKED %s\n' "$1" >&2
  exit 3
}

mode="${1:-}"
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --weston-smoke ) ]] \
  || fail 'usage: --check-only|--weston-smoke'
command -v bwrap >/dev/null 2>&1 || fail 'missing_bubblewrap'
command -v timeout >/dev/null 2>&1 || fail 'missing_timeout'
[[ "$(uname -s)" == Linux ]] || fail 'linux_required'
[[ -n "${XDG_RUNTIME_DIR:-}" && -d "$XDG_RUNTIME_DIR" ]] || fail 'private_runtime_parent_required'

if [[ "$mode" == --weston-smoke ]]; then
  command -v weston >/dev/null 2>&1 || fail 'weston_not_installed'
  command -v weston-simple-shm >/dev/null 2>&1 || fail 'weston_simple_shm_not_installed'
fi

runtime_root="$(mktemp -d "$XDG_RUNTIME_DIR/starbyte-isolated-qa.XXXXXXXX")" \
  || fail 'cannot_create_private_runtime'
chmod 700 "$runtime_root"
cleanup() { rm -rf -- "$runtime_root"; }
trap cleanup EXIT INT TERM HUP

common=(
  --unshare-all --die-with-parent --new-session
  --ro-bind /usr /usr
  --symlink usr/lib /lib --symlink usr/lib /lib64
  --proc /proc --dev /dev
  --tmpfs /tmp --tmpfs /run --dir /run/user
  --bind "$runtime_root" /run/user/qa
  --dir /home --dir /home/qa
  --clearenv --setenv PATH /usr/bin
  --setenv HOME /home/qa
  --setenv XDG_CONFIG_HOME /home/qa/.config
  --setenv XDG_CACHE_HOME /home/qa/.cache
  --setenv XDG_RUNTIME_DIR /run/user/qa
  --setenv WAYLAND_DISPLAY starbyte-qa-wayland
  --unsetenv DISPLAY
  --unsetenv WAYLAND_SOCKET
)

if [[ "$mode" == --check-only ]]; then
  bwrap "${common[@]}" /usr/bin/bash -c '
    test "$XDG_RUNTIME_DIR" = /run/user/qa
    test "$WAYLAND_DISPLAY" = starbyte-qa-wayland
    test -z "${DISPLAY-}"
    test -z "${WAYLAND_SOCKET-}"
    test ! -e "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
    test ! -e /dev/input
    test ! -e /dev/dri
    test ! -e /run/user/qa/bus
    test ! -e /run/user/qa/systemd
    test ! -e /tmp/.X11-unix
    test "$HOME" = /home/qa
    echo PRIVATE_NAMESPACE_VIEW_OK
  ' || fail 'bubblewrap_isolation_check_failed'
  printf 'STARBYTE_ISOLATED_WAYLAND_CHECK_OK display_fallback=disabled host_sockets=hidden devices=hidden\n'
  exit 0
fi

# The compositor and client share only this namespace and socket. They have no
# host input/DRM devices, system/session sockets, host DISPLAY, or user config.
bwrap "${common[@]}" \
  --ro-bind "$(command -v weston)" /usr/bin/weston \
  --ro-bind "$(command -v weston-simple-shm)" /usr/bin/weston-simple-shm \
  /usr/bin/bash -c '
    set -Eeuo pipefail
    weston --backend=headless --renderer=pixman --no-config \
      --socket="$WAYLAND_DISPLAY" --idle-time=0 \
      --log="$XDG_RUNTIME_DIR/weston.log" &
    weston_pid=$!
    cleanup() { kill "$weston_pid" 2>/dev/null || true; wait "$weston_pid" 2>/dev/null || true; }
    trap cleanup EXIT INT TERM HUP
    for _ in {1..100}; do
      kill -0 "$weston_pid" 2>/dev/null || { cat "$XDG_RUNTIME_DIR/weston.log" >&2; exit 1; }
      [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
      sleep 0.05
    done
    [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || { cat "$XDG_RUNTIME_DIR/weston.log" >&2; exit 1; }
    set +e
    timeout --signal=TERM 2s env WAYLAND_DEBUG=1 weston-simple-shm >"$XDG_RUNTIME_DIR/client.log" 2>&1
    client_status=$?
    set -e
    [[ "$client_status" -eq 0 || "$client_status" -eq 124 ]] || {
      cat "$XDG_RUNTIME_DIR/client.log" >&2
      exit 1
    }
    grep -q "wl_display@.*get_registry" "$XDG_RUNTIME_DIR/client.log"
    echo PRIVATE_WESTON_CLIENT_CONNECTED
    cleanup
    trap - EXIT INT TERM HUP
    [[ ! -e "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]
    echo PRIVATE_WESTON_TORN_DOWN
  '
printf 'STARBYTE_ISOLATED_WESTON_SMOKE_OK backend=headless renderer=pixman client=weston-simple-shm\n'
