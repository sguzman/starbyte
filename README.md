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

The GUI uses `eframe` with both Wayland and X11 backends enabled. **Native Wayland behavior and dependencies still require verification on the target Linux system.** These are source-build instructions, not a claim of a working packaged release.

## Linux configuration and cache

By default Starbyte now uses the XDG user directories, independent of the directory from which it is launched:

- Config: `$XDG_CONFIG_HOME/starbyte/config.toml`, falling back to `~/.config/starbyte/config.toml`.
- Cache: `$XDG_CACHE_HOME/starbyte/`, falling back to `~/.cache/starbyte/`.
- An explicit `--config` or `--cache-dir` takes precedence.

If the new config does not exist, the CLI and desktop app can read the old worktree-relative `.config/starbyte/config.toml` or legacy cache-relative configuration when launched from the same directory. The desktop app writes its settings to the new XDG path; the old file is not overwritten. For a different working directory, supply `--config /path/to/old/config.toml` explicitly. No save files are automatically moved.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
```

Run core benchmarks with `cargo bench -p starbyte-core --bench core_bootstrap`. Tests and builds were **not rerun** as part of the documentation inheritance pass. The workspace currently defaults to the CLI crate; run `--workspace` for complete checks.

## Where to go next

- [Current state and evidence](docs/status.md): implemented versus verified, known gaps and confidence levels.
- [Product roadmap](docs/roadmap.md): next milestones in fun-first priority order.
- [Frontend architecture](docs/frontend-architecture.md) and [GUI backlog](docs/gui-roadmap.md).
- [Commercial-game bring-up](docs/commercial-rom-roadmap.md) and [enhancement chips](docs/coprocessor-roadmap.md): historical engineering detail, **not** proof of broad playability.
- [Agent/contributor guide](AGENTS.md): conventions for Rust, testing, Wayland, automation and MCP integration.
- [CLI automation interface](docs/automation.md): machine-readable discovery, local diagnostics and future MCP boundaries.

Starbyte is an evolving hobby emulator, not a high-accuracy replacement for established mature emulators.
