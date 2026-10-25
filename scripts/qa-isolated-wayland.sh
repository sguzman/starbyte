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
[[ $# -eq 1 && ( "$mode" == --check-only || "$mode" == --weston-smoke || "$mode" == --hyprland-headless-smoke || "$mode" == --hyprland-nested-smoke ) ]] \
  || fail 'usage: --check-only|--weston-smoke|--hyprland-headless-smoke|--hyprland-nested-smoke'

# Aquamarine v0.15.1 cannot construct a Wayland render allocator without
# a DRM render node, even when a parent Wayland compositor is available.
# Our namespace deliberately exposes no /dev/dri at all. Fail before
# resolving binaries, creating a runtime, or launching ANY compositor.
# Never weaken the device boundary to make a nested probe appear successful.
if [[ "$mode" == --hyprland-nested-smoke ]]; then
  fail 'nested_hyprland_requires_private_drm_allocator'
fi
if [[ "$mode" == --hyprland-headless-smoke ]]; then
  fail 'headless_hyprland_requires_private_drm_allocator'
fi
command -v bwrap >/dev/null 2>&1 || fail 'missing_bubblewrap'
command -v timeout >/dev/null 2>&1 || fail 'missing_timeout'
command -v realpath >/dev/null 2>&1 || fail 'missing_realpath'
[[ "$(uname -s)" == Linux ]] || fail 'linux_required'
[[ -n "${XDG_RUNTIME_DIR:-}" && -d "$XDG_RUNTIME_DIR" ]] || fail 'private_runtime_parent_required'

weston_bindings=()
weston_bin=''
weston_client=''
wayland_info=''
package_root=''
weston_module_map=''
if [[ "$mode" == --weston-smoke ]]; then
  if [[ -n "${STARBYTE_WESTON_PACKAGE_ROOT:-}" ]]; then
    package_root="$(realpath -e -- "$STARBYTE_WESTON_PACKAGE_ROOT")" \
      || fail 'weston_package_root_missing'
    [[ -x "$package_root/usr/bin/weston" ]] || fail 'weston_package_binary_missing'
    [[ -x "$package_root/usr/bin/weston-simple-shm" ]] || fail 'weston_client_binary_missing'
    [[ -x "$package_root/usr/bin/wayland-info" ]] || fail 'wayland_info_client_missing'
    [[ -d "$package_root/usr/lib/weston" && -d "$package_root/usr/lib/libweston-15" ]] \
      || fail 'weston_package_modules_missing'
    [[ -e "$package_root/usr/lib/libweston-15.so.0" ]] || fail 'weston_package_library_missing'
    weston_bin=/run/user/qa/package/usr/bin/weston
    weston_client=/run/user/qa/package/usr/bin/weston-simple-shm
    wayland_info=/run/user/qa/package/usr/bin/wayland-info
    weston_module_map='libexec_weston.so.0=/run/user/qa/package/usr/lib/weston/libexec_weston.so.0.0.0;headless-backend.so=/run/user/qa/package/usr/lib/libweston-15/headless-backend.so;kiosk-shell.so=/run/user/qa/package/usr/lib/weston/kiosk-shell.so'
  else
    command -v weston >/dev/null 2>&1 || fail 'weston_not_installed'
    command -v weston-simple-shm >/dev/null 2>&1 || fail 'weston_simple_shm_not_installed'
    command -v wayland-info >/dev/null 2>&1 || fail 'wayland_info_client_not_installed'
    weston_bin="$(realpath -e -- "$(command -v weston)")"
    weston_client="$(realpath -e -- "$(command -v weston-simple-shm)")"
    wayland_info="$(realpath -e -- "$(command -v wayland-info)")"
    [[ "$weston_bin" == /usr/* && "$weston_client" == /usr/* && "$wayland_info" == /usr/* ]] \
      || fail 'weston_binaries_must_be_under_usr'
  fi
fi
runtime_root="$(mktemp -d "$XDG_RUNTIME_DIR/starbyte-isolated-qa.XXXXXXXX")" \
  || fail 'cannot_create_private_runtime'
chmod 700 "$runtime_root"
cleanup() { rm -rf -- "$runtime_root"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
if [[ -n "$package_root" ]]; then
  mkdir -p "$runtime_root/package"
  weston_bindings=(--ro-bind "$package_root" /run/user/qa/package)
fi

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
  --setenv STARBYTE_HOST_RUNTIME_DIR "$XDG_RUNTIME_DIR"
  --setenv STARBYTE_HOST_CONFIG_HOME "${XDG_CONFIG_HOME:-$HOME/.config}"
  --setenv STARBYTE_HOST_WAYLAND_SOCKET "${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY:-wayland-0}"
  --setenv STARBYTE_HOST_DBUS_SOCKET "$XDG_RUNTIME_DIR/bus"
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
    test ! -e "$STARBYTE_HOST_RUNTIME_DIR"
    test ! -e "$STARBYTE_HOST_RUNTIME_DIR/hypr"
    test ! -e "$STARBYTE_HOST_CONFIG_HOME"
    test ! -e "$STARBYTE_HOST_WAYLAND_SOCKET"
    test ! -e "$STARBYTE_HOST_DBUS_SOCKET"
    test ! -e /dev/input
    test ! -e /dev/dri
    test ! -e /run/user/qa/bus
    test ! -e /run/user/qa/systemd
    test ! -e /run/dbus/system_bus_socket
    test -z "${DBUS_SESSION_BUS_ADDRESS-}"
    test -z "${HYPRLAND_INSTANCE_SIGNATURE-}"
    test -z "${XDG_SESSION_ID-}"
    test -z "${SYSTEMD_EXEC_PID-}"
    test ! -e /tmp/.X11-unix
    test "$HOME" = /home/qa
    echo PRIVATE_NAMESPACE_VIEW_OK
  ' || fail 'bubblewrap_isolation_check_failed'
  printf 'STARBYTE_ISOLATED_WAYLAND_CHECK_OK display_fallback=disabled host_sockets=hidden devices=hidden\n'
  exit 0
fi

weston_environment=(
  --setenv STARBYTE_WESTON_BIN "$weston_bin"
  --setenv STARBYTE_WESTON_CLIENT "$weston_client"
  --setenv STARBYTE_WAYLAND_INFO "$wayland_info"
)
if [[ -n "$weston_module_map" ]]; then
  weston_environment+=(
    --setenv WESTON_MODULE_MAP "$weston_module_map"
    --setenv LD_LIBRARY_PATH /run/user/qa/package/usr/lib:/run/user/qa/package/usr/lib/weston
    --setenv WESTON_DATA_DIR /run/user/qa/package/usr/share/weston
  )
fi

# The compositor and client share only this namespace and socket. They have no
# host input/DRM devices, system/session sockets, host DISPLAY, or user config.
bwrap "${common[@]}" "${weston_bindings[@]}" "${weston_environment[@]}" \
  /usr/bin/bash -c '
    set -Eeuo pipefail
    : >"$XDG_RUNTIME_DIR/weston.log"
    : >"$XDG_RUNTIME_DIR/client.log"
    : >"$XDG_RUNTIME_DIR/wayland-info.log"
    weston_failure_reason() {
      if grep -qi "Permission denied" "$XDG_RUNTIME_DIR/weston.log"; then
        echo permission_denied
      elif grep -qi "No such file" "$XDG_RUNTIME_DIR/weston.log"; then
        echo missing_runtime_component
      elif grep -qi "failed\|error" "$XDG_RUNTIME_DIR/weston.log"; then
        echo compositor_reported_error
      else
        echo compositor_exited
      fi
    }
    "$STARBYTE_WESTON_BIN" --backend=headless --renderer=pixman --shell=kiosk-shell.so --no-config \
      --socket="$WAYLAND_DISPLAY" --idle-time=0 \
      --log="$XDG_RUNTIME_DIR/weston.log" &
    weston_pid=$!
    cleanup() {
      kill -TERM "$weston_pid" 2>/dev/null || true
      for _ in {1..40}; do
        weston_state=''
        if [[ -r "/proc/$weston_pid/stat" ]]; then
          read -r _ _ weston_state _ <"/proc/$weston_pid/stat" || true
        fi
        [[ -z "$weston_state" || "$weston_state" == Z ]] && break
        sleep 0.05
      done
      if [[ -n "$weston_state" && "$weston_state" != Z ]]; then
        kill -KILL "$weston_pid" 2>/dev/null || true
      fi
      wait "$weston_pid" 2>/dev/null || true
    }
    trap cleanup EXIT
    trap "exit 130" INT
    trap "exit 143" TERM
    trap "exit 129" HUP
    for _ in {1..100}; do
      kill -0 "$weston_pid" 2>/dev/null || {
        printf "PRIVATE_WESTON_START_FAILED reason=%s\n" "$(weston_failure_reason)" >&2
        exit 1
      }
      [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && break
      sleep 0.05
    done
    [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || {
      printf "PRIVATE_WESTON_START_FAILED reason=%s\n" "$(weston_failure_reason)" >&2
      exit 1
    }
    "$STARBYTE_WAYLAND_INFO" >"$XDG_RUNTIME_DIR/wayland-info.log" 2>&1 || {
      echo PRIVATE_WAYLAND_INFO_FAILED >&2
      exit 1
    }
    grep -q "wl_compositor" "$XDG_RUNTIME_DIR/wayland-info.log" || {
      echo PRIVATE_WAYLAND_INFO_NO_COMPOSITOR_GLOBAL >&2
      exit 1
    }
    echo PRIVATE_WESTON_CLIENT_CONNECTED client=wayland-info
    set +e
    timeout --signal=TERM --kill-after=1s 2s "$STARBYTE_WESTON_CLIENT" >"$XDG_RUNTIME_DIR/client.log" 2>&1
    client_status=$?
    set -e
    [[ "$client_status" -eq 0 || "$client_status" -eq 124 ]] || {
      echo "PRIVATE_WESTON_CLIENT_EXIT status=$client_status" >&2
      exit 1
    }
    echo "PRIVATE_WESTON_SIMPLE_SHM_RAN status=$client_status"
    cleanup
    trap - EXIT INT TERM HUP
    [[ ! -e "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] || {
      echo PRIVATE_WESTON_SOCKET_STILL_PRESENT >&2
      exit 1
    }
    echo PRIVATE_WESTON_TORN_DOWN
  '
printf 'STARBYTE_ISOLATED_WESTON_SMOKE_OK backend=headless renderer=pixman client=weston-simple-shm\n'
