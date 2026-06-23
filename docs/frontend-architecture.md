# Frontend Architecture

Starbyte keeps emulator correctness in `starbyte-core` and host orchestration in frontend-facing crates.

## Layers

1. `starbyte-core`

- Owns emulation state, stepping, timing, save-state serialization, framebuffer generation, and audio sample buffering.
- Exposes stable host-facing types such as `Emulator`, `ControllerState`, and asset/runtime configuration.

2. `starbyte-frontend`

- Owns reusable session orchestration for native shells.
- Loads ROMs from paths, advances frames, applies controller state, and exports a frontend-neutral `SessionSnapshot`.
- Does not depend on `egui`, terminal UI code, or platform windowing.

3. Host shells such as `starbyte-egui`

- Render UI, collect input, and translate host events into `starbyte-frontend` session calls.
- Stay thin so alternate shells can be added without touching `starbyte-core`.

## Current Guidance

- Add new frontends on top of `starbyte-frontend` first.
- Keep screenshots, overlays, and presentation concerns in host crates.
- Avoid pushing GUI state or windowing assumptions into `starbyte-core`.
- Treat debugger tooling as out of scope unless the roadmap changes.
