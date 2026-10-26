#!/usr/bin/env bash
# Private native-Wayland focus probe, never the owner's desktop or input devices.
set -Eeuo pipefail
umask 077
fail() { printf 'STARBYTE_HOSTED_FOCUS_BLOCKED %s\n' "$1" >&2; exit 3; }
[[ $# -eq 1 && ( "$1" == --smoke || "$1" == --blur-smoke ) ]] || fail usage
probe="$1"
[[ "$GITHUB_ACTIONS" == true && "$RUNNER_ENVIRONMENT" == github-hosted ]] || fail hosted_only
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail linux_only
[[ ! -v DISPLAY && ! -v WAYLAND_DISPLAY && ! -v HYPRLAND_INSTANCE_SIGNATURE ]] || fail display_present
[[ ! -v DBUS_SESSION_BUS_ADDRESS && ! -v XDG_SESSION_ID ]] || fail session_present
[[ -d "$RUNNER_TEMP" ]] || fail temp_missing
for binary in bwrap timeout realpath Xvfb xdotool xmessage xkbcomp weston wayland-info git; do
  command -v "$binary" >/dev/null || fail "missing_$binary"
done
repo="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || fail checkout_missing
game="$(realpath -e "$repo/target/debug/starbyte-egui")" || fail binary_missing
[[ "$game" == "$repo/"* && -x "$game" ]] || fail binary_outside_checkout
root="$(mktemp -d "$RUNNER_TEMP/starbyte-seat.XXXXXXXX")" || fail private_root_failed
chmod 700 "$root"
trap 'rm -rf -- "$root"' EXIT
install -m 0700 "$game" "$root/starbyte-egui"
cat >"$root/guest.sh" <<'GUEST'
#!/usr/bin/env bash
set -Eeuo pipefail
runtime=/run/user/qa
[[ -x /bin/sh && -x /usr/bin/xkbcomp ]] || {
  echo STARBYTE_FOCUSED_SEAT_FAILED private_shell_or_xkb_compiler_missing >&2
  exit 1
}
[[ ! -e /dev/input && ! -e /dev/dri && ! -e /tmp/.X11-unix ]] || {
  echo STARBYTE_FOCUSED_SEAT_FAILED host_device_or_socket_visible >&2
  exit 1
}
mkdir -p /home/qa/.config /home/qa/.cache /tmp/.X11-unix
[[ -d /var/lib/xkb && -w /var/lib/xkb ]] || {
  echo STARBYTE_FOCUSED_SEAT_FAILED private_xkb_cache_unavailable >&2
  exit 1
}
xvfb_pid='' weston_pid='' game_pid='' rival_pid=''
cleanup() {
  [[ -z "$rival_pid" ]] || { kill "$rival_pid" 2>/dev/null || :; wait "$rival_pid" 2>/dev/null || :; }
  [[ -z "$game_pid" ]] || { kill "$game_pid" 2>/dev/null || :; wait "$game_pid" 2>/dev/null || :; }
  [[ -z "$weston_pid" ]] || { kill "$weston_pid" 2>/dev/null || :; wait "$weston_pid" 2>/dev/null || :; }
  [[ -z "$xvfb_pid" ]] || { kill "$xvfb_pid" 2>/dev/null || :; wait "$xvfb_pid" 2>/dev/null || :; }
}
trap cleanup EXIT
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp -ac >"$runtime/xvfb.log" 2>&1 &
xvfb_pid=$!
for _ in {1..70}; do
  [[ -S /tmp/.X11-unix/X99 ]] && break
  kill -0 "$xvfb_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -S /tmp/.X11-unix/X99 ]] || { tail -n 50 "$runtime/xvfb.log" >&2; exit 1; }
DISPLAY=:99 weston --backend=x11 --renderer=pixman --shell=kiosk-shell.so \
  --no-config --socket="$WAYLAND_DISPLAY" --idle-time=0 >"$runtime/weston.log" 2>&1 &
weston_pid=$!
for _ in {1..100}; do
  [[ -S "$runtime/$WAYLAND_DISPLAY" ]] && break
  kill -0 "$weston_pid" 2>/dev/null || break
  sleep 0.1
done
[[ -S "$runtime/$WAYLAND_DISPLAY" ]] || { tail -n 65 "$runtime/weston.log" >&2; exit 1; }
timeout 5s wayland-info >"$runtime/info.log" 2>&1 || { cat "$runtime/info.log" >&2; exit 1; }
grep -Fq wl_seat "$runtime/info.log" || { echo STARBYTE_FOCUSED_SEAT_FAILED virtual_seat_missing >&2; exit 1; }
echo STARBYTE_PRIVATE_SEAT_CONNECTED input=xvfb host_devices=hidden
game_args=(--demo --native-worker-preview --exit-after-full-worker-focused-clock)
if [[ "$STARBYTE_FOCUS_PROBE" == --blur-smoke ]]; then
  game_args=(--demo --native-worker-preview --exit-after-full-worker-physical-pause)
fi
timeout --signal=TERM --kill-after=2s 18s /run/user/qa/starbyte-egui \
  "${game_args[@]}" --config /home/qa/.config/starbyte.toml \
  --cache-dir /home/qa/.cache/starbyte >"$runtime/game.log" 2>&1 &
game_pid=$!
for _ in {1..65}; do
  window="$(DISPLAY=:99 xdotool search --onlyvisible --name Weston 2>/dev/null | head -n 1 || :)"
  [[ -n "$window" ]] || window="$(DISPLAY=:99 xdotool search --onlyvisible --name . 2>/dev/null | head -n 1 || :)"
  if [[ -n "$window" ]]; then
    DISPLAY=:99 xdotool windowfocus "$window" 2>/dev/null || :
    DISPLAY=:99 xdotool mousemove --window "$window" 55 55 click 1 2>/dev/null || :
    break
  fi
  kill -0 "$game_pid" 2>/dev/null || break
  sleep 0.1
done
if [[ "$STARBYTE_FOCUS_PROBE" == --blur-smoke ]]; then
  # The app arms its real focus policy and publishes this marker *before*
  # an external native Wayland client changes compositor keyboard focus.
  for _ in {1..90}; do
    grep -Fq 'STARBYTE_FULL_LIBRARY_REAL_PAUSE_BEGIN frame=' "$runtime/game.log" && break
    kill -0 "$game_pid" 2>/dev/null || break
    sleep 0.1
  done
  grep -Fq 'STARBYTE_FULL_LIBRARY_REAL_PAUSE_BEGIN frame=' "$runtime/game.log" || {
    echo STARBYTE_FOCUSED_SEAT_FAILED physical_pause_not_armed >&2
    tail -n 35 "$runtime/game.log" >&2
    exit 1
  }
  # Use a separate X11 surface *inside Xvfb*, not an extra Weston
  # client: kiosk-shell can retain the old keyboard seat for new surfaces.
  # Explicit XSetInputFocus to this private rival produces the real outer
  # compositor FocusOut / FocusIn sequence without touching host input.
  DISPLAY=:99 xmessage -name starbyte-private-focus -geometry 320x100+350+240 \
    "Private Starbyte compositor focus transition" >"$runtime/rival.log" 2>&1 &
  rival_pid=$!
  rival_window=''
  for _ in {1..45}; do
    rival_window="$(DISPLAY=:99 xdotool search --onlyvisible --pid "$rival_pid" 2>/dev/null | head -n 1 || :)"
    [[ -n "$rival_window" ]] ||
      rival_window="$(DISPLAY=:99 xdotool search --onlyvisible --name 'xmessage' 2>/dev/null | head -n 1 || :)"
    [[ -n "$rival_window" ]] && break
    kill -0 "$rival_pid" 2>/dev/null || break
    sleep 0.1
  done
  if [[ -z "$rival_window" ]] || ! kill -0 "$rival_pid" 2>/dev/null; then
    echo STARBYTE_FOCUSED_SEAT_FAILED rival_x11_window_missing >&2
    tail -n 35 "$runtime/rival.log" >&2
    exit 1
  fi
  timeout --signal=TERM 3s env DISPLAY=:99 xdotool windowfocus --sync "$rival_window" || {
    echo STARBYTE_FOCUSED_SEAT_FAILED private_rival_focus_failed >&2
    exit 1
  }
  observed_focus="$(DISPLAY=:99 xdotool getwindowfocus)"
  [[ "$observed_focus" == "$rival_window" ]] || {
    echo STARBYTE_FOCUSED_SEAT_FAILED private_rival_not_focused >&2
    exit 1
  }
  echo STARBYTE_PRIVATE_FOCUS_OUT outer_x11=rival
  sleep 2.65
  timeout --signal=TERM 3s env DISPLAY=:99 xdotool windowfocus --sync "$window" || {
    echo STARBYTE_FOCUSED_SEAT_FAILED private_weston_refocus_failed >&2
    exit 1
  }
  DISPLAY=:99 xdotool mousemove --window "$window" 55 55 click 1 2>/dev/null || :
  kill "$rival_pid" 2>/dev/null || :
  wait "$rival_pid" 2>/dev/null || :
  rival_pid=''
  echo STARBYTE_PRIVATE_FOCUS_IN outer_x11=weston
fi
if ! wait "$game_pid"; then
  echo STARBYTE_FOCUSED_SEAT_FAILED game_exit >&2
  tail -n 55 "$runtime/game.log" >&2
  exit 1
fi
if [[ "$STARBYTE_FOCUS_PROBE" == --blur-smoke ]]; then
  grep -F 'STARBYTE_FULL_LIBRARY_REAL_PAUSE_SMOKE_OK advanced_away=' "$runtime/game.log" || {
    echo STARBYTE_FOCUSED_SEAT_FAILED native_blur_resume_missing >&2
    grep -E 'STARBYTE_FULL_LIBRARY_REAL_PAUSE_(BEGIN|SMOKE_FAILED|SMOKE_OK)' "$runtime/game.log" >&2 || :
    tail -n 35 "$runtime/game.log" >&2
    exit 1
  }
  echo STARBYTE_PRIVATE_BLUR_SEAT_OK compositor=weston_x11 seat=xvfb physical=not_tested
else
  grep -F 'STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=' "$runtime/game.log" || {
    echo STARBYTE_FOCUSED_SEAT_FAILED focused_clock_missing >&2
    grep -E 'STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_(BEGIN|FAILED|OK)' "$runtime/game.log" >&2 || :
    tail -n 35 "$runtime/game.log" >&2
    exit 1
  }
  echo STARBYTE_PRIVATE_FOCUSED_SEAT_OK compositor=weston_x11 seat=xvfb physical=not_tested
fi
GUEST
chmod 700 "$root/guest.sh"
# X server lives strictly inside Bubblewrap's private /tmp. The only shared
# namespace is the ephemeral hosted runner's network (CAP_NET_ADMIN is absent).
timeout --signal=TERM --kill-after=2s 55s bwrap \
  --unshare-all --share-net --die-with-parent --new-session \
  --ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/lib /lib --symlink usr/lib64 /lib64 \
  --proc /proc --dev /dev --tmpfs /tmp --tmpfs /run --dir /run/user \
  --dir /var --dir /var/lib --dir /var/lib/xkb \
  --bind "$root" /run/user/qa --dir /home --dir /home/qa \
  --clearenv --setenv PATH /usr/bin --setenv HOME /home/qa \
  --setenv XDG_RUNTIME_DIR /run/user/qa --setenv XDG_CONFIG_HOME /home/qa/.config \
  --setenv XDG_CACHE_HOME /home/qa/.cache \
  --setenv WAYLAND_DISPLAY starbyte-focus-wayland \
  --setenv STARBYTE_FOCUS_PROBE "$probe" \
  --setenv WINIT_UNIX_BACKEND wayland --setenv LIBGL_ALWAYS_SOFTWARE 1 \
  /usr/bin/bash /run/user/qa/guest.sh || fail guest_failed
echo STARBYTE_HOSTED_FOCUSED_WAYLAND_OK probe="$probe" physical=not_tested desktop=untouched
