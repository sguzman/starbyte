# Commercial Fixture Template

Phase 2 commercial fixtures stay local-only. The repo stores the schema and workflow, but not commercial ROM assets.

## Suggested Layout

```text
tmp/
  Super Mario World.zip
  commercial-smw/
    smw-boot.json
    smw-boot.report.json
    artifacts/
    traces/
```

## Record And Replay

```powershell
cargo run -q -- compliance commercial-record "tmp\Super Mario World.zip" --frames 300 --fixture-out "tmp\commercial-smw\smw-boot.json"
cargo run -q -- compliance commercial-summary "tmp\commercial-smw"
cargo run -q -- compliance commercial-run-current "tmp\commercial-smw" --artifact-dir "tmp\commercial-smw\artifacts"
```

Optional trace capture:

```powershell
cargo run -q -- compliance commercial-record "tmp\Super Mario World.zip" --frames 300 --fixture-out "tmp\commercial-smw\smw-boot.json" --trace-out "tmp\commercial-smw\smw-boot.trace.json"
cargo run -q -- compliance commercial-run-current "tmp\commercial-smw" --trace-out "tmp\commercial-smw\traces"
```

## Fixture Shape

```json
[
  {
    "name": "SUPER MARIOWORLD commercial boot",
    "rom": "..\\Super Mario World.zip",
    "frames": 300,
    "controller1": {},
    "setup_writes": [],
    "expected": {
      "frame": 300,
      "cpu_pc": 32875,
      "cpu_pbr": 0,
      "wram_probes": [
        { "name": "wram_7E0000", "address": 8257536, "expected": 158 }
      ],
      "mmio_probes": [
        { "name": "inidisp", "register": 8448, "expected": 128 }
      ],
      "apu_io_activity": {
        "cpu_to_apu_ports": [0, 0, 0, 0],
        "apu_to_cpu_ports": [170, 187, 0, 0],
        "cpu_read_counts": [
          {
            "name": "read_$2140",
            "register": 8512,
            "count": { "min": 40511, "max": 40511 }
          }
        ],
        "cpu_write_counts": [
          {
            "name": "write_$2140",
            "register": 8512,
            "count": { "min": 40513, "max": 40513 }
          }
        ]
      },
      "ppu_write_activity": {
        "required_visible_registers_touched": [8448, 8449, 8455, 8460],
        "min_total_writes": 21571,
        "min_visible_display_write_count": 21564
      },
      "pixel_samples": []
    },
    "trace": {
      "capture_instruction_trace": false
    }
  }
]
```
