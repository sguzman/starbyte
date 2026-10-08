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

## Linux/Wayland

Prefer native Wayland and test under a Wayland session, especially resizing, fullscreen, scaling, keyboard/gamepad focus, dialogs, audio continuity and idle/input behavior. Keep the X11 fallback isolated and do not call Wayland validated merely because the `wayland` Cargo feature compiles. Avoid fixed minimum windows that make tiling inconvenient.

## AI and MCP friendliness

- Expose stable, documented, serializable schemas for state inspection and automation before adding complex autonomous behavior.
- Use bounded CLI commands and JSON outputs as the initial agent interface.
- Query `starbyte capabilities` for the v1 command manifest and `starbyte doctor --json` for local platform state. See `docs/automation.md`.
- For any future MCP server, define explicit tools/resources, input schemas, capability boundaries, opt-in permissions and safe read-only defaults.
- Never grant an agent automatic access to arbitrary local ROM directories, file writes, network downloads or process execution.
- Do not claim an MCP endpoint is operational until implemented and tested.

## Documentation discipline

`README.md` explains the product and how to use it. `docs/status.md` documents verified facts and uncertainties. `docs/roadmap.md` prioritizes upcoming work. Detailed subsystem documents retain technical history. Do not turn the README into a chronological changelog or inflate milestones by checking boxes for scaffolding only.
