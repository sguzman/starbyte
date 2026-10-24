#!/usr/bin/env bash
# Local Codex executor: temporarily attach a native Wayland session to the
# already-queued Starbyte Actions QA run(s). No permanent runner service,
# privilege escalation, repository credential export or manual gameplay QA.
set -Eeuo pipefail
umask 077

fail() {
  printf 'STARBYTE_QA_RUNNER_FAILED %s\n' "$1" >&2
  exit 1
}

if [[ $# -gt 1 || (${1:-} != '' && ${1:-} != '--two-queued-runs') ]]; then
  fail 'usage: qa-runner-ephemeral.sh [--two-queued-runs]'
fi

[[ "$(id -u)" != 0 ]] || fail 'must_run_as_desktop_user_not_root'
[[ "${XDG_SESSION_TYPE:-}" == wayland ]] || fail 'must_be_in_interactive_wayland_session'
[[ -n "${WAYLAND_DISPLAY:-}" && -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]] ||
  fail 'must_inherit_active_hyprland_environment'
[[ -d "${XDG_RUNTIME_DIR:-/nonexistent}" ]] || fail 'missing_runtime_directory'

for command_name in gh curl sha256sum tar hyprctl git; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing_${command_name}"
done
hyprctl -j activeworkspace >/dev/null 2>&1 || fail 'hyprland_ipc_unavailable'

# Requires the owner's pre-existing gh login with Administration:write on
# this repository. The ephemeral registration token is never printed.
export GH_PROMPT_DISABLED=1
gh auth status >/dev/null 2>&1 || fail 'github_cli_not_authenticated'
gh api repos/sguzman/starbyte >/dev/null 2>&1 ||
  fail 'authenticated_github_repository_unavailable'

asset="$(gh api repos/actions/runner/releases/latest --jq '
  .assets[]
  | select(.name | startswith("actions-runner-linux-x64-") and endswith(".tar.gz"))
  | "\(.browser_download_url) \(.digest)"
' | head -n 1)"
read -r download_url digest <<<"$asset"
[[ "$download_url" == https://github.com/actions/runner/releases/download/* ]] ||
  fail 'missing_official_runner_release'
[[ "$digest" =~ ^sha256:[[:xdigit:]]{64}$ ]] ||
  fail 'missing_official_runner_sha256_digest'

cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/starbyte/native-qa-runner"
mkdir -p "$cache_root"
chmod 700 "$cache_root"
archive="$cache_root/$(basename "$download_url")"
if [[ ! -f "$archive" ]]; then
  part="$archive.partial.$$"
  trap 'rm -f -- "$part"' EXIT
  curl --fail --location --retry 3 --connect-timeout 20 \
    --output "$part" "$download_url" || fail 'runner_download_failed'
  printf '%s  %s\n' "${digest#sha256:}" "$part" | sha256sum -c --status ||
    fail 'runner_archive_checksum_invalid'
  mv -- "$part" "$archive"
  trap - EXIT
fi
printf '%s  %s\n' "${digest#sha256:}" "$archive" | sha256sum -c --status ||
  fail 'cached_runner_archive_checksum_invalid'

# Isolated, ephemeral runner instances each process exactly one accepted job.
# Two iterations consume both previous queued attempts, if they still exist.
# No background daemon remains attached to the user's desktop afterward.
max_jobs=1
if [[ "${1:-}" == '--two-queued-runs' ]]; then max_jobs=2; fi
for ((job = 1; job <= max_jobs; job++)); do
  runner_root="$(mktemp -d "${TMPDIR:-/tmp}/starbyte-actions-ephemeral.XXXXXXXX")"
  chmod 700 "$runner_root"
  tar -xzf "$archive" -C "$runner_root" || fail 'runner_extraction_failed'
  registration_token="$(gh api --method POST \
    repos/sguzman/starbyte/actions/runners/registration-token --jq '.token')" ||
    fail 'runner_registration_requires_repository_admin_access'
  [[ -n "$registration_token" ]] || fail 'empty_runner_registration_token'

  (
    cd "$runner_root"
    # --ephemeral deregisters this instance after one Actions job.
    ./config.sh --unattended --ephemeral --url https://github.com/sguzman/starbyte \
      --name "starbyte-hyprland-${USER:-owner}-$$-$job" \
      --labels hyprland --work _work --token "$registration_token" \
      >"$runner_root/config.log" 2>&1 ||
      fail 'ephemeral_runner_registration_failed_private_log'
    unset registration_token
    # Keep Wayland/Hyprland environment inherited from the real login
    # session. The actual QA script enforces native compositor identity.
    ./run.sh >"$runner_root/run.log" 2>&1 ||
      fail 'ephemeral_runner_execution_failed_private_log'
  )
  # The runner is unregistered; raw logs remain private to the local agent.
  printf 'STARBYTE_QA_EPHEMERAL_RUNNER_FINISHED job=%s status=inspect_actions_results\n' "$job"
done
