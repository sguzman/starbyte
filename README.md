# Starbyte

**A cozy, feature-rich SNES emulator built in Rust for Linux desktops.**

Starbyte aims to make playing and exploring Super Nintendo games pleasant: an inviting library, responsive controls, convenient saves, thoughtful customization, and a desktop experience that feels at home on Wayland. It is developed with AI assistance and designed to be approachable by both human contributors and coding agents.

**Project status: experimental.** Starbyte has substantial emulation and frontend infrastructure, but broad commercial-game playability has **not** been established. Do not treat a successful frame probe, synthetic test, or completed scaffold as proof that a commercial game is playable.

## Philosophy

- **Fun first.** Prioritize enjoyable games and helpful user features over exhaustive hardware-cycle accuracy.
- **Compatibility is empirical.** Judge success by reproducible game-specific boot, title, input, gameplay, audio, and save milestones.
- **Rust native.** Keep the emulation core and app logic in Rust; use mature system libraries rather than rewriting everything.
- **Wayland first.** Linux/Wayland is the primary supported desktop target; X11 and Windows are secondary, best-effort paths.
- **AI-friendly by design.** Small bounded changes, explicit interfaces, reproducible tests, machine-readable tools and clear agent instructions.
- **MCP-friendly, not MCP-dependent.** Design documented, permission-bounded interfaces and automation adapters. No MCP server is claimed to exist yet.
- **No copyrighted assets in the repository.** Users supply any ROMs and optional firmware they are entitled to use.

Accuracy remains useful when it enables gameplay or prevents regressions. It is a means, not the identity of the project.

## Current architecture

| Crate | Responsibility |
| --- | --- |
| `starbyte-core` | CPU, PPU, APU, memory/bus, cartridges, enhancement chips, emulation state |
| `starbyte-frontend` | Session orchestration and reusable library services |
| `starbyte-egui` | Native desktop UI, game library and window/input integration |
| `starbyte-cli` | Inspection, execution, tests, reports and automation surface |

## Getting started (Linux)

Install a Rust toolchain compatible with `rust-version = 1.85` or newer and the system libraries required by `eframe`, `gilrs` and your audio/windowing stack. From the repository root:

```sh
cargo run -p starbyte-egui --
```

To open a local game:

```sh
cargo run -p starbyte-egui -- --rom /path/to/your/game.sfc
```

For a library directory:

```sh
cargo run -p starbyte-egui -- --rom-dir /path/to/your/roms
```

For CLI help:

```sh
cargo run -p starbyte-cli -- --help
```

The desktop has a dedicated **Play View**, which makes the rendered game the main content rather than a small sidebar preview. Launching a ROM opens Play View automatically. Use **F9** to switch between the game and library, **Escape** to return to the library, **F5/F8** for temporary quick save/load, and the in-game toolbar for pause, fullscreen, and three **disk-backed save slots**. Disk slots are local JSON snapshots that include the current ROM and may be large; they are not battery-backed SRAM. The `integer_scale` setting preserves whole-pixel multiples when there is room, but scales down to fit narrow tiling windows.

The GUI uses `eframe` with both Wayland and X11 backends enabled. **Native Wayland behavior and dependencies still require verification on the target Linux system.** These are source-build instructions, not a claim of a working packaged release.

## Linux configuration and cache

By default Starbyte uses XDG user directories, independent of the directory from which it is launched:

- Config: `$XDG_CONFIG_HOME/starbyte/config.toml`, falling back to `~/.config/starbyte/config.toml`.
- Cache: `$XDG_CACHE_HOME/starbyte/`, falling back to `~/.cache/starbyte/`.
- Persistent slots: `$XDG_STATE_HOME/starbyte/states/`, falling back to `~/.local/state/starbyte/states/`. Disk slots are keyed to the ROM's content and can be redirected with an explicit state-directory override in host integrations.
- An explicit `--config` or `--cache-dir` takes precedence.

If the new config does not exist, the CLI and desktop app can read the old worktree-relative `.config/starbyte/config.toml` or legacy cache-relative configuration when launched from the same directory. The desktop app writes its settings to the new XDG path; the old file is not overwritten. For a different working directory, supply `--config /path/to/old/config.toml` explicitly. No save files are automatically moved.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

Run core benchmarks with `cargo bench -p starbyte-core --bench core_bootstrap`. Hosted GitHub Actions runs Linux formatting/Clippy and Linux/Windows workspace tests. See the [latest runs](https://github.com/sguzman/starbyte/actions); a passing build is not proof of native Wayland or commercial-game playability. The workspace defaults to the CLI crate, so use `--workspace` for complete checks.

## Where to go next

- [Current state and evidence](docs/status.md): implemented versus verified, known gaps and confidence levels.
- [Product roadmap](docs/roadmap.md): next milestones in fun-first priority order.
- [PPU implementation coverage](docs/ppu-coverage.md): precisely supported graphics features and outstanding compatibility gaps.
- [Frontend architecture](docs/frontend-architecture.md) and [GUI backlog](docs/gui-roadmap.md).
- [Commercial-game bring-up](docs/commercial-rom-roadmap.md) and [enhancement chips](docs/coprocessor-roadmap.md): historical engineering detail, **not** proof of broad playability.
- [Agent/contributor guide](AGENTS.md): conventions for Rust, testing, Wayland, automation and MCP integration.
- [CLI automation interface](docs/automation.md): machine-readable discovery, local diagnostics and future MCP boundaries.

Starbyte is an evolving hobby emulator, not a high-accuracy replacement for established mature emulators.
