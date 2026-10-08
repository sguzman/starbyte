//! End-to-end synthetic ROM boot: 65816 instructions -> bus -> PPU -> RGBA.
 //! No commercial assets, downloaded firmware, or external ROM required.

use starbyte_core::{Emulator, cartridge::Cartridge};

fn write_register(program: &mut Vec<u8>, register: u16, value: u8) {
    let [lo, hi] = register.to_le_bytes();
    program.extend_from_slice(&[0xA9, value, 0x8D, lo, hi]); // LDA #imm; STA abs.
}

fn picture_rom(fixed_math: bool) -> Cartridge {
    let mut rom = vec![0_u8; 0x10000];
    let header = 0x7FC0;
    rom[header..header + 21].copy_from_slice(b"STARBYTE CPU PPU TEST");
    rom[header + 0x15] = 0x20; // LoROM, no enhancement chip.
    rom[header + 0x17] = 0x09;
    rom[header + 0x18] = 0x00;
    rom[header + 0x19] = 0x01;
    rom[header + 0x1C] = 0xFF;
    rom[header + 0x1D] = 0xFF;
    rom[0x7FFC] = 0x00; // Reset vector $8000.
    rom[0x7FFD] = 0x80;

    let mut program = Vec::new();
    // Upload palette color 1 = red.
    write_register(&mut program, 0x2121, 0x01);
    write_register(&mut program, 0x2122, 0x1F);
    write_register(&mut program, 0x2122, 0x00);

    // Upload one visible BG1 4bpp tile pixel to VRAM byte $2000.
    write_register(&mut program, 0x2115, 0x80);
    write_register(&mut program, 0x2116, 0x00);
    write_register(&mut program, 0x2117, 0x10);
    write_register(&mut program, 0x2118, 0x80);
    write_register(&mut program, 0x2119, 0x00);

    // Tilemap at VRAM $0000 defaults to character zero.
    write_register(&mut program, 0x2105, 0x01); // Mode 1.
    write_register(&mut program, 0x210B, 0x01); // BG1 character data $2000.
    write_register(&mut program, 0x212C, 0x01); // BG1 on main screen.
    if fixed_math {
        write_register(&mut program, 0x2132, 0x50); // Fixed green 16/31.
        write_register(&mut program, 0x2131, 0x01); // Add fixed color to BG1.
    }
    write_register(&mut program, 0x2100, 0x0F); // Unblank at full brightness.
    program.extend_from_slice(&[0x80, 0xFE]); // BRA forever (don't BRK).
    rom[..program.len()].copy_from_slice(&program);
    Cartridge::from_bytes(rom, None).unwrap()
}

fn boot_frame(fixed_math: bool) -> Emulator {
    let mut emulator = Emulator::default();
    emulator.load_rom(picture_rom(fixed_math));
    emulator.run_until_frame().unwrap();
    emulator
}

#[test]
fn real_cpu_program_uploads_a_visible_background_pixel() {
    let emulator = boot_frame(false);
    let frame = emulator.framebuffer();
    assert_eq!(emulator.timing().frame, 1);
    assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 255]);
    assert_eq!(&frame.pixels()[4..8], &[0, 0, 0, 255]);
    assert_eq!(emulator.peek_ppu_register(0x2100), Some(0x0F));
    assert!(emulator.system_observability().ppu_write_counts()[0x18] > 0);
}

#[test]
fn real_cpu_program_can_enable_fixed_color_math() {
    let emulator = boot_frame(true);
    assert_eq!(&emulator.framebuffer().pixels()[..4], &[248, 128, 0, 255]);
    assert_eq!(&emulator.framebuffer().pixels()[4..8], &[0, 0, 0, 255]);
    assert_eq!(emulator.peek_ppu_register(0x2131), Some(0x01));
}
