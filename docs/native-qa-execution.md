# Unattended native Hyprland QA execution

Starbyte has **two different execution environments**. Regular GitHub-hosted
Linux CI runs Rust and virtual-Wayland tests. It does not have the owner's GPU,
speakers, controller, active Hyprland workspaces or saved game paths. The
physical acceptance workload needs the actual user login session.

## Local agent path (preferred when Codex has desktop access)

Inside the trusted checkout in the active Hyprland login session, the local
coding agent runs `bash scripts/qa-native-hyprland.sh`. The script refuses a
missing Wayland display or Hyprland instance, builds the emulator, tests the QA
binary itself, then executes all **11** original-window probes, including both
real compositor workspace transitions. It checks native-Wayland client
identification and restoration to the original workspace.

All raw logs and host information remain in private (0700) local temporary
directories. There is no public artifact upload and no mutation of the owner's
normal game configuration or saves. The GitHub Actions log receives only a
redacted success/failure marker. A failed probe is a failure; it does not
authorize production cutover.

## GitHub dispatch path

`.github/workflows/native-hyprland.yml` runs only when an authorized actor
invokes `workflow_dispatch` on main **or commits a change to**
`.github/native-qa-dispatch` on main. These triggers never accept public
pull-request code. The job targets **[self-hosted, linux, x64, hyprland]** with
read-only GitHub permissions and no persisted checkout credentials.

The runner must already be **registered, online, labeled `hyprland`, and
running as part of the owner's interactive Wayland login**. A system service
without `WAYLAND_DISPLAY` and `HYPRLAND_INSTANCE_SIGNATURE` is deliberately
rejected, not allowed to fall back to XWayland or fake physical evidence.
GitHub-hosted runners cannot satisfy these requirements. This repository
does **not** automatically enroll the owner's computer as a remote execution
host or obtain any registration credentials.

To request a trusted run through GitHub, the steward updates
`.github/native-qa-dispatch` on main; then watches the **native Hyprland
acceptance** Actions run. A run queued for a missing self-hosted runner is
**not tested**. A failure indicating the missing session is also **not
tested**, not a passing gate. Never increase ledger percentages on a queued
job or generic CI success.

## Acceptance boundaries

The 11-probe native collector can establish original-window synthetic
behavior and, when a real local run succeeds, two compositor transitions.
Those do not independently prove that real speakers are audible, that a real
gamepad's controls reach the SNES port, that a USB/audio device physically
disconnects and reconnects, or that the production default has safely changed.
Treat these gate components as **not tested** until evidence has actually
been collected by dedicated on-host automation.

A successful run publishes only the narrow
`STARBYTE_NATIVE_QA_OK 11/11` marker in the workflow log; private
`results.tsv`, `summary.json`, and compositor receipts remain local. Verify
the job's Git SHA matches the checkout before attributing anything to a
commit. The authoritative gate wording and task ledger remain in
`docs/qa-native-hyprland.md` and `docs/scheduler-progress.json`.
