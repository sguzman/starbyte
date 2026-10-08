# Super Mario World — compatibility investigation

**Current result: no verified playability; first 60 frames are black while audio data is being uploaded.** The first-frame instruction trace shows forward progress through the expected boot routine, so an initialization deadlock has **not** been demonstrated. This document tracks observations, not assumptions about compatibility.

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

## First-frame instruction and bus trace

The user also provided a one-frame `compliance commercial-record` instruction trace (5,103 instructions, 2026-10-08), enabling direct identification of the startup code:

- At `$00:8000`–`$00:8018`, the ROM disables interrupts and DMA, clears APU communication ports, and writes `$80` to `$2100` (forced blank).
- It reaches `$00:8079` (`SPC700UploadLoop`), reads **`$BBAA`** from `$2140/$2141`, sets up an ARAM destination **`$0500`**, writes transfer command **`$CC`** to `$2140`, and receives the matching acknowledgement.
- At `$00:8095`–`$00:80A8`, the CPU repeatedly sends the next byte through `$2141` with a monotonically increasing low-byte index through `$2140`. During frame 1 it makes **304 reads** of `$2140`, **305 writes** each to `$2140` and `$2141`, and writes **only one PPU register** (`$2100` forced blank).
- The 60-frame log later passes temporarily through `$00:8A53`–`$00:8A61` around frames 29–33, then returns to the transfer loop. This is evidence of execution *beyond* the first transfer phase, not proof of an infinite loop.

The routine and transfer handshake match the documented Super Mario World startup disassembly ([SMWDisX `SPC700UploadLoop`](https://github.com/IsoFrieze/SMWDisX/blob/master/bank_00.asm)) and the [SPC700 boot protocol](https://www.sneslab.net/wiki/SPC700/Driver_Upload). **The trace does not demonstrate a stalled APU acknowledgement**, and changing the emulator's APU logic on that assumption would be unjustified.

**Interpretation:** `frame_counter = 60` measures timing progression, not a successful boot. The 60-frame window ends while the game is still doing initial sound uploads, with the screen intentionally forced blank. A longer run is needed before we can decide whether this is an emulator fault, simply early boot, or both.



## Next evidence needed

1. Run a longer 360-frame headless probe with `--no-save-ram` and `--frame-log`, sampling a small number of PPM images. Check whether the APU upload completes and whether forced blank is cleared or any PPU display registers are configured. If execution fails, preserve the log through the failing frame.
2. If the game stays in the same code region, capture an instruction trace **starting near the later frame** rather than redundantly tracing the already understood first frame. The current `commercial-record` trace captures from startup; add bounded later-frame capture before requesting a large trace.
3. Record the ROM's exact local digest/revision **without** uploading its bytes.
4. Fix only a demonstrated emulator issue, backed by synthetic regressions, and re-run the commercial probe.
5. Do not upgrade status to boot/title/gameplay until a recognizable screen and responsive input are observed.

## Acceptance criteria

- Recognizable, stable title screen with nonblank, correctly arranged backgrounds/sprites.
- Game accepts Start; controllable first gameplay segment, with no recurring freezes or major corruption.
- Audio and save behavior checked separately.
- Native Hyprland/Wayland smoke test, including frame pacing, resize/fullscreen and input focus.

Related: [project status](../status.md), [commercial-ROM investigation tooling](../automation.md), [product roadmap](../roadmap.md).
