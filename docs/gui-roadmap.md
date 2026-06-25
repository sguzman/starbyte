# GUI Roadmap

This document tracks the library-first `egui` frontend work that sits on top of `starbyte-frontend`. The goal is to grow a practical desktop shell without pushing GUI or network concerns into `starbyte-core`.

## Phase 1: Config And App-State Foundation

- [x] Expand persistent runtime config with audio, video, input, cheat, library, and advanced/cache sections.
- [x] Add explicit cache-root and config-path support that both CLI and GUI can share.
- [x] Keep the config file human-editable and `print-config` compatible with the expanded shape.
- [x] Keep GUI/library state out of `starbyte-core` emulation logic.

## Phase 2: Library Scanning And Installed/Missing Model

- [x] Add frontend-neutral library types and snapshot/filter models in `starbyte-frontend`.
- [x] Scan configured ROM directories recursively for local SNES ROMs.
- [x] Normalize local ROM identity into stable game ids for merge/caching behavior.
- [x] Merge installed ROMs with metadata-only entries and surface present/missing state explicitly.
- [x] Support an installed-only filter across CLI and GUI surfaces.

## Phase 3: Metadata And Cover Provider Integration

- [x] Add pluggable provider traits for metadata and cover retrieval.
- [x] Implement one concrete public metadata/cover provider.
- [x] Cache metadata under `.cache/starbyte/games/metadata/`.
- [x] Cache cover images under `.cache/starbyte/games/covers/`.
- [x] Expose multiple library presentation modes: list, grid, and detailed.

## Phase 4: Cheats Integration And Per-Game Toggles

- [x] Add a pluggable cheat-provider trait.
- [x] Implement one concrete public cheat provider.
- [x] Cache cheats under `.cache/starbyte/games/cheats/`.
- [x] Persist enabled cheat selections per game in runtime config.
- [x] Expose per-game cheat toggles in the properties UI.
- [x] Apply enabled cheats to the live emulator runtime.

## Phase 5: CLI Cache-Management Commands

- [x] Add a dedicated `library` CLI command group.
- [x] Add manual scan, metadata refresh, cover refresh, cheat refresh, and refresh-all commands.
- [x] Support targeting by installed-only, game id, title, or ROM path.
- [x] Add machine-readable JSON output for library/cache operations.
- [x] Add a disabled ROM-download command surface that reports unsupported status clearly.

## Phase 6: `egui` Library Browser Views And Filters

- [x] Replace the bootstrap shell with a library-first application layout.
- [x] Add top-bar search, filter, view-mode, and refresh controls.
- [x] Add ROM-directory management from the GUI.
- [x] Add list, grid, and detailed library views.
- [x] Clearly indicate which games are installed locally and which are metadata-only.
- [x] Keep emulator session controls available without making them the primary UI.

## Phase 7: Context Menus, Properties, And Polish

- [x] Add per-game context menus with play/properties/refresh actions.
- [x] Add a properties window with local ROM info, provider info, and cache-adjacent details.
- [x] Add an “open ROM folder” action when a local ROM exists.
- [x] Add settings sections for audio, video, input, cheats, library, and advanced/cache options.
- [x] Keep the frontend thin by routing library/domain work through `starbyte-frontend`.
- [ ] Add richer provider coverage and broader metadata fields beyond the first public provider pass.

## Notes

- [x] ROM downloading is intentionally unsupported in v1.
- [x] Provider architecture leaves a future hook for ROM-download support without promising it now.
- [x] The current implementation is intended as a usable desktop library shell, not a final frontend polish pass.
