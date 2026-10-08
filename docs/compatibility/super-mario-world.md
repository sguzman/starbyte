# Super Mario World — compatibility investigation

**Current result: not playable (startup blocked / black screen).** This document tracks *observed* behavior, not a promise of compatibility.

## First reproducible headless probe

On 2026-10-08, the user ran the optimized Starbyte CLI against a locally supplied SNES ZIP, with `--frames 60 --no-save-ram --frame-log` and a 24-image PPM sample. The user provided the resulting JSONL log and screenshots for analysis; no copyrighted ROM or derived program code was committed.

| Property | Observed result |
| --- | --- |
| Archive filename | `Super Mario World.zip` |
| Loaded member | `Super Mario World.smc` |
| Internal cartridge title | `SUPER MARIOWORLD` |
| Mapper | LoROM |
| Exact ROM SHA-256 / revision | **Not yet recorded.** The filename does not prove USA/Japan/PAL revision. |
| Frames completed | 60/60 without a reported execution error |
| Pixel evidence | 0 nonblack pixels, 1 distinct RGB color, identical framebuffer hash across all 60 frames |
| Captured screenshots | 24 sampled PPM files; all identical black frames |
| PPU state | Forced blank throughout; brightness = 0, main screen enable mask = 0, background mode = 0 |
| CPU behavior | CPU continued executing; end-of-frame PC usually `$00:8095`–`$00:80A8`, with a temporary range `$00:8A53`–`$00:8A61` during frames 29–33 |
| Audio | APU stepping occurred, but authentic audio/game playback not verified |
| Native Wayland GUI | Not established by this headless probe |

**Interpretation:** The reported `frame_counter = 60` measures emulator timing progression, **not** successful game startup. Every rendered frame remained black, with forced blank still active. The CPU-PC pattern suggests repeated execution through small code regions, but does not identify why without an instruction/bus trace. In particular, an APU handshake wait is a hypothesis, not a finding; distinguish it from other CPU, MMIO, DMA or timing errors before changing emulation semantics.

## Next evidence needed

1. Capture the first 1–2 frames with `compliance commercial-record` and `--trace-out`, **without committing** the generated trace or ROM. Check actual opcode/PC paths and reads/writes to `$2140`–`$2143`, `$4200`–`$4212`, and `$2100`–`$213F`. Save the trace locally or share it privately for diagnosis.
2. Re-run a short `run` probe with the expanded `--frame-log` fields (`cpu` registers, `apu_io_activity` and `ppu_write_activity`) to see whether the game writes display registers or polls audio ports.
3. Record the ROM's local digest and precise revision **without** uploading or redistributing its bytes.
4. Fix a demonstrated emulation issue with a small synthetic regression test, then repeat this same commercial probe.
5. Do not upgrade status to boot/title/gameplay until a recognizable screen and responsive input are actually observed.

## Acceptance criteria

- Recognizable, stable title screen with nonblank, correctly arranged backgrounds/sprites.
- Game accepts Start; controllable first gameplay segment, with no recurring freezes or major corruption.
- Audio and save behavior checked separately.
- Native Hyprland/Wayland smoke test, including frame pacing, resize/fullscreen and input focus.

Related: [project status](../status.md), [commercial-ROM investigation tooling](../automation.md), [product roadmap](../roadmap.md).
