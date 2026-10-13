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

To test the native window and graphics path **without downloading or supplying a ROM**, run the source-generated red/cyan checkerboard demo:

```sh
cargo run -p starbyte-egui -- --demo
```

The game toolbar's **Input** toggle displays currently held SNES controller buttons, so you can test keyboard/gamepad mapping with the demo even though its checkerboard does not respond to input. This exercises the emulator and windowing, but is **not** evidence that any commercial game is playable.

**Controller mapping:** Settings → Input lists connected USB gamepads, live button labels, and analog/D-pad axis values. Every SNES controller action has **Rebind Pad**; it captures face/shoulder/D-pad button events or a deliberate analog-stick/D-pad-hat direction, then persists the mapping. **Reset Pad Defaults** restores the standard mapping; conflicting assignments are flagged. Input capture, focus changes and quick button taps have synthetic regression tests, but the physical generic-controller playability gate remains open. See [native gamepad input mapping](docs/input-mapping.md) and [Issue #1](https://github.com/sguzman/starbyte/issues/1).

On first launch with that folder available, Starbyte automatically discovers `~/Games/Roms/SNES` and remembers it in the local config. Removing the directory later is respected: automatic discovery does not repeatedly re-add it. This is a convenience, not a fixed requirement: use **Settings → Library → ROM Directories** to add/remove other folders. Entering `~/...` is supported there.

SNES game ROMs do not normally contain retail box-art imagery. To fetch covers from Libretro's external thumbnail catalog, press **Get Covers** in the library toolbar. This is a network operation performed in the background, only for installed games; it can take time on a large collection. Cached artwork is reused on subsequent launches. Games without a matching catalog title retain a neutral placeholder.

ZIP collections are supported natively. The scanner indexes valid `.sfc`, `.smc`, `.swc`, and `.fig` ROM files *inside* `.zip` archives. It keeps source ZIPs unchanged and only extracts a game into the XDG cache when selected. You do **not** need to unzip your library. After a scan, open **Settings → Open Scan Report** to see source-level diagnostics in the XDG cache (`manifests/rom-scan-report.json`), including failed or unreadable ZIP files.

For any other library directory:

```sh
cargo run -p starbyte-egui -- --rom-dir /path/to/your/roms
```

Starbyte can read a configured local [Cheatarium](https://github.com/sguzman/cheatarium) checkout via **Settings → Cheats**. The game cheat refresh uses its offline SNES index rather than downloading source codes when a local checkout is configured. Matches are unverified title candidates; supported single Game Genie cartridge-ROM read substitutions and single 7E/7F WRAM address/value codes can be enabled manually, while unresolved compound codes and unsupported formats remain read-only. The CLI also supports read-only candidate searches, and no ROM hash/revision match is claimed. See [Cheatarium integration](docs/cheatarium.md).

To check that the default Linux audio device accepts stereo output independently of game emulation, run the **opt-in synthetic speaker probe** (440 Hz, about 0.25 seconds; no ROM or firmware required):

```sh
cargo run -p starbyte-egui -- --speaker-test
```

For an **explicitly experimental** native graphical sound-driver probe on a single startup ROM, use `cargo run -p starbyte-egui -- --rom /path/to/game.sfc --experimental-sound-driver`. This mode replaces the compatibility bootstrap with isolated IPL upload/host-clocked SPC700 execution and may stall or produce incorrect sound. It loads a private copy of existing battery SRAM (after flushing pending normal-play progress), **never writes probe changes back** to disk and never reads or writes durable save slots. The same mode can be reached from Play View's **Sound → Try experimental game audio** confirmation. Experimental SPC execution uses a five-sample key-on startup delay; sound quality, speaker audibility and compatibility remain unverified. The flag is not persisted and does not apply automatically to subsequent library launches. Normal gameplay remains on the stable compatibility path.

**Sound-state visibility:** while a game is running, the status bar now explicitly distinguishes `Sound: SILENT (compatibility)` from opt-in experimental sound. The Sound menu displays last-frame and cumulative nonzero stereo PCM pairs, the OS audio-device state, and how many fresh audio frames the playback worker actually appended to the Rodio mixer. `Sound: PCM submitted (audibility unknown)` is not confirmation that speakers emitted sound. The default mode still cannot produce game music; a rebuilt binary alone does not turn it on. To investigate sound once without overwriting saves, use **Sound (silent) → Try experimental game audio → Restart isolated with sound**. Neither sound mode nor hardware output routing can be inferred from an audio-sample-count-only status line. 

This is only a local audio-device smoke test: a successful exit does not prove that real SNES music works. Normal emulation opens the speaker device lazily only when nonzero sound is generated. A missing device does not stop graphics or controls; device creation, bounded queueing and automatic retry run on a dedicated background audio thread rather than blocking the UI. Game audio more than 150 ms old is discarded to limit audible lag. Rust dependencies are pinned in `Cargo.lock` and CI uses locked builds.

For CLI help:

```sh
cargo run -p starbyte-cli -- --help
```

The desktop has a dedicated **Play View**, which makes the rendered game the main content rather than a small sidebar preview. Launching a ROM opens Play View automatically. Use **F9** to switch between the game and library, **Escape** to return to the library, **F5/F8** for temporary quick save/load, and the in-game toolbar for pause, fullscreen, **F12 PNG screenshots**, and three **disk-backed save slots**. Disk slots are local JSON snapshots that include the current ROM and may be large; they are not battery-backed SRAM. The `integer_scale` setting preserves whole-pixel multiples when there is room, but scales down to fit narrow tiling windows. While a game runs, the bottom status bar shows **Emu FPS** (completed emulated frames per real second, using a rolling two-second window; SNES NTSC target ~60.1). The Play View **Timing** toggle and Session panel additionally show frame time, UI wake lateness, texture-upload time, frame gaps of 40 ms or more, and peak gap. Frame-gap spikes include host scheduling delays; they do not by themselves prove slow CPU emulation. This is not GPU-present/display refresh FPS. Pausing, loading another game, or restoring a state clears the FPS sample window. Paused Step 60 advances incrementally with a Cancel button rather than blocking the UI for all 60 frames.

Select an installed library card and click **Play Selected**, or use the card's **Play game** action. The separate **Resume/Pause** control only operates an already-loaded game. ROM extraction has its own background queue, so game launches do not wait for Libretro artwork downloads. A bottom status bar shows queued/running jobs.

The library has a direct **Open ROM...** file picker, and accepts ROM files or single-game ZIP archives dragged from a file manager. ZIPs containing multiple ROMs must be opened through the library to choose the intended member. Its **Recent** menu remembers up to eight locally opened games and offers one-click reloading without launching games automatically at startup. Missing paths remain visible but disabled until their files return.

Desktop speaker output is now wired through a bounded, best-effort Rodio backend: it opens the default audio device only after emulation generates nonzero 32 kHz stereo PCM, and audio-device failure does not prevent video or input. Queued sound is cleared on pause, ROM changes and state restores. The **Sound** menu in Play View now offers an immediate session-only mute switch, while Settings > Audio controls real speaker enable/disable and live 0–100% mixer volume. `Mute on startup` affects only the initial session mute state; it is not changed by pressing Mute during play. The emulated DSP stays at 32 kHz stereo and the OS output device handles resampling; the legacy `sample_rate_hz` setting is not a functional output-rate control and is no longer presented as one. **This is not yet a sound-quality or native-speaker claim:** an opt-in, headless Super Mario World audio probe on 2026-10-12 produced nonzero stereo PCM from the actual uploaded sound driver, but normal commercial-ROM playback still uses the silent bootstrap and native Linux speaker output remains unverified. The app requires the platform audio development/runtime libraries needed by Rodio/CPAL when building and using speaker output.

The GUI uses `eframe` with both Wayland and X11 backends enabled. **Native Wayland behavior and dependencies still require verification on the target Linux system.** These are source-build instructions, not a claim of a working packaged release.

## Linux configuration and cache

By default Starbyte uses XDG user directories, independent of the directory from which it is launched:

- Config: `$XDG_CONFIG_HOME/starbyte/config.toml`, falling back to `~/.config/starbyte/config.toml`.
- Cache: `$XDG_CACHE_HOME/starbyte/`, falling back to `~/.cache/starbyte/`.
- Cartridge SRAM: `$XDG_DATA_HOME/starbyte/saves/`, falling back to `~/.local/share/starbyte/saves/`. Modified save RAM is written atomically on game switch, roughly every 30 seconds during desktop sessions, and when Starbyte exits.
- Screenshots: `$XDG_DATA_HOME/starbyte/screenshots/`, falling back to `~/.local/share/starbyte/screenshots/`. F12 exports a native-resolution PNG without UI chrome.
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
