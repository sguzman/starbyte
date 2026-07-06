//! Bootstrap PPU register model and software frame generation.

use serde::{Deserialize, Serialize};

/// Native SNES framebuffer dimensions.
pub const SCREEN_WIDTH: usize = 256;
/// Native SNES framebuffer dimensions.
pub const SCREEN_HEIGHT: usize = 224;

const PPU_REGISTER_COUNT: usize = 0x40;
const CGRAM_BYTES: usize = 512;
const VRAM_BYTES: usize = 64 * 1024;
const TILEMAP_TILE_COUNT: usize = 32 * 32;
const TILE_BYTES_4BPP: usize = 32;

/// Software framebuffer in RGBA8 format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameBuffer {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self {
            width: SCREEN_WIDTH,
            height: SCREEN_HEIGHT,
            pixels: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4],
        }
    }
}

impl FrameBuffer {
    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Backing RGBA pixels.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Mutable access for future renderers.
    #[must_use]
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }
}

/// Minimal PPU state needed for bootstrap register correctness and frame output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ppu {
    registers: Vec<u8>,
    cgram: Vec<u8>,
    vram: Vec<u8>,
    cgram_address: u16,
    vram_address: u16,
    vram_increment: u16,
    bg1_scroll_x: u16,
    bg1_scroll_y: u16,
    bg1_hofs_latch: Option<u8>,
    bg1_vofs_latch: Option<u8>,
}

impl Default for Ppu {
    fn default() -> Self {
        Self {
            registers: vec![0; PPU_REGISTER_COUNT],
            cgram: vec![0; CGRAM_BYTES],
            vram: vec![0; VRAM_BYTES],
            cgram_address: 0,
            vram_address: 0,
            vram_increment: 1,
            bg1_scroll_x: 0,
            bg1_scroll_y: 0,
            bg1_hofs_latch: None,
            bg1_vofs_latch: None,
        }
    }
}

impl Ppu {
    /// Read one CPU-visible PPU register.
    #[must_use]
    pub fn read_register(&self, register: u16) -> u8 {
        match register {
            0x2139 => self.vram_word_byte(self.vram_address, false),
            0x213A => self.vram_word_byte(self.vram_address, true),
            0x213B => self.cgram[self.cgram_address as usize % CGRAM_BYTES],
            0x2100..=0x213F => self.registers[usize::from(register - 0x2100)],
            _ => 0,
        }
    }

    /// Write one CPU-visible PPU register.
    pub fn write_register(&mut self, register: u16, value: u8) {
        if !(0x2100..=0x213F).contains(&register) {
            return;
        }

        self.registers[usize::from(register - 0x2100)] = value;
        match register {
            0x210D => {
                if let Some(low) = self.bg1_hofs_latch.take() {
                    self.bg1_scroll_x = u16::from(low) | (u16::from(value) << 8);
                } else {
                    self.bg1_hofs_latch = Some(value);
                }
            }
            0x210E => {
                if let Some(low) = self.bg1_vofs_latch.take() {
                    self.bg1_scroll_y = u16::from(low) | (u16::from(value) << 8);
                } else {
                    self.bg1_vofs_latch = Some(value);
                }
            }
            0x2115 => {
                self.vram_increment = match value & 0x03 {
                    0 => 1,
                    1 => 32,
                    _ => 128,
                };
            }
            0x2116 => {
                self.vram_address = (self.vram_address & 0xFF00) | u16::from(value);
            }
            0x2117 => {
                self.vram_address = (self.vram_address & 0x00FF) | (u16::from(value) << 8);
            }
            0x2118 => self.write_vram_data(value, false),
            0x2119 => self.write_vram_data(value, true),
            0x2121 => {
                self.cgram_address = u16::from(value) * 2;
            }
            0x2122 => {
                let index = self.cgram_address as usize % CGRAM_BYTES;
                self.cgram[index] = value;
                self.cgram_address = (self.cgram_address + 1) & 0x01FF;
            }
            _ => {}
        }
    }

    /// Render one deterministic bootstrap frame.
    pub fn render_frame(&self, framebuffer: &mut FrameBuffer) {
        let forced_blank = self.registers[0x00] & 0x80 != 0;
        if forced_blank {
            fill_frame(framebuffer, [0, 0, 0, 0xFF]);
            return;
        }

        let backdrop = bgr555_to_rgba(self.backdrop_color());
        if self.registers[0x2C] & 0x01 == 0 {
            fill_frame(framebuffer, backdrop);
            return;
        }

        self.render_bg1(framebuffer, backdrop);
    }

    /// Borrow raw CGRAM bytes for tests and regression harnesses.
    #[must_use]
    pub fn cgram(&self) -> &[u8] {
        &self.cgram
    }

    /// Borrow raw VRAM bytes for tests and regression harnesses.
    #[must_use]
    pub fn vram(&self) -> &[u8] {
        &self.vram
    }

    fn write_vram_data(&mut self, value: u8, high_byte: bool) {
        let index = self.vram_byte_index(self.vram_address, high_byte);
        self.vram[index] = value;
        if high_byte {
            self.vram_address = self.vram_address.wrapping_add(self.vram_increment);
        }
    }

    fn vram_word_byte(&self, address: u16, high_byte: bool) -> u8 {
        self.vram[self.vram_byte_index(address, high_byte)]
    }

    fn vram_byte_index(&self, address: u16, high_byte: bool) -> usize {
        let word_index = usize::from(address) * 2;
        (word_index + usize::from(high_byte)) % VRAM_BYTES
    }

    fn render_bg1(&self, framebuffer: &mut FrameBuffer, backdrop: [u8; 4]) {
        let bgmode = self.registers[0x05] & 0x07;
        if bgmode > 1 {
            fill_frame(framebuffer, backdrop);
            return;
        }

        let tilemap_base = usize::from(self.registers[0x07] & 0xFC) << 8;
        let tiledata_base = usize::from(self.registers[0x0B] & 0x0F) << 12;

        for y in 0..framebuffer.height {
            for x in 0..framebuffer.width {
                let pixel =
                    self.bg1_pixel(x as u16, y as u16, tilemap_base, tiledata_base, backdrop);
                let offset = (y * framebuffer.width + x) * 4;
                framebuffer.pixels[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
    }

    fn bg1_pixel(
        &self,
        x: u16,
        y: u16,
        tilemap_base: usize,
        tiledata_base: usize,
        backdrop: [u8; 4],
    ) -> [u8; 4] {
        let world_x = x.wrapping_add(self.bg1_scroll_x) & 0x00FF;
        let world_y = y.wrapping_add(self.bg1_scroll_y) & 0x00FF;
        let tile_x = usize::from(world_x / 8);
        let tile_y = usize::from(world_y / 8);
        let tile_index = tile_y * 32 + tile_x;
        if tile_index >= TILEMAP_TILE_COUNT {
            return backdrop;
        }

        let entry_index = (tilemap_base + tile_index * 2) % VRAM_BYTES;
        let entry = u16::from_le_bytes([
            self.vram[entry_index],
            self.vram[(entry_index + 1) % VRAM_BYTES],
        ]);
        let tile_number = usize::from(entry & 0x03FF);
        let palette = usize::from((entry >> 10) & 0x07);
        let hflip = entry & 0x4000 != 0;
        let vflip = entry & 0x8000 != 0;

        let fine_x = usize::from(world_x % 8);
        let fine_y = usize::from(world_y % 8);
        let tile_x = if hflip { 7 - fine_x } else { fine_x };
        let tile_y = if vflip { 7 - fine_y } else { fine_y };
        let color_index = self.tile_pixel_4bpp(tiledata_base, tile_number, tile_x, tile_y);
        if color_index == 0 {
            return backdrop;
        }

        let cgram_index = (palette * 16 + usize::from(color_index)) * 2;
        let color = u16::from_le_bytes([
            self.cgram[cgram_index % CGRAM_BYTES],
            self.cgram[(cgram_index + 1) % CGRAM_BYTES],
        ]);
        bgr555_to_rgba(color)
    }

    fn tile_pixel_4bpp(&self, tiledata_base: usize, tile_number: usize, x: usize, y: usize) -> u8 {
        let tile_base = (tiledata_base + tile_number * TILE_BYTES_4BPP) % VRAM_BYTES;
        let plane0 = self.vram[(tile_base + y * 2) % VRAM_BYTES];
        let plane1 = self.vram[(tile_base + y * 2 + 1) % VRAM_BYTES];
        let plane2 = self.vram[(tile_base + 16 + y * 2) % VRAM_BYTES];
        let plane3 = self.vram[(tile_base + 16 + y * 2 + 1) % VRAM_BYTES];
        let shift = 7 - x;

        ((plane0 >> shift) & 0x01)
            | (((plane1 >> shift) & 0x01) << 1)
            | (((plane2 >> shift) & 0x01) << 2)
            | (((plane3 >> shift) & 0x01) << 3)
    }

    fn backdrop_color(&self) -> u16 {
        u16::from(self.cgram[0]) | (u16::from(self.cgram[1]) << 8)
    }
}

fn fill_frame(framebuffer: &mut FrameBuffer, rgba: [u8; 4]) {
    for pixel in framebuffer.pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&rgba);
    }
}

fn bgr555_to_rgba(color: u16) -> [u8; 4] {
    let blue = ((color & 0x1F) as u8) << 3;
    let green = (((color >> 5) & 0x1F) as u8) << 3;
    let red = (((color >> 10) & 0x1F) as u8) << 3;
    [red, green, blue, 0xFF]
}

#[cfg(test)]
mod tests {
    use super::{FrameBuffer, Ppu};

    fn write_color(ppu: &mut Ppu, slot: u8, color: u16) {
        let [low, high] = color.to_le_bytes();
        ppu.write_register(0x2121, slot);
        ppu.write_register(0x2122, low);
        ppu.write_register(0x2122, high);
    }

    #[test]
    fn cgram_stream_writes_update_backdrop_color() {
        let mut ppu = Ppu::default();
        ppu.write_register(0x2121, 0x00);
        ppu.write_register(0x2122, 0x1F);
        ppu.write_register(0x2122, 0x00);

        assert_eq!(ppu.cgram()[0], 0x1F);
        assert_eq!(ppu.cgram()[1], 0x00);
    }

    #[test]
    fn vram_data_ports_store_words_and_advance_address() {
        let mut ppu = Ppu::default();
        ppu.write_register(0x2116, 0x00);
        ppu.write_register(0x2117, 0x00);
        ppu.write_register(0x2118, 0x34);
        ppu.write_register(0x2119, 0x12);
        ppu.write_register(0x2118, 0x78);
        ppu.write_register(0x2119, 0x56);

        assert_eq!(&ppu.vram()[..4], &[0x34, 0x12, 0x78, 0x56]);
    }

    #[test]
    fn forced_blank_renders_black() {
        let mut ppu = Ppu::default();
        let mut frame = FrameBuffer::default();
        ppu.write_register(0x2100, 0x80);
        ppu.render_frame(&mut frame);

        assert!(
            frame
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [0, 0, 0, 0xFF])
        );
    }

    #[test]
    fn bg1_tilemap_render_uses_vram_tiles_and_cgram_palette() {
        let mut ppu = Ppu::default();
        let mut frame = FrameBuffer::default();

        write_color(&mut ppu, 0x00, 0x0000);
        write_color(&mut ppu, 0x01, 0x7C00);

        ppu.write_register(0x2116, 0x00);
        ppu.write_register(0x2117, 0x00);
        ppu.write_register(0x2118, 0x00);
        ppu.write_register(0x2119, 0x00);

        ppu.write_register(0x2116, 0x00);
        ppu.write_register(0x2117, 0x08);
        for row in 0..8 {
            let low = if row == 0 { 0x80 } else { 0x00 };
            ppu.write_register(0x2118, low);
            ppu.write_register(0x2119, 0x00);
        }
        for _ in 0..8 {
            ppu.write_register(0x2118, 0x00);
            ppu.write_register(0x2119, 0x00);
        }

        ppu.write_register(0x2105, 0x01);
        ppu.write_register(0x210B, 0x01);
        ppu.write_register(0x212C, 0x01);
        ppu.render_frame(&mut frame);

        assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 0xFF]);
        assert_eq!(&frame.pixels()[4..8], &[0, 0, 0, 0xFF]);
    }

    #[test]
    fn screen_disable_falls_back_to_backdrop() {
        let mut ppu = Ppu::default();
        let mut frame = FrameBuffer::default();
        write_color(&mut ppu, 0x00, 0x7C00);
        ppu.render_frame(&mut frame);

        assert!(
            frame
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [248, 0, 0, 0xFF])
        );
    }
}
