//! Register-driven pixel regressions for early SNES Mode 1 scene composition.
//! These are synthetic, permissively generated fixtures; no commercial ROM data.

use starbyte_core::ppu::{FrameBuffer, Ppu};

fn palette(ppu: &mut Ppu, slot: u8, value: u16) {
    let [low, high] = value.to_le_bytes();
    ppu.write_register(0x2121, slot);
    ppu.write_register(0x2122, low);
    ppu.write_register(0x2122, high);
}

fn vram_word(ppu: &mut Ppu, byte_address: u16, value: u16) {
    assert_eq!(byte_address & 1, 0);
    let address = byte_address / 2;
    let [address_low, address_high] = address.to_le_bytes();
    let [low, high] = value.to_le_bytes();
    ppu.write_register(0x2115, 0x80); // Word address increments after high-byte write.
    ppu.write_register(0x2116, address_low);
    ppu.write_register(0x2117, address_high);
    ppu.write_register(0x2118, low);
    ppu.write_register(0x2119, high);
}

fn pixel(frame: &FrameBuffer, x: usize, y: usize) -> [u8; 4] {
    let start = (y * frame.width() + x) * 4;
    frame.pixels()[start..start + 4].try_into().unwrap()
}

fn sample_mode1_bg1_bg2() -> Ppu {
    let mut ppu = Ppu::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 0, 0);
    palette(&mut ppu, 1, 0x001F); // Red BG1 palette 0, color 1.
    palette(&mut ppu, 17, 0x03E0); // Green BG2 palette 1, color 1.
    vram_word(&mut ppu, 0x0000, 0); // BG1 map -> tile 0, palette 0, low priority.
    vram_word(&mut ppu, 0x0800, 0x2400); // BG2 -> palette 1, high priority.
    vram_word(&mut ppu, 0x2000, 0x0080); // BG1 character pixel 0,0 = 1.
    vram_word(&mut ppu, 0x4000, 0x0080); // BG2 character pixel 0,0 = 1.
    ppu.write_register(0x2105, 0x01);
    ppu.write_register(0x2108, 0x04); // BG2 map starts at byte 0x0800.
    ppu.write_register(0x210B, 0x21); // BG1 chars 0x2000, BG2 chars 0x4000.
    ppu.write_register(0x212C, 0x03);
    ppu
}

#[test]
fn mode1_tile_priority_overrides_fixed_layer_order() {
    let mut ppu = sample_mode1_bg1_bg2();
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [0, 248, 0, 255]);

    // When both tiles are low, BG1 wins over BG2.
    vram_word(&mut ppu, 0x0800, 0x0400);
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);

    // When both tiles are high, BG1 wins over BG2 again.
    vram_word(&mut ppu, 0x0800, 0x2400);
    vram_word(&mut ppu, 0x0000, 0x2000);
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);
}

#[test]
fn sprite_priority_places_objects_behind_and_in_front_of_bg_tiles() {
    let mut ppu = sample_mode1_bg1_bg2();
    palette(&mut ppu, 129, 0x7C00); // Sprite palette 0, color 1 = blue.
    vram_word(&mut ppu, 0x4000, 0x0080);
    ppu.write_register(0x2101, 0x01); // Sprite tiles at byte 0x4000.
    ppu.write_register(0x2102, 0);
    ppu.write_register(0x2103, 0);
    for value in [0, 0, 0, 0] {
        ppu.write_register(0x2104, value);
    }
    ppu.write_register(0x212C, 0x11); // BG1 + sprites.

    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);

    ppu.write_register(0x2102, 1); // OAM attribute byte at offset 3.
    ppu.write_register(0x2103, 0);
    ppu.write_register(0x2104, 0);
    ppu.write_register(0x2104, 0x30); // Sprite priority 3.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 248, 255]);
}

#[test]
fn mode1_bg3_high_priority_switches_foreground_hud_order() {
    let mut ppu = Ppu::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 1, 0x001F); // BG1 red.
    palette(&mut ppu, 5, 0x7C00); // BG3 2bpp palette 1 blue.
    vram_word(&mut ppu, 0x0000, 0x0000);
    vram_word(&mut ppu, 0x1000, 0x2400); // BG3 high tile, palette 1.
    vram_word(&mut ppu, 0x2000, 0x0080); // BG1 4bpp character.
    vram_word(&mut ppu, 0x6000, 0x0080); // BG3 2bpp character.
    ppu.write_register(0x2109, 0x08);
    ppu.write_register(0x210B, 0x01);
    ppu.write_register(0x210C, 0x03);
    ppu.write_register(0x212C, 0x05);
    ppu.write_register(0x2105, 0x01);
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);

    ppu.write_register(0x2105, 0x09);
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 248, 255]);
}

#[test]
fn sixteen_pixel_characters_select_four_tiles_and_flip_as_a_unit() {
    let mut ppu = Ppu::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 1, 0x001F); // red
    palette(&mut ppu, 2, 0x03E0); // green
    palette(&mut ppu, 3, 0x7C00); // blue
    palette(&mut ppu, 4, 0x03FF); // yellow
    vram_word(&mut ppu, 0x0000, 0x0000);
    vram_word(&mut ppu, 0x2000, 0x0080); // tile 0, color 1
    vram_word(&mut ppu, 0x2020, 0x8000); // tile 1, color 2
    vram_word(&mut ppu, 0x2200, 0x8080); // tile 16, color 3
    vram_word(&mut ppu, 0x2230, 0x0080); // tile 17, plane2, color 4
    ppu.write_register(0x2105, 0x11); // Mode 1 BG1 uses 16x16 characters.
    ppu.write_register(0x210B, 0x01);
    ppu.write_register(0x212C, 0x01);
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);
    assert_eq!(pixel(&frame, 8, 0), [0, 248, 0, 255]);
    assert_eq!(pixel(&frame, 0, 8), [0, 0, 248, 255]);
    assert_eq!(pixel(&frame, 8, 8), [248, 248, 0, 255]);

    vram_word(&mut ppu, 0x0000, 0x4000); // Flip entire character horizontally.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 15, 0), [248, 0, 0, 255]);
    assert_eq!(pixel(&frame, 7, 0), [0, 248, 0, 255]);
}

#[test]
fn vmain_increment_port_and_step_size_follow_register_bits() {
    let mut ppu = Ppu::default();
    // Reset VMAIN = 0: auto-increment on low-byte writes.
    ppu.write_register(0x2118, 0xAA);
    ppu.write_register(0x2118, 0xBB);
    assert_eq!(ppu.vram()[0], 0xAA);
    assert_eq!(ppu.vram()[2], 0xBB);

    ppu.write_register(0x2115, 0x01); // Low-byte access, increment by 32 words.
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);
    ppu.write_register(0x2118, 0xCC);
    ppu.write_register(0x2118, 0xDD);
    assert_eq!(ppu.vram()[0], 0xCC);
    assert_eq!(ppu.vram()[64], 0xDD);

    ppu.write_register(0x2115, 0x80); // High-byte access increments by one word.
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);
    ppu.write_register(0x2118, 0x01);
    ppu.write_register(0x2119, 0x02);
    ppu.write_register(0x2118, 0x03);
    ppu.write_register(0x2119, 0x04);
    assert_eq!(&ppu.vram()[..4], &[1, 2, 3, 4]);
}

#[test]
fn vmain_rearranges_low_address_bits_for_planar_vram_streaming() {
    let mut ppu = Ppu::default();
    let cases = [
        (0x84, 0x0003_u16, 0x0018_usize), // 8-bit rearrangement.
        (0x88, 0x0040_u16, 0x0001_usize), // 9-bit rearrangement.
        (0x8C, 0x0080_u16, 0x0001_usize), // 10-bit rearrangement.
    ];
    for (vmain, raw_word, translated_word) in cases {
        let [low, high] = raw_word.to_le_bytes();
        ppu.write_register(0x2115, vmain);
        ppu.write_register(0x2116, low);
        ppu.write_register(0x2117, high);
        ppu.write_register(0x2118, 0x5E);
        ppu.write_register(0x2119, 0x6F);
        let address = translated_word * 2;
        assert_eq!(&ppu.vram()[address..address + 2], &[0x5E, 0x6F]);
    }
}

#[test]
fn vram_read_latch_prefetches_on_vmadd_and_refetches_before_increment() {
    let mut ppu = Ppu::default();
    vram_word(&mut ppu, 0x0000, 0x1234);
    vram_word(&mut ppu, 0x0002, 0x5678);
    ppu.write_register(0x2115, 0x80); // High-byte read increments VMADD.
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);
    assert_eq!(ppu.read_data_register(0x2139), 0x34);
    assert_eq!(ppu.read_data_register(0x213A), 0x12);
    // On the incrementing read, the old address refills the latch.
    assert_eq!(ppu.read_data_register(0x2139), 0x34);
    assert_eq!(ppu.read_data_register(0x213A), 0x12);
    assert_eq!(ppu.read_data_register(0x2139), 0x78);
    assert_eq!(ppu.read_data_register(0x213A), 0x56);

    ppu.write_register(0x2115, 0x00); // Low-byte read increments instead.
    ppu.write_register(0x2116, 0x00);
    ppu.write_register(0x2117, 0x00);
    assert_eq!(ppu.read_data_register(0x2139), 0x34);
    assert_eq!(ppu.read_data_register(0x2139), 0x34);
    assert_eq!(ppu.read_data_register(0x2139), 0x78);
}

#[test]
fn ppu_cpu_stream_reads_advance_oam_and_cgram() {
    let mut ppu = Ppu::default();
    ppu.write_register(0x2102, 0);
    ppu.write_register(0x2103, 0);
    ppu.write_register(0x2104, 0x12);
    ppu.write_register(0x2104, 0x34);
    ppu.write_register(0x2102, 0);
    ppu.write_register(0x2103, 0);
    assert_eq!(ppu.read_data_register(0x2138), 0x12);
    assert_eq!(ppu.read_data_register(0x2138), 0x34);

    palette(&mut ppu, 0, 0x1234);
    ppu.write_register(0x2121, 0);
    assert_eq!(ppu.read_data_register(0x213B), 0x34);
    assert_eq!(ppu.read_data_register(0x213B), 0x12);
}

#[test]
fn bus_vram_reads_observe_incrementing_latch_semantics() {
    use starbyte_core::bus::Bus;
    use starbyte_core::system::SystemBus;

    let mut bus = SystemBus::default();
    bus.write(0x002115, 0x80);
    bus.write(0x002116, 0);
    bus.write(0x002117, 0);
    bus.write(0x002118, 0xAB);
    bus.write(0x002119, 0xCD);
    bus.write(0x002116, 0);
    bus.write(0x002117, 0);
    assert_eq!(bus.read(0x002139), 0xAB);
    assert_eq!(bus.read(0x00213A), 0xCD);
    assert_eq!(bus.read(0x002139), 0xAB);
}

#[test]
fn bg_screen_and_tile_bases_use_vram_word_units() {
    let mut ppu = Ppu::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 1, 0x001F);
    vram_word(&mut ppu, 0x0800, 0x0000); // Tilemap at 0x400 VRAM words.
    vram_word(&mut ppu, 0x4000, 0x0080); // Tiles at 0x2000 VRAM words.
    ppu.write_register(0x2105, 0x01);
    ppu.write_register(0x2107, 0x04); // BG1 map base = byte 0x0800.
    ppu.write_register(0x210B, 0x02); // BG1 character base = byte 0x4000.
    ppu.write_register(0x212C, 0x01);
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);
}

#[test]
fn bg_mosaic_snaps_pixels_to_screen_anchored_blocks() {
    let mut ppu = sample_mode1_bg1_bg2();
    ppu.write_register(0x212C, 0x01); // Show BG1 only.
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);
    assert_eq!(pixel(&frame, 1, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&frame, 0, 1), [0, 0, 0, 255]);

    ppu.write_register(0x2106, 0x11); // BG1 only, 2x2 mosaic blocks.
    ppu.render_frame(&mut frame);
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        assert_eq!(pixel(&frame, x, y), [248, 0, 0, 255]);
    }
    assert_eq!(pixel(&frame, 2, 0), [0, 0, 0, 255]);
}

/// SNES OBJ tile numbers wrap horizontally every 16 cells; Y positions
/// wrap modulo 256 even if the visible image is only 224 scanlines.
#[test]
fn large_sprite_wraps_character_row_and_vertical_position() {
    let mut ppu = Ppu::default();
    let mut frame = FrameBuffer::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 129, 0x001F); // sprite red, color 1

    // OBJ name-base $4000 bytes, tile $F0 at $5E00. The right-hand
    // character of a sprite beginning with $FF must wrap to $F0.
    vram_word(&mut ppu, 0x5E00, 0x0080); // tile F0 row 0 first pixel
    vram_word(&mut ppu, 0x5E02, 0x0080); // tile F0 row 1 first pixel
    ppu.write_register(0x2101, 0x01);
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x00);
    for value in [16, 0, 0xFF, 0x30] {
        ppu.write_register(0x2104, value);
    }
    // Sprite 0 uses the large 16x16 size from OBJSEL.
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x01);
    ppu.write_register(0x2104, 0x02);
    ppu.write_register(0x212C, 0x10);

    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 24, 0), [248, 0, 0, 255]);

    // X high bit means negative X. An OBJ at -8 with the same FF tile
    // must expose its right-hand tile F0 at screen pixel 0.
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x00);
    for value in [248, 0, 0xFF, 0x30] {
        ppu.write_register(0x2104, value);
    }
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x01);
    ppu.write_register(0x2104, 0x03); // 9-bit negative X and large-size bit.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);

    // The same object at Y=255 wraps its second row to screen Y=0.
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x00);
    for value in [16, 255, 0xFF, 0x30] {
        ppu.write_register(0x2104, value);
    }
    ppu.write_register(0x2102, 0x00);
    ppu.write_register(0x2103, 0x01);
    ppu.write_register(0x2104, 0x02); // Restore nonnegative X while keeping large sprite.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 24, 0), [248, 0, 0, 255]);
}

#[test]
fn background_window_masks_pixels_and_supports_inversion() {
    let mut ppu = sample_mode1_bg1_bg2();
    palette(&mut ppu, 0, 0x03E0); // Backdrop is green.
    vram_word(&mut ppu, 0x2000, 0x00FF); // Eight opaque red pixels.
    ppu.write_register(0x212C, 0x01); // BG1 only.
    ppu.write_register(0x2126, 2);
    ppu.write_register(0x2127, 4);
    ppu.write_register(0x2123, 0x02); // BG1 window 1 enabled.
    ppu.write_register(0x212E, 0x01); // BG1 window on main screen.
    let mut frame = FrameBuffer::default();
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [248, 0, 0, 255]);
    assert_eq!(pixel(&frame, 2, 0), [0, 248, 0, 255]);
    assert_eq!(pixel(&frame, 4, 0), [0, 248, 0, 255]); // Right edge inclusive.
    assert_eq!(pixel(&frame, 5, 0), [248, 0, 0, 255]);

    ppu.write_register(0x2123, 0x03); // Invert window 1.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [0, 248, 0, 255]);
    assert_eq!(pixel(&frame, 2, 0), [248, 0, 0, 255]);

    ppu.write_register(0x2123, 0x00); // No windows selected.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 2, 0), [248, 0, 0, 255]);
}

#[test]
fn background_window_logic_supports_or_and_xor_xnor() {
    let mut ppu = sample_mode1_bg1_bg2();
    palette(&mut ppu, 0, 0x03E0);
    vram_word(&mut ppu, 0x2000, 0x00FF);
    ppu.write_register(0x212C, 0x01);
    ppu.write_register(0x212E, 0x01);
    ppu.write_register(0x2123, 0x0A); // BG1: both windows enabled.
    ppu.write_register(0x2126, 1);
    ppu.write_register(0x2127, 3);
    ppu.write_register(0x2128, 3);
    ppu.write_register(0x2129, 5);
    let mut frame = FrameBuffer::default();
    let red = [248, 0, 0, 255];
    let green = [0, 248, 0, 255];

    ppu.write_register(0x212A, 0); // OR: mask union.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 1, 0), green);
    assert_eq!(pixel(&frame, 3, 0), green);
    assert_eq!(pixel(&frame, 5, 0), green);
    assert_eq!(pixel(&frame, 6, 0), red);

    ppu.write_register(0x212A, 1); // AND: only overlap.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 1, 0), red);
    assert_eq!(pixel(&frame, 3, 0), green);
    assert_eq!(pixel(&frame, 5, 0), red);

    ppu.write_register(0x212A, 2); // XOR: exclude overlap.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 1, 0), green);
    assert_eq!(pixel(&frame, 3, 0), red);
    assert_eq!(pixel(&frame, 5, 0), green);

    ppu.write_register(0x212A, 3); // XNOR: overlap and outside both.
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), green);
    assert_eq!(pixel(&frame, 1, 0), red);
    assert_eq!(pixel(&frame, 3, 0), green);
}

#[test]
fn object_window_masks_high_priority_sprite_pixels() {
    let mut ppu = Ppu::default();
    let mut frame = FrameBuffer::default();
    ppu.write_register(0x2100, 0x0F);
    palette(&mut ppu, 0, 0x03E0); // Backdrop green.
    palette(&mut ppu, 129, 0x7C00); // OBJ blue.
    vram_word(&mut ppu, 0x4000, 0x00FF);
    ppu.write_register(0x2101, 0x01);
    ppu.write_register(0x2102, 0);
    ppu.write_register(0x2103, 0);
    for byte in [0, 0, 0, 0x30] {
        ppu.write_register(0x2104, byte);
    }
    ppu.write_register(0x212C, 0x10); // OBJ main screen.
    ppu.write_register(0x212E, 0x10); // OBJ window on main.
    ppu.write_register(0x2125, 0x02); // OBJ window 1.
    ppu.write_register(0x2126, 2);
    ppu.write_register(0x2127, 4);
    ppu.render_frame(&mut frame);
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 248, 255]);
    assert_eq!(pixel(&frame, 3, 0), [0, 248, 0, 255]);
    assert_eq!(pixel(&frame, 7, 0), [0, 0, 248, 255]);
}
