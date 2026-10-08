//! A copyright-free, self-contained SNES graphics test cartridge.
//!
//! Generated every time from 65816 opcodes. It is deliberately tiny and
//! diagnostic, not a bundled commercial game or a playability benchmark.

/// Assemble a minimal LoROM that draws a red/cyan checkerboard in Mode 0.
///
/// Intended for checking native Wayland startup, display scaling, screenshots,
/// and basic CPU -> bus -> PPU rendering without third-party ROM files.
#[must_use]
pub fn demo_rom_bytes() -> Vec<u8> {
    let mut rom = vec![0_u8; 0x10000];
    let header = 0x7FC0;
    rom[header..header + 21].copy_from_slice(b"STARBYTE DISPLAY DEMO");
    rom[header + 0x15] = 0x20; // LoROM mapping.
    rom[header + 0x17] = 0x09;
    rom[header + 0x18] = 0x00; // No battery-backed SRAM.
    rom[header + 0x19] = 0x01;
    rom[header + 0x1C] = 0xFF;
    rom[header + 0x1D] = 0xFF;
    rom[0x7FFC] = 0x00; // Reset CPU to $00:8000.
    rom[0x7FFD] = 0x80;

    let mut code = Vec::new();
    // Each pair is LDA #8bit / STA absolute.
    fn write_register(code: &mut Vec<u8>, register: u16, value: u8) {
        let [low, high] = register.to_le_bytes();
        code.extend_from_slice(&[0xA9, value, 0x8D, low, high]);
    }

    // CGRAM color 0 = black, 1 = red, 2 = cyan.
    for (slot, low, high) in [(0, 0x00, 0x00), (1, 0x1F, 0x00), (2, 0xE0, 0x7F)] {
        write_register(&mut code, 0x2121, slot);
        write_register(&mut code, 0x2122, low);
        write_register(&mut code, 0x2122, high);
    }

    // One 2bpp tile at VRAM byte $2000, repeated by BG1's zero tilemap.
    // 4x4 alternating red/cyan squares remain obvious at integer scales.
    write_register(&mut code, 0x2115, 0x80); // Increment after high write.
    write_register(&mut code, 0x2116, 0x00);
    write_register(&mut code, 0x2117, 0x10);
    for row in 0..8 {
        let plane0 = if row < 4 { 0xF0 } else { 0x0F };
        write_register(&mut code, 0x2118, plane0);
        write_register(&mut code, 0x2119, !plane0);
    }

    write_register(&mut code, 0x2105, 0x00); // Mode 0, 2bpp.
    write_register(&mut code, 0x210B, 0x01); // BG1 tile data at byte $2000.
    write_register(&mut code, 0x212C, 0x01); // BG1 visible on main screen.
    write_register(&mut code, 0x2100, 0x0F); // Unblank, full brightness.
    code.extend_from_slice(&[0x80, 0xFE]); // BRA -2 forever.
    rom[..code.len()].copy_from_slice(&code);
    rom
}

#[cfg(test)]
mod tests {
    use crate::{Emulator, cartridge::Cartridge};

    use super::demo_rom_bytes;

    fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        let start = (y * 256 + x) * 4;
        pixels[start..start + 4].try_into().unwrap()
    }

    #[test]
    fn generated_cartridge_draws_deterministic_checkerboard_from_cpu_instructions() {
        let bytes = demo_rom_bytes();
        assert_eq!(bytes.len(), 0x10000);
        let cart = Cartridge::from_bytes(bytes, None).unwrap();
        assert_eq!(cart.header().title, "STARBYTE DISPLAY DEMO");
        assert_eq!(cart.header().ram_size_bytes(), 0);

        let mut emu = Emulator::default();
        emu.load_rom(cart);
        emu.run_until_frame().unwrap();
        let pixels = emu.framebuffer().pixels();
        let red = [248, 0, 0, 255];
        let cyan = [0, 248, 248, 255];
        assert_eq!(pixel(pixels, 0, 0), red);
        assert_eq!(pixel(pixels, 4, 0), cyan);
        assert_eq!(pixel(pixels, 0, 4), cyan);
        assert_eq!(pixel(pixels, 4, 4), red);
        assert_eq!(pixel(pixels, 8, 0), red);
    }
}
