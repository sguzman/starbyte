# Super Mario World — compatibility investigation

**Current result (2026-10-09): a 2,400-frame real-ROM probe reaches Yoshi's House and shows a Right-correlated Mario sprite displacement.** After three verified Start presses navigate title, MARIO A file selection and 1-player selection, the Dinosaur Land welcome message appears. A short B pulse at frame 950 does not visibly dismiss it; a second B pulse at frame 1700 initiates a fade and enters a screen labeled **YOSHI'S HOUSE** by frame 1780. Mario remains near screen x=120 through frame 2100, then moves toward x=181 by frame 2120 during a 40-frame Right input. **All 2,400 frames complete without CPU errors**, but the scene's black background and disconnected white graphic fragments, combined with later unusual sprite placement, mean **accurate playable gameplay is not yet established**. Native Wayland controls, jumping, collision correctness, audio and saves remain unverified.

## Controller bit-layout correction (2026-10-09)

The 560-frame automatic-joypad retest completed without CPU errors. During frames 450–454, the CLI's host and latched joypad word both reported `$0008` for Start, and the title demonstration continued. That matched Starbyte's previous encoder but **not** SNES hardware: the correct auto-read word for Start alone is `$1000` (`$4218=$00, $4219=$10`), with buttons ordered BYsS UDLR AXlr 0000. The old implementation reversed the serial order and treated the low four signature bits as buttons. This is a verified encoding defect independent of input timing.

The core now encodes the standard SNES auto-read word and shifts serial data most-significant first. Unconnected controller ports 2–4 explicitly read zero. Unit tests cover all twelve button positions, serial ordering and the automatic-read registers. **The subsequent 700-frame real-ROM retest verified the intended title-to-file-selection transition.**

## Ninth headless probe: welcome dismissal, Yoshi's House and Right response (2,400 frames, 2026-10-09)

With code from main `c143f20`, the user ran `Super Mario World.zip` with `--frames 2400 --no-save-ram` and scheduled inputs `450:start;455:none;550:start;555:none;680:start;685:none;950:b;955:none;1400:right;1440:none;1700:b;1705:none;2100:right;2140:none`. The JSONL file has **2,400/2,400 successful frame records**; the archive includes 120 spaced PPM samples. The host and latched controller values agree at every scheduled input: B=`$8000`, Right=`$0100`, Start=`$1000`.

| Frame / interval | Observed behavior |
| --- | --- |
| 840–1699 | Dinosaur Land welcome text remains visible; no effective screen transition following B at 950 or Right at 1400 |
| 1700–1740 | The second B pulse at 1700 is followed by a fade from the welcome screen |
| 1760–1800 | A screen labeled **YOSHI'S HOUSE** appears, with Mario, HUD and other sprites |
| 1900–2100 | Mario's red-pixel sprite center stays near screen x=119–121 |
| 2100–2120 | A 40-frame Right pulse begins; Mario's red-pixel center shifts from x≈120 to x≈181 by frame 2120 |
| 2140–2380 | The sprite appears at unusual, inconsistent positions, with a final recurring position near x≈161, y≈125 and little horizontal progress |

**Interpretation:** title, file and player menus work, the introductory text can be dismissed with a subsequent B input, and a directional pulse correlates with movement of Mario's on-screen sprite. The exact reason the first B at 950 had no visible effect is not yet established; do not claim it was a missed hardware latch, since the controller register showed `$8000`. A **black background with fragmented/white tile-looking objects** persists in Yoshi's House, so graphics and/or game physics remain suspect. An image-derived screen coordinate is not the same as Mario's authoritative in-game position. Proper walking/jumping and successful level completion are **not verified**. Headless completion alone is not a playability certification.

**Diagnostic improvement:** optional side-effect-free canonical WRAM watches (`--watch-wram 7E0100,7E0094,7E0095,...` with `--frame-log`) will permit correlating game mode and position RAM with sampled sprites on a future local run. This is generic emulator observability, not a ROM patch or a game-specific workaround. Verify implementation via matching-commit CI before the next user probe.

## Eighth headless probe: first in-game scene after three Start presses (1,600 frames, 2026-10-09)

The user ran the same local `Super Mario World.zip` at main commit `a32e36b`, with `--frames 1600 --no-save-ram --controller1-events "450:start;455:none;550:start;555:none;680:start;685:none"`. All **1,600** JSONL frame attempts succeeded with no CPU error, and 80 sampled PPM images were inspected. The frame log captured `$1000` in the host and latched joypad word on each Start pulse, with `$0000` after release.

| Sampled frame | Observed game state |
| --- | --- |
| 440 | Animated Super Mario World title demo |
| 460 | File-selection screen, MARIO A/B/C marked EMPTY |
| 560 | 1 PLAYER GAME / 2 PLAYER GAME selection |
| 680 | Default 1-player option still on screen while third Start pulse takes effect |
| 720–740 | Black transition frames; CPU advances and does not error |
| 760 | Recognizable in-level HUD, score/time elements, grassy land, bushes |
| 780–820 | Mario character appears against the introductory terrain |
| 840–1580 | The introductory message beginning **Welcome! This is Dinosaur Land...** is legible and remains onscreen with Mario and terrain; no further user input is scheduled |

This is the first confirmed **save-slot selection, player-count selection and entry into an actual in-game scene** in Super Mario World. The in-game introductory message remains visible because the scripted controller timeline stops after frame 685; there is no evidence of an emulator hang or of failed subsequent input. We **must not** claim ordinary walking/jumping, overworld navigation, authentic DSP sound or reliable SRAM saves from this evidence. The exact ROM revision/hash is still unidentified. Original game images and proprietary ROM bytes remain exclusively in user-supplied local diagnostic artifacts; none are committed here.

**Next test:** replay the same initial Start pulses, then send an explicit B/A/Start dismissal input after the welcome text is visible (e.g. frame 980), sample later frames for a transition to the overworld, and only then test movement. A distinct user-input-driven transition is needed before marking normal gameplay control as verified.

## Seventh headless probe: real-ROM Start opens file selection (700 frames, 2026-10-09)

The user retested the unmodified locally held `Super Mario World.zip` with the controller-bit-order fix at main commit `14697c4`, using `--frames 700 --no-save-ram --controller1-events "450:start;455:none"`, JSONL diagnostics and sampled PPM framebuffers. The optimized run completed **700/700** frames with no CPU error. Input observations:

| Frame(s) | Host controller 1 bits | Auto-read latched bits | Rendered behavior |
| --- | --- | --- | --- |
| 440, 449 | `$0000` | `$0000` | Title demo continues |
| 450–454 | `$1000` | `$1000` | Start delivered in the correct SNES hardware bit; title transition begins |
| 455+ | `$0000` | `$0000` | Button released |
| 460–680 sampled | `$0000` | `$0000` | File-select menu stays visible, showing MARIO A/B/C as EMPTY and a blinking selection cursor |

The sampled image at frame 460 clearly shows the file-selection menu. The CPU resumes its normal loop (around `$00:806B–$00:806D`) and subsequent frames remain error-free. The menu's pointer periodically blinks, so this is **not** simply a stuck framebuffer. This is Starbyte's first verified **commercial-ROM interactive title navigation** milestone. The ROM's precise revision/hash remains unrecorded; no proprietary ROM bytes or screenshots are committed.

The next reproducible target is a **second Start press while MARIO A is selected**, followed by a separately timed 1-player selection if available, then the first demonstrably input-controlled game scene. Keep those distinct from automatic animations and confirm each by captured frames. `--no-save-ram` intentionally avoids persisting cartridge save data during these compatibility probes.

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

## Fourth headless probe: corrected master-clock timing, 360 requested

The user ran the corrected master-clock build (`44779da5`) against the same ZIP with per-frame logging on 2026-10-08. This run completed **38 frames** and returned an emulator error when attempting **frame 39**:

```text
unsupported opcode for 65816: 0xFF at 0x0004A0
```

The 39-line JSONL includes the final failed-frame record. The 24-sample budget captured only frames 1, 18, and 36; all were still black, consistent with early initialization having shifted to a more realistic master-clock schedule.

| Frame | End-frame CPU bank:PC | Stack | NMITIMEN | Display |
| --- | --- | --- | --- | --- |
| 36 | `$00:AAD4` | `$01F9` | `$00` | forced blank |
| 37 | `$00:806D` | `$01FF` | `$81` | forced blank cleared, still black |
| 38 | `$00:FFFF` | `$F886` | `$81` | 301 nonblack pixels (not recognizable screen) |
| 39 failed | `$00:04A0` | `$AFDE` | `$81` | frame 38 remains the last rendered buffer |

DMA bytes remained **9,952** and PPU writes **22,353** at failure. The ordinary frame log tells us the first major stack/control-flow corruption occurs *inside frame 38*, immediately after NMI is enabled and the display transitions out of forced blank. The final `$FF` opcode is a symptom: implementing additional CPU opcodes without determining why execution reached WRAM `$04A0` risks hiding the true defect.

### Next diagnostic capability

A new opt-in `run --trace-frame 38 --trace-out PATH` path runs prior frames normally, then records each successful instruction in **only frame 38** with before/after CPU registers, bus events, and a null opcode on interrupt service. The trace file includes a final status record even if the selected frame errors. This is specifically intended to find the **first** wrong stack write, RTI, vector fetch, or jump leading to `$00:FFFF`. The trace implementation has synthetic integration tests; commercial-frame conclusions must wait for the user's returned trace.

## Fifth probe: frame-38 CPU instruction trace — first proven divergence

On 2026-10-08 the user captured **22,220 CPU steps of frame 38** with `run --frames 38 --trace-frame 38`. The command completed; the trace includes a successful footer and **20,880 ordinary/interrupt steps before the decisive jump**.

**This is the first instruction-level, causal explanation of the runaway stack:**

1. At instruction 20,863, `JSR RunGameMode` (`$00:8072`) enters `$00:9322`. The mode value is **1** (Nintendo Presents). `JSL ExecutePtr` at `$00:9325` jumps to `$00:86DF`, pushing a three-byte long-call return address on stack.
2. On entry to `ExecutePtr` the processor status is **`$20`**, with the `X` index-width flag **clear** (16-bit index registers). This is incorrect for the indexed-return dispatcher, which expects `X=1` (8-bit Y) at entry. The public [SMWDisX `ExecutePtr` routine](https://github.com/IsoFrieze/SMWDisX/blob/master/bank_00.asm) uses `STY $03; PLY; STY $00; REP #$30; ...; PLA; STA $01; ...; JML [$00]`. Thus `PLY` must remove **one** byte from the long-call return before `REP` promotes the index to 16 bits.
3. With `X=0`, `PLY` at `$00:86E1` removes **two** bytes, incorrectly consuming return address `$9328`. The subsequent `PLA` removes the remaining bank byte `$00` **and the low byte `$74` of the outer `JSR` return**, leaving `$7400`. `STA $01` constructs pointer bank `$74`, and `JML [$00]` at instruction **20,880**, address **`$00:86F7`**, jumps to **`$74:0000`**.
4. At `$74:0000` the CPU reads `$00` (`BRK`); the native-mode BRK vector at `$00:FFE6` is `$FFFF`. A repeating `$00:FFFF` / `$00:0002` / `$00:0004` BRK path then pushes four more bytes per iteration until the stack wraps. This **fully explains the stack and PC explosion observed in frame 38**, without implicating the PPU as the initiating fault.

**Historical interpretation before the frame-37 trace:** the frame-38 evidence demonstrated **how** a cleared X flag corrupted the long-call dispatcher but did not yet explain **where** X was cleared. The first frame-38 instruction already has P=`$22` (X=0), and the CPU returns from an NMI with that same width before calling `RunGameMode`. In the preceding 39-frame diagnostic, **frame 36 ended with P=`$10` (X=1)** while **frame 37 ended with P=`$22` (X=0)**. Thus the flag transition is definitively bounded to frame 37 (which also contains the first game-mode update). The frame-37 trace below subsequently identified the incorrect native interrupt stack push at instruction 13,604; **do not** force X=1 at any game-specific address.

The subsequent bounded frame-37 trace (below) established the initiating defect without repeating long runs.

## Sixth probe: frame-37 instruction trace — root cause confirmed and fixed in core

On 2026-10-08 the user captured the entire **14,135-step frame 37** (`run --frames 37 --no-save-ram --trace-frame 37`). The trace completed successfully; it provides a direct explanation of why frame 38 enters the `$74:0000` jump/BRK spiral:

| Step (zero-based) | Instruction | 65816 processor state | Finding |
| --- | --- | --- | --- |
| 13,602–13,603 | `LDA #$81; STA $4200` (`$00:93F7`–`$00:93F9`) | P = `$B0`, native mode, X = 1 | Super Mario World enables NMI and automatic joypad polling as expected. |
| **13,604** | **Hardware NMI entry** at `$00:93FC` | **P = `$B0`; stacked status = `$A0` at `$01FA`** | **First demonstrably incorrect event:** CPU implementation removed bit `$10` from the stacked processor status in native mode. |
| 13,605–13,695 | `$00:816A`–`$00:82C2` | Normal NMI prologue, PPU updates, epilogue | Handler reads status and restores A/X/Y/DBR. The NMI code's temporary `REP`/`SEP` changes are intentional. |
| **13,696** | **`RTI`** at `$00:82C3` | **restores P = `$A0`**, returns to `$00:93FC` | X width has silently changed from 8-bit to 16-bit. |
| 13,697–13,700 | `RTS`; `STZ $10`; game-loop poll | Status becomes `$22` following a legitimate load | Main thread inherits the incorrect 16-bit index width; this makes next frame's `ExecutePtr` use an incorrect stack width. |

The error was in the general `Cpu65816::service_interrupt` implementation:

```rust
self.push_stack(bus, trace, self.registers.p & !0x10)?;
```

Bit `$10` is an emulation-mode B marker in the pushed processor status, **but the native-mode X width flag**. Clearing it during NMI/IRQ stack push was wrong. The implementation now constructs status according to CPU mode: clear the emulation-mode B marker on hardware interrupts, but **preserve the entire native P register**. Regression tests exercise native NMI, native IRQ and emulation NMI with stack bytes and `RTI`; a full-frame native NMI test also asserts preservation of M/X width. The fix is general-purpose, not game-specific.

**Post-fix Super Mario World gameplay has not yet been verified.** The proof here establishes the specific cause of frame-37 index-width loss and the resulting frame-38 wrong-bank jump. It does not prove other CPU, PPU, DMA or audio correctness.

## Seventh probe: native NMI fix confirmed by a real ROM, then missing LDA long,X

On 2026-10-08, after the native-status NMI correction passed CI, the user reran the locally held `Super Mario World.zip` with `run --frames 120 --no-save-ram --frame-log` and sampled PPM images. The run **completed 108 frames**, stopping on the first instruction of the following segment with:

```text
emulation failed at requested frame 109
unsupported opcode for 65816: 0xBF at 0x00B8B0
```

Compared with the previously failing frame 39, this confirms that the native NMI correction removed the specific bad-width `ExecutePtr` jump and runaway stack. The measured CPU PC stays in the expected `$00:806B`–`$00:806D` main-loop range from frames 38–100, with the stack pointer stable at **`$01FF`**. The end-frame framebuffer first contains nonblack pixels at frame **38**, and sampled frames 40 through 100 show a **stable, recognizable white "Nintendo Presents" logo on black**. Unlike earlier corrupt tile samples, this is a visibly coherent startup graphic, but a full title screen and gameplay have not been tested.

Frame 101 enters the next game-mode transition, with the stack at `$01F9` and CPU PC near `$00:B933`. During frames 101–108, the engine begins unpacking graphics; it stops at `$00:B8B0` during frame 109, with the stack still in the normal `$01FB` area. This is **not an arbitrary illegal-opcode location**. The public [SMWDisX disassembly of `CODE_00B8AD`](https://github.com/IsoFrieze/SMWDisX/blob/master/bank_00.asm) shows:

```asm
CODE_00B8AD:
    LDY.W #$0008
  - LDA.L MarioGraphics,X
```

At `$00:B8B0`, the `LDA.L ...,X` instruction uses the **legal opcode `$BF`** (65816 absolute long indexed X). Starbyte lacked this decoder entry. Generic support for both `LDA long` (`$AF`) and `LDA long,X` (`$BF`) is now committed, with synthetic tests covering accumulator width, bank carry, the independent DBR and 24-bit address wrap. No game-specific special case or proprietary ROM data was added.

**Evidence-bound conclusion:** The first major CPU corruption is fixed, and real startup rendering is stable through frame 100. The new failure is a specific missing instruction in an otherwise legitimate game graphics routine. The new opcode implementations still require a **post-fix ROM retest** before title/gameplay compatibility can be assessed.

## Eighth probe: $BF support clears graphics decompression; next missing $DF at frame 166

On 2026-10-08, with the `LDA long`/`LDA long,X` changes integrated, the user ran the same local ZIP through `run --frames 180 --no-save-ram --frame-log` with 18 bounded screenshot samples. **Frames 1–165 completed normally**; frame 166 failed cleanly on a different missing 65816 instruction:

```text
emulation failed at requested frame 166
unsupported opcode for 65816: 0xDF at 0x05DA2C
```

The user-supplied JSONL contains 165 successful frame records followed by the failed frame. The frame-166 CPU registers remain plausible for native subroutine execution: `PBR=$05`, `DBR=$05`, `PC=$DA2C`, `S=$01F9`, `P=$32`, `X=4`, `Y=4`. The CPU did not enter the previous BRK stack-corruption spiral; the resulting pointer is a **real code address** in [SMWDisX `bank_05.asm`](https://github.com/IsoFrieze/SMWDisX/blob/master/bank_05.asm):

```asm
CODE_05DA24:
    LDX.B #$04
    LDY.B #$04
    LDA.B [Layer1DataPtr],Y
    AND.B #$0F
CODE_05DA2C:
    CMP.L DATA_05D760,X
    BEQ CODE_05DA38
    DEX
    BPL CODE_05DA2C
```

`$DF` is the valid **CMP absolute-long,X** opcode. Starbyte lacked it. A general implementation of both `CMP long` (`$CF`) and `CMP long,X` (`$DF`) now uses the existing accumulator-width-sensitive comparison semantics, including 24-bit bank carry/wrap and flags, and carries synthetic regressions. A corresponding full-address-space word-access wrap fix was also committed.

The sampled startup frames remain visually coherent: the white "Nintendo Presents" logo is stable through about frame 130, fades from brightness 15 to 1 over frames 132–160, and becomes black at frame 161. A black frame after the fade **does not by itself mean another rendering failure**; the CPU moves into sound and then level-entry initialization. No title screen, first level, input or audio fidelity has been verified.

**Compatibility remains "startup progressing, not playable-verified."** Passing 165 frames and encountering a legitimate opcode is meaningful evidence of progress but does not constitute a playable game.

## Ninth probe response: close the opcode decoder gap systematically

The frame-166 error `$DF` is a valid indexed long comparison (`CMP.L`). Instead of asking the user to run their private ROM for each missing opcode, the CPU core's missing legal instruction entries were closed in synthetic batches:

- `CMP long` and `CMP long,X`, plus all previously missing CMP indirect and stack-relative forms; tests verify 8- and 16-bit comparison flags, bank carry, and 24-bit address wrap.
- Shared accumulator addressing logic fills the remaining `LDA`, `AND`, `ADC`, `EOR`, and `SBC` memory modes, with a table-driven synthetic test across 38 added cases and 16-bit arithmetic tests.
- Remaining `BIT`, `TSB`, `TRB`, `WDM`, `JML`, indirect `JMP`, `PEA`, `PEI`, `PER`, `MVN` and `MVP` modes are implemented. Indexed indirect `JSR` also now reads its pointer from the program bank, not bank zero.
- `WAI` and `STP` now have CPU idle-state handling, with PPU/APU master clocks continuing to advance even when no CPU bus instruction events occur.

All **256 opcode values** are handled by the dispatch table after these changes; tests and CI must establish correctness. Opcode coverage is **not** a claim of complete hardware emulation: cycle timing, decimal arithmetic, dummy reads, interrupt edge cases, and full video/audio accuracy still require work. No SNES ROM bytes were used as test fixtures or committed.

## Tenth probe: complete 300-frame run reaches the Super Mario World title screen

On 2026-10-08 / 2026-10-09 UTC, the user reran the same private LoROM `Super Mario World.smc` member through Starbyte after the all-256-opcode implementation. Command:

```text
starbyte run "Super Mario World.zip" --frames 300 --no-save-ram
  --frame-log /tmp/starbyte-smw-300.jsonl
  --frame-images-dir /tmp/starbyte-smw-300-frames
  --frame-image-every 10 --max-frame-images 30
```

**Outcome: all 300 of 300 frames completed successfully**, with zero logged emulation errors. The user supplied the full 300-row JSONL and 30 PPM captures (plus terminal log), which were independently inspected. This is the first actual title-screen observation and a substantial increase from the prior 165-frame run stopped by `CMP long,X`.

| Frame(s) | Direct observations |
| --- | --- |
| 1–37 | Blank screen during initialization and initial audio upload; NMI and PPU setup proceed. |
| 38–131 | Coherent, recognizable Nintendo Presents text on black background. |
| 132–161 | Opening fades to black as PPU brightness declines. |
| 162–212 | Dark loading/transition phase, CPU moves through routines with a normal, bounded stack and DMA/PPU work. |
| 213–214 | Forced blank disabled, title background enabled, brightness still zero. |
| **215** | **First title image pixels**: 19,235 nonblack pixels, eight RGB colors. Screenshot contains legible colored `SUPER MARIO WORLD` logo, patterned border and `© 1990, 1991 Nintendo` below. |
| 215–245 | Smooth brightness fade-in from 1 to 15; stable framebuffer contents and plausible CPU main-loop state; stack returns to `$01FF`. |
| 245–285 | Full brightness. Coherent title image remains stable while game code continues. |
| **286–300** | **Title scene changes:** 23,273 nonblack pixels and 16 colors, with additional green foreground graphics visible in screenshots 290–300. This is evidence of the beginning of a title animation, not verified controller-responsive gameplay. |

The final CPU sample at frame 300 has `PBR:PC=$00:CAE7`, `S=$01FD`, `P=$E0`, and `DBR=$00` while title-scene code executes. Total transferred DMA bytes reach 259,293; no unsupported opcode and no runaway BRK stack sequence occurred. Framebuffer hashes change 32 times during the 300-frame run. The final display is unblanked, brightness 15, Mode 1, with main-screen enables `$15`.

The accompanying ZIP member title and LoROM header were identified, but the precise ROM digest/region revision has **not** been independently recorded. No commercial ROM bytes or screenshots were committed to this repository; the supplied diagnostics remain external.

**Compatibility milestone: title-screen rendering confirmed; gameplay not yet tested.** Synthetic CPU opcode coverage and bounded headless title frames are not enough to establish accurate input handling, sound effects/music, physics, title-demo progression or save reliability.

## Eleventh probe: 900-frame title demo and missing VBlank joypad polling

On 2026-10-09 UTC the user ran `starbyte run "Super Mario World.zip" --frames 900 --no-save-ram --controller1-events "450:start;455:none"`, saving 900 JSONL frame records and 46 PPM screenshot samples. The entire run **completed successfully** with no CPU halt, unsupported opcode, host crash or corrupt-stack regression. The input timeline applied Start before frame 450 and released it before frame 455; the CLI log confirms both updates.

Captured frames 320–900 show **substantial, coherent title-demo gameplay animation** within the Super Mario World title border: Mario, Yoshi, enemies, coins, foreground terrain, and horizontal scrolling. At frames 440, 460, 480, 520 and later, the animation proceeds but no visible file-selection screen appears after the Start pulse. The title sprite/demo is the *scripted attract sequence*, not proof that the host can control Mario or launch a first playable level.

The reported `irq_timer.nmitimen` value during frames 38–900 is **129 decimal (`$81`)**: VBlank NMI enabled (bit 7), **automatic controller read enabled (bit 0)**. Starbyte's pre-fix `SystemBus` had working *manual* controller latching (`$4016` strobe) and returned `latched1` at `$4218/$4219`, but **never automatically latched on VBlank**, despite bit 0 being enabled. Thus the game's automatically polled pad values could remain unchanged even while the host `controller1` state changed. This is a specific emulation omission, not just an unproven theory about input timing.

A general hardware-level fix has been committed:
- Begin automatic controller-1 sampling each VBlank only when NMITIMEN bit 0 is set.
- Preserve sampled button bits and update `$4218/$4219` when the approximate 4,224-master-clock automatic read interval completes. `$4212` bit 0 reports busy during that interval.
- Keep explicit `$4016` serial latching available. New synthetic tests exercise Start, disabled polling, the busy interval, repeated VBlank updates and a snapshot unaffected by a host release during polling.
- Expose a read-only `joypad` record in each opted-in `starbyte.frame_log.v1` entry (`host_controller1_bits`, `latched_controller1_bits`, `auto_read_busy`) so future tests can distinguish host delivery from what the emulated game can see.

**Next evidence needed:** rerun a bounded scheduled Start pulse after CI passes and verify both that `$4218` reflects Start during the correct frames and that the title/demo transitions visibly into an interactive screen. Do not mark gameplay verified solely from animation or from accepted input bits. The busy interval is modeled approximately; precise sub-scanline timing remains an open emulator fidelity task.

## Next evidence needed

1. **Confirmed:** three Start presses reach the Dinosaur Land welcome; a later B press enters Yoshi's House; a Right pulse correlates with Mario sprite movement. Next, compare canonical game-mode and Mario-position WRAM values against the controller timeline, and exercise short Left/Right/Jump pulses as separate tests.
2. Investigate the black/fragmented Yoshi's House graphics and abnormal later sprite positioning before claiming normal movement, collision physics or playable levels.
3. Investigate DSP music/audio, native Wayland frame pacing and SRAM independently; synthetic samples are not faithful game sound.
4. Record the exact local cartridge checksum/revision without committing or distributing ROM bytes.
5. Add synthetic regressions for newly confirmed hardware faults and upgrade compatibility only when demonstrated.

## Acceptance criteria

- Recognizable, stable title screen with nonblank, correctly arranged backgrounds/sprites.
- Game accepts Start; controllable first gameplay segment, with no recurring freezes or major corruption.
- Audio and save behavior checked separately.
- Native Hyprland/Wayland smoke test, including frame pacing, resize/fullscreen and input focus.

Related: [project status](../status.md), [commercial-ROM investigation tooling](../automation.md), [product roadmap](../roadmap.md).
