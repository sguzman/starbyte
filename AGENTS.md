# Starbyte contributor and coding-agent guide

Read `README.md`, `docs/status.md`, and `docs/roadmap.md` before modifying the code.

## Product invariant

Starbyte is a **fun-first Rust SNES emulator** targeting **Linux + native Wayland**. Good game experiences, a comfortable UI, convenience features and reliable automation outrank cycle-perfect hardware fidelity. Never optimize for abstract accuracy at the expense of the product without a demonstrated gameplay reason.

## Repository boundaries

- `starbyte-core`: emulation, no host UI, network or windowing assumptions.
- `starbyte-frontend`: platform-neutral sessions, library, metadata and service orchestration.
- `starbyte-egui`: desktop UI, user input, presentation, Wayland/X11 host concerns.
- `starbyte-cli`: machine-readable inspection, reproducible reports and headless test/automation commands.
- Keep third-party ROMs, firmware, secrets, caches and screenshots out of commits unless redistribution is explicitly permitted.

## Changes and evidence

1. Inspect the implementation and relevant tests before making claims.
2. Prefer a small, reviewable, bounded change that solves an observed user-facing problem.
3. Execute `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace` when possible; report exactly what was and was not run.
4. Do not equate passing synthetic tests with commercial ROM compatibility.
5. Capture a reproducible game-specific milestone: ROM identification by local digest, frames, visible scene, input, audio, save/load result, logs, platform/backend, and known anomalies. Do not commit copyrighted ROMs.
6. Keep `docs/status.md` and `docs/roadmap.md` current when implementation status changes.
7. Use conventional, scoped commit messages and commit completed work. Do not silently rewrite history.
8. Preserve existing behavior unless replacing it is intentional and documented.

## Unattended QA and desktop-focus protection

**Non-negotiable:** Never switch the owner's active Hyprland workspace, focus a
QA window, launch GUI probes, or change the owner's compositor settings in an
unattended Codex run. A separate workspace does **not** provide a separate
keyboard focus or seat. The owner explicitly reported unwanted focus hijacking
in both original and targeted focused-clock retests.

**Current guard:** `bash scripts/qa-native-hyprland.sh` and
`bash scripts/qa-native-hyprland.sh --focused-only` intentionally refuse
execution before launching any GUI windows or dispatching any workspace changes.
The direct `starbyte-native-qa` binary also refuses GUI tests. No agent may
bypass these guards through ad hoc commands or environmental overrides.

**Allowed without desktop interruption:**
`bash scripts/qa-native-hyprland.sh --check-only` builds the collector
and runs its contract tests and the scheduler ledger, without GUI launches.
Normal `cargo test`, lint and hosted Weston CI are likewise available but
must not be represented as physical compositor, audio or gamepad acceptance.

**Real focused QA may resume only** after a separately isolated compositor
**and input seat** has been independently demonstrated not to move focus or
open test windows on the owner's active desktop. Merely using
`movetoworkspacesilent` or a no-initial-focus window rule does not establish
that the window is keyboard-focused for D-12. No current route satisfies this
requirement; do not ask the human to test it manually.

The first real native run at `b1448b3` passed 11/12 and physically verified
off-workspace emulation (A-08) and blur pause/resume (D-14), now recorded as
71/77 accepted (92%). The targeted run at `fd522d4` measured 181 frames
at 60.006 FPS over 3016 ms but failed focused foreground/native-client
verification (0/1); D-12 and the other five physical gates remain open.

**Roles:** ChatGPT directs and audits; local Codex performs unattended,
non-interrupting diagnostics and future physical testing only after the
isolation requirement is satisfied. Do not claim a GitHub-hosted virtual
Wayland run or an unassigned self-hosted Actions job is physical acceptance.
Do not publish raw logs, ROMs, personal paths, audio routes, or device IDs.
Do not install a permanent runner or delegate terminal QA to the owner.

## Linux/Wayland

Prefer native Wayland and test under a Wayland session, especially resizing, fullscreen, scaling, keyboard/gamepad focus, dialogs, audio continuity and idle/input behavior. Keep the X11 fallback isolated and do not call Wayland validated merely because the `wayland` Cargo feature compiles. Avoid fixed minimum windows that make tiling inconvenient.

## AI and MCP friendliness

- Expose stable, documented, serializable schemas for state inspection and automation before adding complex autonomous behavior.
- Use bounded CLI commands and JSON outputs as the initial agent interface.
- Query `starbyte capabilities` for the v1 command manifest, `starbyte doctor --json` for local platform state, and `starbyte inspect ROM --json` for cartridge metadata. See `docs/automation.md`.
- For any future MCP server, define explicit tools/resources, input schemas, capability boundaries, opt-in permissions and safe read-only defaults.
- Never grant an agent automatic access to arbitrary local ROM directories, file writes, network downloads or process execution.
- Do not claim an MCP endpoint is operational until implemented and tested.

## Documentation discipline

`README.md` explains the product and how to use it. `docs/status.md` documents verified facts and uncertainties. `docs/roadmap.md` prioritizes upcoming work. Detailed subsystem documents retain technical history. Do not turn the README into a chronological changelog or inflate milestones by checking boxes for scaffolding only.

## Autonomous native acceptance (coding agent owned)

The remaining scheduler gates require **real** Hyprland, audio and controller
evidence. Do not ask a human to move workspaces, press keys, test games or
transcribe logs. If this agent has an interactive terminal in the owner's
native Wayland session, run `bash scripts/qa-native-hyprland.sh` directly.
For unattended GitHub job execution from that same session, use
`bash scripts/qa-runner-ephemeral.sh` with an already authenticated
repository-admin `gh` CLI. It downloads a checksum-verified official runner,
dispatches the current main's physical workflow, drains existing pending native
runs using ephemeral one-job registrations, and exits without installing a
permanent service. Neither script can attach to a desktop remotely.

Keep raw compositor metadata, device information and save files in the
isolated private local QA directories. Publish only sanitized results, complete
task-specific evidence and the exact tested commit. Physical D-12, A-08, D-13
and D-14 must be evaluated independently; current native automation is an
adjunct, not an automatic 77/77 acceptance certificate. Speaker audibility,
real gamepad control, save persistence and production cutover require their
own observed acceptance before editing the ledger.
