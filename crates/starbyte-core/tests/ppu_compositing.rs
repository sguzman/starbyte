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
    palette(&mut ppu, 0, 0);
    palette(&mut ppu, 1, 0x7C00); // Red BG1 palette 0, color 1.
    palette(&mut ppu, 17, 0x03E0); // Green BG2 palette 1, color 1.
    vram_word(&mut ppu, 0x0000, 0); // BG1 map -> tile 0, palette 0, low priority.
    vram_word(&mut ppu, 0x0800, 0x2400); // BG2 -> palette 1, high priority.
    vram_word(&mut ppu, 0x1000, 0x0080); // BG1 character pixel 0,0 = 1.
    vram_word(&mut ppu, 0x2000, 0x0080); // BG2 character pixel 0,0 = 1.
    ppu.write_register(0x2105, 0x01);
    ppu.write_register(0x2108, 0x08); // BG2 map starts at byte 0x0800.
    ppu.write_register(0x210B, 0x21); // BG1 chars 0x1000, BG2 chars 0x2000.
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
    palette(&mut ppu, 129, 0x001F); // Sprite palette 0, color 1 = blue.
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
    palette(&mut ppu, 1, 0x7C00); // BG1 red.
    palette(&mut ppu, 5, 0x001F); // BG3 2bpp palette 1 blue.
    vram_word(&mut ppu, 0x0000, 0x0000);
    vram_word(&mut ppu, 0x1000, 0x2400); // BG3 high tile, palette 1.
    vram_word(&mut ppu, 0x2000, 0x0080); // BG1 4bpp character.
    vram_word(&mut ppu, 0x3000, 0x0080); // BG3 2bpp character.
    ppu.write_register(0x2109, 0x10);
    ppu.write_register(0x210B, 0x02);
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
    palette(&mut ppu, 1, 0x7C00); // red
    palette(&mut ppu, 2, 0x03E0); // green
    palette(&mut ppu, 3, 0x001F); // blue
    palette(&mut ppu, 4, 0x7FE0); // yellow
    vram_word(&mut ppu, 0x0000, 0x0000);
    vram_word(&mut ppu, 0x2000, 0x0080); // tile 0, color 1
    vram_word(&mut ppu, 0x2020, 0x8000); // tile 1, color 2
    vram_word(&mut ppu, 0x2200, 0x8080); // tile 16, color 3
    vram_word(&mut ppu, 0x2230, 0x0080); // tile 17, plane2, color 4
    ppu.write_register(0x2105, 0x11); // Mode 1 BG1 uses 16x16 characters.
    ppu.write_register(0x210B, 0x02);
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
