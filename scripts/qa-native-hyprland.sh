#!/usr/bin/env bash
# Non-disruptive QA entrypoint. Never switch workspaces, change window focus,
# spawn a GUI test window, or mutate the owner's Hyprland configuration.
#
# The first local 12-probe run stole the owner's desktop focus. Merely moving
# windows to a dedicated workspace did not protect the owner. Until an
# independently isolated compositor/seat is actually available and verified,
# unattended physical GUI tests must fail closed.
set -Eeuo pipefail
umask 077

fail() {
  printf 'STARBYTE_NATIVE_QA_BLOCKED %s\n' "$1" >&2
  exit 3
}

case "${1:-}" in
  --check-only)
    [[ $# -eq 1 ]] || fail 'usage: --check-only'
    ;;
  ""|--focused-only)
    [[ $# -le 1 ]] || fail 'unsupported_args'
    fail 'owner_desktop_focus_must_not_be_stolen; physical_gui_qa_requires_independent_compositor_and_seat'
    ;;
  *)
    fail 'unsupported_args'
    ;;
esac

for command_name in cargo git; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing_$command_name"
done

checkout_root="$(git rev-parse --show-toplevel)"
cd "$checkout_root"
revision="$(git rev-parse HEAD)"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail 'invalid_git_revision'
log_root="$(mktemp -d "${TMPDIR:-/tmp}/starbyte-native-check.XXXXXXXX")"
chmod 700 "$log_root"
if ! cargo build --locked -p starbyte-egui --bins >"$log_root/build.log" 2>&1; then
  fail 'build_failed; diagnostics_retained_locally'
fi
if ! cargo test --locked -p starbyte-egui --bin starbyte-native-qa >"$log_root/collector-tests.log" 2>&1; then
  fail 'collector_contract_failed; diagnostics_retained_locally'
fi
if ! cargo test --locked -p starbyte-cli --test scheduler_ledger >"$log_root/scheduler-ledger.log" 2>&1; then
  fail 'scheduler_ledger_failed; diagnostics_retained_locally'
fi
printf 'STARBYTE_NATIVE_CHECK_OK commit=%s physical_gui=not_run workspace_switches=0 gui_windows=0\n' "$revision"
printf 'STARBYTE_NATIVE_QA_BLOCKED physical_acceptance_requires_isolated_compositor_and_seat\n'
