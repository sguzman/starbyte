# PPU support and verification

Starbyte's framebuffer is currently a software-rendered 256×224 RGBA image. Only documented, synthetic behaviors below have been exercised in hosted CI. It is **not** a claim of overall hardware correctness or commercial playability.

## Supported baseline

| Feature/registers | Current implementation | Known limitations |
| --- | --- | --- |
| INIDISP `$2100` | Forced blank and whole-frame brightness 0–15 | Mid-scanline fades and color timing |
| BGMODE `$2105` | Mode 0 2bpp and Mode 1 4/4/2bpp backgrounds with per-tile priority and 8×8/16×16 tile selection | Modes 2–7, offsets-per-tile and full mode-specific priority |
| BGnSC `$2107–0A`, BGnNBA `$210B–0C` | Tilemap quadrants, character bases in VRAM word units, tile flips, palettes, transparent pixel 0 | Raster-time map changes and full timing |
| BGnHOFS/VOFS `$210D–14` | Shared BG scroll byte latches, effective 10-bit horizontal and vertical offsets, background wrapping | Mode 7 separate latches; actual scanline/HDMA raster captures |
| MOSAIC `$2106` | Selected backgrounds sample screen-anchored pixel blocks | Raster changes |
| OBJSEL/OAM `$2101–04` | OAM writes, sprite sizes, palettes, relative priorities, flip bits, signed X and wraparound Y, tile-row wrap | Per-scanline sprite/tile limits, rotation priority, overflow flags |
| VRAM `$2115–19, $2139–3A` | VMAIN port increment selection, 1/32/128 strides, remapping, prefetch read latch | Active-display access restrictions and bus/open-bus minutiae |
| CGRAM `$2121–22, $213B` | Streaming color writes/reads, red low five bits, green middle, blue high bits | Partial hardware read latch and open-bus behavior |
| TM/TMW/window `$2123–2C, $212E` | Main-screen BG/OBJ layer selection and masking; two horizontal windows, inversion, inclusive bounds and OR/AND/XOR/XNOR | Subscreen TSW, color-window clipping, per-scanline changes |
| CGWSEL/CGADSUB/COLDATA `$2130–32` | Selected-layer fixed-color add/subtract/half in 5-bit channels; OBJ palettes 0–3 excluded | Subscreen blending, color-window regions/forced black; unsupported combinations currently leave main screen unblended |
| SETINI `$2133` | Register storage only | Overscan/interlace/hires and picture geometry |

## Reproducible evidence

- `crates/starbyte-core/tests/ppu_compositing.rs`: synthetic pixel checks covering layers, sprites, 16×16 quadrants, VRAM ports, CGRAM/brightness, windows, mosaic and scroll.
- `crates/starbyte-core/tests/cpu_to_ppu_boot.rs`: source-generated LoROM executes actual 65816 writes through the system bus to create a visible BG1 pixel; the second fixture exercises fixed-color math. This is not proprietary software.
- `crates/starbyte-core/src/ppu/mod.rs`: unit-level framebuffer, DMA-visible PPU register, and palette tests.
- The CLI `run --report-json` reports PPU register activity, display mode, brightness, enabled layers, frame hash, nonblack-pixel count and color diversity for user-supplied ROM probes. These are diagnostic signals, **not** validation of correct visuals.

## Work that would visibly improve games

- Confirm actual Super Mario World title/menu output; use the new run reports and screenshots to locate missing layers and timing behavior.
- Support the remaining subscreen / color math window behaviors that affect translucent sprites, water, and layered scenes.
- Improve scanline-accurate display state for HDMA and partial-frame changes; record tests showing before/after behavior.
- Extend Mode 2–7 rendering only as real compatibility examples justify it.
- Measure software-rendering frame time alongside actual game behavior before committing to expensive filters.

For underlying register reference, see [SNESdev PPU registers](https://snes.nesdev.org/wiki/PPU_registers), [windowing](https://snes.nesdev.org/wiki/Windowing), and [color math](https://snes.nesdev.org/wiki/Color_math).
