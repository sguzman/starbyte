# Starbyte automation and future MCP adapter

Starbyte is designed to be straightforward for AI coding agents to build, test and inspect. **It does not currently implement an MCP server**. The initial supported interface is the local Rust CLI.

## Discover commands

```sh
cargo run -p starbyte-cli -- capabilities
cargo run -p starbyte-cli -- doctor --json
```

The JSON responses declare `starbyte.capabilities.v1` and `starbyte.doctor.v1`. The first advertises supported command shapes and their side effects; the second inspects OS, session environment, config/cache locations and existence flags. Setting `WAYLAND_DISPLAY` is **not** proof that native Wayland rendering works.

Other existing CLI actions include `inspect /path/to/game.sfc --json` (schema `starbyte.rom_inspect.v1`, no playability claim), `print-config json`, `library scan --json`, and `run /path/to/game.sfc --frames 1 --report-json /path/to/report.json`. The ROM run writes its report explicitly. Library scanning may write cache data and should not be treated as read-only. Provider-refresh commands may access the network if enabled.

## Agent safety rules

- No network upload, ROM transfer, arbitrary shell tool or remote-control daemon is implied by the command manifest.
- The user chooses which ROM paths, game directories and report destinations are exposed.
- Do not infer broad commercial compatibility from synthetic smoke results.
- Prefer explicit time and frame budgets for ROM execution; untrusted ROMs and external metadata remain untrusted inputs.
- No background server, remote endpoint, or MCP transport is provided by `capabilities` or `doctor`.

## Future MCP design

A future opt-in MCP adapter should initially be **read-only** and expose bounded local resources: `starbyte://version`, `starbyte://diagnostics`, and `starbyte://library/summary`. Operations requiring ROM execution, directory scanning, file writes, or network access must require user authorization and precisely scoped arguments. Version tool schemas and validate them with protocol-level tests before claiming MCP compatibility.

The authoritative product priorities remain in [the roadmap](roadmap.md), and actual verification status in [the status snapshot](status.md).
