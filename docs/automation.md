# Starbyte automation and future MCP adapter

Starbyte is designed to be straightforward for AI coding agents to build, test and inspect. **It does not currently implement an MCP server**. The initial supported interface is the local Rust CLI.

## Discover commands

```sh
cargo run -p starbyte-cli -- capabilities
cargo run -p starbyte-cli -- doctor --json
```

The CLI reserves **stdout for command output** and sends tracing diagnostics to **stderr**, so machine-readable JSON remains parseable even while cartridge loading emits logs.

The JSON responses declare `starbyte.capabilities.v1` and `starbyte.doctor.v1`. The first advertises supported command shapes and their side effects; the second inspects OS, session environment, config/cache locations and existence flags. Setting `WAYLAND_DISPLAY` is **not** proof that native Wayland rendering works.

ROM execution reports (`run ROM --frames N --report-json PATH`) now include a versioned `starbyte.run_report.v1` schema. For black-screen triage, inspect `ppu_display` (brightness, forced blank, mode, layers, VRAM base registers) and `framebuffer` (nonblack pixel count, distinct RGB colors, center pixel, hash). These are deterministic **end-of-run diagnostics**, not verified gameplay measurements or proof of visual correctness.

### Frame-by-frame failure capture

For a game whose display flashes, corrupts, or turns black before emulation stalls, use an explicit local JSONL log. For example:

```sh
cargo run --release -p starbyte-cli -- run /path/to/game.sfc --frames 60 --no-save-ram --frame-log /tmp/starbyte-frames.jsonl --report-json /tmp/starbyte-final.json
```

Each completed frame emits and flushes a `starbyte.frame_log.v1` JSON object containing the requested frame, actual completed frame count, 65816 CPU register state, framebuffer hash, nonblack pixel count, distinct RGB color count, PPU display setup, cumulative PPU register write activity, CPU↔APU communication ports/read/write counters, DMA/HDMA transferred-byte count, programmed NMITIMEN and H/V IRQ timer targets, and APU steps. If a frame returns an emulation error, the log contains a final `status: "error"` record with the error string and the **last fully rendered framebuffer**. The separate end-of-run report is **not** written when emulation errors; the frame log preserves completed evidence. These reports cannot establish that the visible scene is *correct*, only what the emulator computed.

The frame log is **opt-in**, writes only to its explicitly supplied local path, truncates a preexisting file at that path, and is flushed after each attempted frame.

For hands-free visual evidence, add `--frame-images-dir /tmp/starbyte-images`. The CLI saves PPM screenshots of completed frames: the first frame and then every frame by default, capped at **24** files per run. Use `--frame-image-every 10` to sample more widely or `--max-frame-images 8` to reduce disk use. Image paths are named `frame-000001.ppm` and so on using the actual completed frame counter. These are local generated outputs, not uploaded, and an existing matching filename in the chosen directory is replaced. Frames that fail before completion are not newly rendered or saved.

Example capturing early SMW behavior from a single-ROM ZIP:

```sh
cargo run --release -p starbyte-cli -- run "$HOME/Games/Roms/SNES/Super Mario World (USA).zip" --frames 60 --no-save-ram --frame-log /tmp/starbyte-smw.jsonl --frame-images-dir /tmp/starbyte-smw-frames --frame-image-every 5 --max-frame-images 16
```

The path above is an example, not a claim that a particular archive exists locally; pass your actual file path. Use `--no-save-ram` for controlled probes so Starbyte neither loads an existing cartridge SRAM sidecar nor writes one on exit. Ordinary `run` still supports persistent SRAM; the diagnostic flag does not modify that default.
 Do not publish ROM-specific file paths or proprietary traces without review. The CLI also accepts a ZIP path and currently selects its first supported ROM member; multi-ROM ZIPs must be disambiguated outside this command. Source archives are not modified.

Read-only Cheatarium lookup: `starbyte cheatarium --index /path/to/snes.json.gz --title 'Donkey Kong Country' --json`. It reads an explicit local index file only, reports unverified title candidates and source provenance, and cannot write to a ROM or enable cheats. See [Cheatarium integration](cheatarium.md).

Other existing CLI actions include `inspect /path/to/game.sfc --json` (schema `starbyte.rom_inspect.v1`, no playability claim), `print-config json`, `library scan --json`, and `run /path/to/game.sfc --frames 1 --report-json /path/to/report.json`. The ROM run writes its report explicitly. Library scanning may write cache data and should not be treated as read-only. Provider-refresh commands may access the network if enabled.

### Targeted instruction tracing inside a normal ROM run

When a frame log identifies a suspicious **one-based frame number**, use the standard `run` command's optional `--trace-frame N --trace-out PATH` pair. For example:

```sh
cargo run --release -p starbyte-cli -- run /path/to/game.zip --frames 38 --no-save-ram --trace-frame 38 --trace-out /tmp/starbyte-frame38.jsonl
```

Only the selected frame is instrumented; preceding frames run through the normal guard. Each successfully executed instruction is recorded with CPU register snapshots before and after, its opcode (null for interrupt service), and all observed bus events. The final record includes the CPU state and an error field even if the traced frame fails on an unsupported opcode. The 100,000-instruction-per-frame safety guard limits trace size but **this remains an extensive diagnostic** (often tens of thousands of JSONL lines), so capture only one frame at a time and compress before sharing. The command never uploads anything and does not copy proprietary ROM bytes into the repository.

### Instruction-level startup investigation

When black frames persist, the frame counters alone cannot reveal what the CPU is waiting for. For a single-ROM local ZIP, record one frame of instruction/bus evidence using:

```sh
cargo run --release -p starbyte-cli -- compliance commercial-record /path/to/game.zip --frames 1 --fixture-out /tmp/starbyte-boot-fixture.json --trace-out /tmp/starbyte-boot-trace.json
```

The command writes the fixture, a sibling `.report.json` file and the instruction trace only to the requested local paths. It does not save cartridge SRAM. One frame is normally sufficient to inspect the first busy loop; the captured trace can still be large. Use the trace's `frame`, `pbr`, `pc`, `opcode` and `mmio_events` to identify repeated reads and unmet handshakes. The `opcode` is **null** on interrupt-service steps that did not fetch an instruction; treating the first stack write as an opcode would produce false diagnoses.

To inspect one frame **later** in a long boot without storing all preceding instructions:

```sh
cargo run --release -p starbyte-cli -- compliance commercial-record /path/to/game.zip --frames 181 --trace-from-frame 180 --fixture-out /tmp/starbyte-late.json --trace-out /tmp/starbyte-late-trace.json
```

`--trace-from-frame N` skips the first **N completed frames**, then starts instruction recording. The selected trace start is persisted in the fixture for reproducible replay. For example, `--frames 181 --trace-from-frame 180` runs 181 frames but records only frame 181 (zero-based trace field `frame: 180`). The usual core instruction-per-frame guard still applies. Do not request needlessly large trace windows; frame logs are lighter for long scans. This captures *emulator behavior*, not proof of ROM correctness or commercial playability. Never commit private game traces or proprietary ROM bytes to the source repository.

## Agent safety rules

- No network upload, ROM transfer, arbitrary shell tool or remote-control daemon is implied by the command manifest.
- The user chooses which ROM paths, game directories and report destinations are exposed.
- Do not infer broad commercial compatibility from synthetic smoke results.
- Prefer explicit time and frame budgets for ROM execution; untrusted ROMs and external metadata remain untrusted inputs.
- No background server, remote endpoint, or MCP transport is provided by `capabilities` or `doctor`.

## Future MCP design

A future opt-in MCP adapter should initially be **read-only** and expose bounded local resources: `starbyte://version`, `starbyte://diagnostics`, and `starbyte://library/summary`. Operations requiring ROM execution, directory scanning, file writes, or network access must require user authorization and precisely scoped arguments. Version tool schemas and validate them with protocol-level tests before claiming MCP compatibility.

The authoritative product priorities remain in [the roadmap](roadmap.md), and actual verification status in [the status snapshot](status.md).
