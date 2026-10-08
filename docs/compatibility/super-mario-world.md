# Super Mario World — compatibility investigation

**Current result: startup graphics appear, but execution and rendering are corrupt; a 360-frame probe aborted from a host stack overflow after frame 340.** The first-frame trace confirms a real audio upload, not an initial deadlock. Playability remains unverified. This document records observations separately from suspected causes.

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



## Second headless probe: 360 requested frames

The user ran `starbyte-cli run` with `--frames 360 --no-save-ram --frame-log`, sampling PPM screenshots every 18 frames, and supplied the resulting archive and terminal log on 2026-10-08. The process **aborted with a Rust stack overflow**, rather than returning a recoverable frame error. The frame log was flushed through **frame 340**.

| Frame / range | Observed outcome |
| --- | --- |
| 1–100 | Long SPC upload and forced-blank initialization, matching the earlier trace. |
| 101–147 | CPU exits upload; PPU register write count rises during continued forced blank. |
| 148 | `$2100` forced blank clears, brightness reaches 15. |
| 149 | First nonblack framebuffer (301 nonblack pixels); CPU PC ends at `$00:0002`. |
| 150 onward | End-of-frame CPU PC frequently `$00:FFFF`; framebuffer alternates between black and visibly corrupted scenes. |
| 180 and 270 sampled images | Recognizable **"Nintendo Presents"** startup lettering, with distorted or missing background graphics. |
| 252 and 306 sampled images | Large repeated/striped tile patterns instead of stable, correctly composed backgrounds. |
| 340 | 4,443,789 cumulative PPU register writes; `$00:FFFF` end-frame PC; forced blank is clear but framebuffer black. |
| Frame 341 attempt | Host stack overflow, process aborted (SIGABRT); no completed frame 341 record. |

**Conclusions grounded in user evidence:** Starbyte reaches a recognizable part of the real startup sequence, so initial sound upload is not the overall blocker. CPU execution becomes abnormal and rendering is severely corrupt before the stack overflow. The data alone cannot identify the exact offending instruction or prove which CPU/PPU/DMA defect causes the first divergence.

### Correctness defects identified from source

While investigating the crash, source review identified several independent hardware-model defects:

- General DMA treated its **A-bus** accesses as arbitrary CPU MMIO reads/writes. In particular, reverse DMA could write `$420B` and recursively invoke `execute_dma` until overflowing the host stack. The new regression configures that exact reverse-transfer case and verifies it finishes once.
- PPU/CPU MMIO was inadvertently decoded in unrelated cartridge banks, rather than just SNES system banks. This can corrupt memory access and trigger unintended side effects.
- Timer IRQs were raised on **every scanline crossing** whenever either H/V IRQ bit was enabled, without respecting the programmed `$4207`–`$420A` timer coordinates. An approximate H-only, V-only and combined comparator now replaces that unconditional behavior; precise hardware-cycle timing remains future work.

These are **genuine hardware-model corrections**, but they have not yet been validated by a post-fix run of this commercial ROM. Successful synthetic tests do not demonstrate that the game is playable or that the exact cause of the observed SIGABRT is removed.

## Third headless probe: 360 requested frames after DMA/IRQ fixes

On 2026-10-08, the user retested the same locally held archive against Starbyte after the DMA/MMIO and timer-IRQ corrections, with `--no-save-ram` and flushed JSONL. This probe **completed 150 frames and returned a structured CPU error while attempting frame 151**:

```text
unsupported opcode for 65816: 0x0B at 0xD980BC
```

There was **no host stack overflow** in this run. The frame log contains 151 records (150 successful frames plus one failed-frame record), documenting:

- Frame 148: CPU PC `$00:806D`, native mode, stack `$01FF`, NMITIMEN `$81`, DMA 9,248 bytes, PPU writes 21,591.
- Frame 149: CPU PC `$00:0002`, stack `$012A`, first 301 nonblack pixels, DMA 9,952 bytes, PPU writes 22,333.
- Frame 150: CPU PC `$00:FFFF`, stack `$ECCA`, with 301 nonblack pixels and 22,353 PPU writes.
- Error during frame 151: PC `$D9:80BC`, stack `$DA9E`, invalidly accessed instruction `0x0B`. Note: `0x0B` is the **legal 65816 PHD instruction** and Starbyte's decoder lacked it. Implementing PHD eliminates that specific decoder gap, but does not establish that the unexpected bank or stack corruption was repaired.

**Interpretation:** The previous unbounded recursive-DMA stack overflow did not recur, but CPU control flow already diverges during the first real NMI/graphics update. Treat the late unsupported opcode as a symptom until an instruction-level trace identifies the first wrong branch, interrupt vector, stack operation, or memory access.

### Independently identified timing defect

The timing implementation previously treated **each CPU master clock as a whole PPU dot**, using 341 clocks rather than approximately 1,364 master clocks per NTSC scanline. Real SNES timing distinguishes 4 master clocks per dot (see [SNESdev timing](https://snes.nesdev.org/wiki/Timing)). This would advance VBlank/NMI and HDMA cadence about four times too quickly relative to the 65816 execution trace. The fix accumulates sub-dot master clocks, progresses a PPU dot per four, converts H/V timer comparisons accordingly, and raises the guarded instruction-per-frame budget. Precise long-dot variations, DRAM-refresh stalls and exact CPU bus cycle penalties are still outside the current model.

The 65816 core also now implements `PHD`/`PLD`, forces interrupt vector targets to bank zero in both modes, and keeps emulation-mode stack transfers inside page `$01`. These are general hardware correctness changes; **none was validated on this user's ROM at documentation time**.

## Next evidence needed

1. Re-run a bounded 360-frame probe after pulling the 65816 and master-clock/dot corrections; report the last successfully completed frame, any error, and whether recognizable graphics advance.
2. If control flow still leaves the expected startup/NMI routines, capture **frames 149–150** using `compliance commercial-record --frames 150 --trace-from-frame 148` (or the equivalent earlier boundary after corrected timing). Look for the first wrong interrupt, stack access, or transfer, not merely the final unsupported opcode.
3. Record the local cartridge digest and precise revision; the ZIP filename is not sufficient evidence.
4. Fix additional demonstrated core defects with copyright-free regressions before another ROM probe.
5. Do not mark title/gameplay verified until the startup/title screen is stable and controller input is responsive.

## Acceptance criteria

- Recognizable, stable title screen with nonblank, correctly arranged backgrounds/sprites.
- Game accepts Start; controllable first gameplay segment, with no recurring freezes or major corruption.
- Audio and save behavior checked separately.
- Native Hyprland/Wayland smoke test, including frame pacing, resize/fullscreen and input focus.

Related: [project status](../status.md), [commercial-ROM investigation tooling](../automation.md), [product roadmap](../roadmap.md).
