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

## Unattended real-Hyprland QA

When this coding agent runs **inside the owner's actual interactive Hyprland
session**, run `bash scripts/qa-native-hyprland.sh` itself, without delegating
terminal, controller or window operations to the owner. It builds the binaries,
runs the nine synthetic native-window probes plus two genuine physical workspace
transitions, restores the compositor workspace and keeps raw logs private.
Report actual observed results, not inferred availability.

When orchestrating through GitHub instead, change only
`.github/native-qa-dispatch` on main to trigger the dedicated trusted
self-hosted workflow. This requires an **already registered and running**
`self-hosted, linux, x64` runner in the actual login session;
a queued job does **not** mean the desktop was tested. Do not install a
system-level runner, leak host credentials, upload logs or treat a generic
cloud CI machine as a substitute. See `docs/native-qa-execution.md`.
Never mark physical speaker, gamepad, save-state or production-cutover gates
accepted without their own direct evidence.

## Physical acceptance: local Codex / QA operator

A coding agent already executing inside the owner's **actual interactive Hyprland session** owns native QA, not the user. From a trusted current Starbyte checkout, it runs `bash scripts/qa-native-hyprland.sh`; if authenticated local `gh` has repository runner-registration and Actions privileges, `bash scripts/qa-runner-ephemeral.sh` attaches a temporary one-job runner to the existing physical workflow. Do not tell the user to run commands, inspect windows, perform controller QA or collect output. Do not give a GitHub-hosted Weston check the meaning of physical Hyprland evidence. The local collector writes private original logs and prints only a sanitized physical telemetry receipt on complete success; no secrets or personal paths in public issues/CI.

When a chat agent has GitHub-only access, it **cannot execute commands on the user's desktop**, even if a physical workflow was dispatched. Check for an actual assigned self-hosted runner and completed physical job. If none is connected, record the execution blocker precisely, without inventing a test result, automatically accepting gates, or emitting repeated empty dispatches.

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
