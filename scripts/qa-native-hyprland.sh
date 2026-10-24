#!/usr/bin/env bash
# Run from a trusted Starbyte checkout within a *real*, interactive Hyprland
# session. This is designed for a local Codex agent or an already-attached
# self-hosted runner, not for a generic GitHub-hosted Linux VM.
set -Eeuo pipefail
umask 077

fail() {
  printf 'STARBYTE_NATIVE_QA_FAILED %s\n' "$1" >&2
  exit 1
}

[[ "${XDG_SESSION_TYPE:-}" == wayland ]] || fail 'no_native_wayland_session'
[[ -n "${WAYLAND_DISPLAY:-}" ]] || fail 'no_wayland_display'
[[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]] || fail 'no_hyprland_session'
for command_name in cargo git hyprctl jq; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing_$command_name"
done

# Do not run public, untrusted code with unrestricted access to an owner's
# desktop. This checkout may be mutated by an authorized local coding agent
# but CI dispatches are pinned to the repository's protected main branch.
if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
  case "${GITHUB_EVENT_NAME:-}" in
    workflow_dispatch|push) ;;
    *) fail 'unsafe_action_event' ;;
  esac
  [[ "${GITHUB_REPOSITORY:-}" == sguzman/starbyte ]] || fail 'unexpected_repository'
  [[ "${GITHUB_REF:-}" == refs/heads/main ]] || fail 'not_main_branch'
fi

checkout_root="$(git rev-parse --show-toplevel)"
cd "$checkout_root"
revision="$(git rev-parse HEAD)"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail 'invalid_git_revision'

# Every file, including raw logs and compositor observations, stays on the
# owner's machine. Never upload a public Actions artifact containing paths,
# window titles, audio routing, controller metadata or saved game bytes.
local_root="$(mktemp -d "${TMPDIR:-/tmp}/starbyte-native-qa.XXXXXXXX")"
chmod 700 "$local_root"
printf '%s\n' "$revision" >"$local_root/revision.txt"

if ! cargo build --locked -p starbyte-egui --bins >"$local_root/build.log" 2>&1; then
  fail 'build_failed_private_log_retained'
fi
if ! cargo test --locked -p starbyte-egui --bin starbyte-native-qa >"$local_root/runner-tests.log" 2>&1; then
  fail 'runner_contract_failed_private_log_retained'
fi

# Do not create twelve test windows on the owner's working workspace.
# The physical tests still need compositor focus, so run them together in one
# fresh, unused numeric QA workspace rather than scattering windows among
# whichever workspaces are active. Build/tests above do not change focus.
original_workspace="$(hyprctl -j activeworkspace | jq -er '.id | select(type == "number" and . > 0)')" ||
  fail 'original_workspace_not_regular_numeric'
qa_workspace=$((800000 + $ % 100000))
while hyprctl -j workspaces | jq -e --argjson id "$qa_workspace"   '.[] | select(.id == $id)' >/dev/null; do
  qa_workspace=$((qa_workspace + 1))
done
# Do not steal the user's focus back if they intentionally changed to a
# different workspace during QA. Restore only when the active workspace is
# ours, including a temporary starbyte-qa-* compositor transition.
restore_desktop_workspace() {
  local active_id active_name
  active_id="$(hyprctl -j activeworkspace | jq -r '.id // empty' 2>/dev/null)" || return
  active_name="$(hyprctl -j activeworkspace | jq -r '.name // empty' 2>/dev/null)" || return
  if [[ "$active_id" == "$qa_workspace" || "$active_name" == starbyte-qa-* || "$active_name" == name:starbyte-qa-* ]]; then
    hyprctl dispatch "hl.dsp.focus({ workspace = $original_workspace })" >/dev/null 2>&1 ||
      printf 'STARBYTE_NATIVE_QA_FAILED desktop_workspace_restore_failed\n' >&2
  fi
}
trap restore_desktop_workspace EXIT
hyprctl dispatch "hl.dsp.focus({ workspace = $qa_workspace })" >/dev/null ||
  fail 'cannot_enter_dedicated_qa_workspace'
[[ "$(hyprctl -j activeworkspace | jq -r '.id // empty')" == "$qa_workspace" ]] ||
  fail 'dedicated_qa_workspace_not_active'

set +e
STARBYTE_QA_WORKSPACE_ID="$qa_workspace" target/debug/starbyte-native-qa --physical-workspace >"$local_root/native-qa.log" 2>&1
probe_status=$?
set -e

# The binary writes to a second private, unique root; keep the original
# path available locally but never print or upload it into public CI.
probe_root="$(sed -n 's/^Local-only logs and context: //p' "$local_root/native-qa.log" | tail -n 1)"
[[ "$probe_status" == 0 ]] || fail 'physical_probe_failed_private_logs_retained'
[[ -n "$probe_root" && -r "$probe_root/results.tsv" ]] || fail 'no_probe_receipts'

# Nine native smokes, one focused presentation check, and two real workspace transitions.
awk -F '\t' '
  NR == 1 { if ($1 != "probe" || $2 != "result") exit 1; next }
  $2 != "pass" { exit 1 }
  { count++ }
  END { if (count != 12) exit 1 }
' "$probe_root/results.tsv" || fail 'incomplete_12_probe_receipts'
focused_receipt="$probe_root/real-focused-clock/focused-clock-observation.json"
[[ -s "$focused_receipt" ]] || fail 'missing_focused_clock_compositor_receipt'
grep -Fq '"native_wayland_client": true' "$focused_receipt" ||
  fail 'focused_clock_was_not_native_wayland'
grep -Fq '"window_retained_keyboard_focus": true' "$focused_receipt" ||
  fail 'focused_clock_did_not_retain_focus'
for physical in real-workspace-progress real-background-pause; do
  receipt="$probe_root/$physical/workspace-transition.json"
  [[ -s "$receipt" ]] || fail "missing_${physical}_receipt"
  grep -Fq '"returned_to_original_workspace": true' "$receipt" || fail "incomplete_${physical}_return"
  grep -Fq '"starbyte_native_wayland_client": true' "$receipt" || fail "unverified_${physical}_native_client"
done

# The original-window collector produces a strictly numeric/boolean,
# identity-free receipt only after every probe succeeds. Raw logs and host
# metadata remain private; this exact machine-readable line is safe for
# GitHub job logs and provides the evidence needed to review specific gates.
[[ -s "$probe_root/public-evidence.json" ]] ||
  fail 'missing_sanitized_physical_evidence'
public_evidence="$(sed -n 's/^STARBYTE_NATIVE_PHYSICAL_EVIDENCE=//p' "$local_root/native-qa.log" | tail -n 1)"
[[ -n "$public_evidence" ]] || fail 'missing_sanitized_physical_evidence_line'

cat >"$local_root/summary.json" <<EOF
{
  "schema_version": 1,
  "source_commit": "$revision",
  "result": "pass",
  "native_original_window_probes": {"passed": 9, "total": 9},
  "physical_focused_clock_probe": {"passed": 1, "total": 1},
  "physical_hyprland_workspace_probes": {"passed": 2, "total": 2},
  "speaker_audibility": "not_tested",
  "physical_gamepad": "not_tested",
  "production_cutover": "not_tested",
  "acceptance_ledger_mutated": false
}
EOF

# Only a sanitized, narrowly scoped result reaches the public Actions log.
printf 'STARBYTE_NATIVE_QA_OK 12/12 commit=%s physical_workspace=2 speaker=not_tested gamepad=not_tested cutover=not_tested\n' "$revision"
printf 'STARBYTE_NATIVE_PHYSICAL_EVIDENCE=%s\n' "$public_evidence"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    printf '### Starbyte physical Hyprland adjunct\n\n'
    printf '12/12 probes passed at `%s`; two included genuine workspace transitions and one focused clock measurement.\n\n' "$revision"
    printf '**Not tested:** audible speakers, physical gamepad, production cutover. No physical checklist gate has been automatically accepted. Raw results remain local-only.\n'
  } >>"$GITHUB_STEP_SUMMARY"
fi
