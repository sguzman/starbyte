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
hyprland_bin=''
hyprctl_bin=''
if [[ "$mode" == --weston-smoke || "$mode" == --hyprland-nested-smoke ]]; then
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
if [[ "$mode" == --hyprland-headless-smoke || "$mode" == --hyprland-nested-smoke ]]; then
  command -v Hyprland >/dev/null 2>&1 || fail 'hyprland_not_installed'
  command -v hyprctl >/dev/null 2>&1 || fail 'hyprctl_not_installed'
  hyprland_bin="$(realpath -e -- "$(command -v Hyprland)")"
  hyprctl_bin="$(realpath -e -- "$(command -v hyprctl)")"
  [[ "$hyprland_bin" == /usr/* && "$hyprctl_bin" == /usr/* ]] \
    || fail 'hyprland_binaries_must_be_under_usr'
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

if [[ "$mode" == --hyprland-headless-smoke ]]; then
  set +e
  bwrap "${common[@]}" \
    --setenv STARBYTE_HYPRLAND_BIN "$hyprland_bin" \
    --setenv STARBYTE_HYPRCTL_BIN "$hyprctl_bin" \
    --setenv HYPRLAND_NO_SD_VARS 1 \
    --setenv HYPRLAND_NO_SD_TARGET 1 \
    --setenv HYPRLAND_NO_RT 1 \
    --setenv HYPRLAND_NO_CRASHREPORTER 1 \
    --setenv LIBGL_ALWAYS_SOFTWARE 1 \
    --setenv MESA_LOADER_DRIVER_OVERRIDE llvmpipe \
    /usr/bin/bash -c '
      set -Eeuo pipefail
      config="$XDG_RUNTIME_DIR/hyprland.conf"
      log="$XDG_RUNTIME_DIR/hyprland.log"
      cat >"$config" <<"EOF"
misc {
  disable_hyprland_guiutils_check = true
}
xwayland {
  enabled = false
}
EOF
      : >"$log"
      "$STARBYTE_HYPRLAND_BIN" --config "$config" >"$log" 2>&1 &
      hyprland_pid=$!
      cleanup() {
        kill -TERM "$hyprland_pid" 2>/dev/null || true
        for _ in {1..40}; do
          state=""
          if [[ -r "/proc/$hyprland_pid/stat" ]]; then
            read -r _ _ state _ <"/proc/$hyprland_pid/stat" || true
          fi
          [[ -z "$state" || "$state" == Z ]] && break
          sleep 0.05
        done
        if [[ -n "$state" && "$state" != Z ]]; then
          kill -KILL "$hyprland_pid" 2>/dev/null || true
        fi
        wait "$hyprland_pid" 2>/dev/null || true
      }
      trap cleanup EXIT
      trap "exit 130" INT
      trap "exit 143" TERM
      trap "exit 129" HUP
      for _ in {1..100}; do
        if ! kill -0 "$hyprland_pid" 2>/dev/null; then
          set +e
          wait "$hyprland_pid"
          status=$?
          set -e
          if grep -qi "Cannot open backend: no allocator available" "$log"; then
            echo STARBYTE_HEADLESS_HYPRLAND_BLOCKED reason=no_render_allocator
          else
            printf "STARBYTE_HEADLESS_HYPRLAND_START_FAILED status=%s reason=early_exit\n" "$status"
          fi
          cleanup
          trap - EXIT INT TERM HUP
          if [[ -d "$XDG_RUNTIME_DIR/hypr" ]] && find "$XDG_RUNTIME_DIR/hypr" -type s -print -quit | grep -q .; then
            echo STARBYTE_HEADLESS_HYPRLAND_FAILURE_SOCKET_REMAINS
            exit 6
          fi
          echo STARBYTE_HEADLESS_HYPRLAND_FAILURE_CLEANUP_OK
          exit 4
        fi
        instance_dir=""
        if [[ -d "$XDG_RUNTIME_DIR/hypr" ]]; then
          for candidate in "$XDG_RUNTIME_DIR"/hypr/*; do
            [[ -d "$candidate" ]] && instance_dir="$candidate" && break
          done
        fi
        if [[ -n "$instance_dir" && -S "$instance_dir/.socket.sock" ]]; then
          signature="${instance_dir##*/}"
          [[ -r "$instance_dir/hyprland.lock" ]] || {
            echo STARBYTE_HEADLESS_HYPRLAND_IPC_IDENTITY_MISSING
            exit 5
          }
          {
            read -r instance_pid
            read -r instance_wayland_socket
          } <"$instance_dir/hyprland.lock"
          [[ "$instance_pid" == "$hyprland_pid" && -S "$XDG_RUNTIME_DIR/$instance_wayland_socket" ]] || {
            echo STARBYTE_HEADLESS_HYPRLAND_PRIVATE_SOCKET_MISMATCH
            exit 5
          }
          export HYPRLAND_INSTANCE_SIGNATURE="$signature"
          monitors="$("$STARBYTE_HYPRCTL_BIN" -j monitors all 2>/dev/null)" || {
            echo STARBYTE_HEADLESS_HYPRLAND_IPC_FAILED
            exit 5
          }
          printf "%s" "$monitors" | /usr/bin/jq -e "any(.[]; .name == \"HEADLESS-0\")" >/dev/null || {
            echo STARBYTE_HEADLESS_HYPRLAND_OUTPUT_MISSING
            exit 5
          }
          grep -Eq "implementation headless|backend.*headless|Headless" "$log" || {
            echo STARBYTE_HEADLESS_HYPRLAND_BACKEND_UNVERIFIED
            exit 5
          }
          echo STARBYTE_HEADLESS_HYPRLAND_STARTED backend=headless output=HEADLESS-0 private_wayland_socket=present private_ipc=reachable
          "$STARBYTE_HYPRCTL_BIN" dispatch exit >/dev/null
          wait "$hyprland_pid"
          cleanup
          trap - EXIT INT TERM HUP
          [[ ! -e "$XDG_RUNTIME_DIR/hypr" ]] || {
            echo STARBYTE_HEADLESS_HYPRLAND_SOCKET_REMAINS
            exit 6
          }
          echo STARBYTE_HEADLESS_HYPRLAND_TORN_DOWN
          exit 0
        fi
        sleep 0.05
      done
      echo STARBYTE_HEADLESS_HYPRLAND_START_TIMEOUT
      cleanup
      trap - EXIT INT TERM HUP
      if [[ -d "$XDG_RUNTIME_DIR/hypr" ]] && find "$XDG_RUNTIME_DIR/hypr" -type s -print -quit | grep -q .; then
        echo STARBYTE_HEADLESS_HYPRLAND_TIMEOUT_SOCKET_REMAINS
        exit 6
      fi
      echo STARBYTE_HEADLESS_HYPRLAND_TIMEOUT_CLEANUP_OK
      exit 124
    '
  result=$?
  set -e
  if [[ "$result" -eq 4 ]]; then
    exit 4
  fi
  [[ "$result" -eq 0 ]] || fail "hyprland_headless_smoke_failed status=$result"
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

if [[ "$mode" == --hyprland-nested-smoke ]]; then
  # Mount only this reviewed supervisor as a read-only file. No host config,
  # display socket, input device, DRM node or audio route enters the sandbox.
  nested_supervisor="$(realpath -e -- "$(dirname -- "${BASH_SOURCE[0]}")/qa-hyprland-nested-inside.sh")" \
    || fail 'nested_supervisor_missing'
  : >"$runtime_root/nested-supervisor"
  timeout --signal=TERM --kill-after=2s 20s \
    bwrap "${common[@]}" "${weston_bindings[@]}" "${weston_environment[@]}" \
      --ro-bind "$nested_supervisor" /run/user/qa/nested-supervisor \
      --setenv STARBYTE_HYPRLAND_BIN "$hyprland_bin" \
      --setenv STARBYTE_HYPRCTL_BIN "$hyprctl_bin" \
      --setenv HYPRLAND_NO_SD_VARS 1 \
      --setenv HYPRLAND_NO_SD_TARGET 1 \
      --setenv HYPRLAND_NO_RT 1 \
      --setenv HYPRLAND_NO_CRASHREPORTER 1 \
      --setenv LIBGL_ALWAYS_SOFTWARE 1 \
      --setenv MESA_LOADER_DRIVER_OVERRIDE llvmpipe \
      /usr/bin/bash /run/user/qa/nested-supervisor \
    || fail 'nested_hyprland_smoke_unverified_or_failed'
  printf 'STARBYTE_ISOLATED_NESTED_HYPRLAND_SMOKE_OK backend=wayland private_ipc=verified input_seat=not_tested\n'
  exit 0
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
