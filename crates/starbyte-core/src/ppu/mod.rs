//! Bootstrap PPU register model and software frame generation.

use serde::{Deserialize, Serialize};

/// Native SNES framebuffer dimensions.
pub const SCREEN_WIDTH: usize = 256;
/// Native SNES framebuffer dimensions.
pub const SCREEN_HEIGHT: usize = 224;

const PPU_REGISTER_COUNT: usize = 0x40;
const CGRAM_BYTES: usize = 512;

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
    cgram_address: u16,
    cgram_high_byte: bool,
}

impl Default for Ppu {
    fn default() -> Self {
        Self {
            registers: vec![0; PPU_REGISTER_COUNT],
            cgram: vec![0; CGRAM_BYTES],
            cgram_address: 0,
            cgram_high_byte: false,
        }
    }
}

impl Ppu {
    /// Read one CPU-visible PPU register.
    #[must_use]
    pub fn read_register(&self, register: u16) -> u8 {
        match register {
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
            0x2121 => {
                self.cgram_address = u16::from(value) * 2;
                self.cgram_high_byte = false;
            }
            0x2122 => {
                let index = self.cgram_address as usize % CGRAM_BYTES;
                self.cgram[index] = value;
                self.cgram_address = (self.cgram_address + 1) & 0x01FF;
                self.cgram_high_byte = !self.cgram_high_byte;
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
        let bgmode = self.registers[0x05] & 0x07;
        let main_screen_enable = self.registers[0x2C];

        for y in 0..framebuffer.height {
            for x in 0..framebuffer.width {
                let mut color = backdrop;

                if main_screen_enable != 0 {
                    let stripe = (((x / 8) ^ (y / 8)) & 1) as u8;
                    let accent = bgmode.saturating_mul(20).saturating_add(20);
                    color[0] = color[0].saturating_add(accent.saturating_mul(stripe));
                    color[1] = color[1].saturating_add((accent / 2).saturating_mul(stripe));
                    color[2] = color[2].saturating_sub((accent / 3).saturating_mul(stripe));
                }

                let pixel = (y * framebuffer.width + x) * 4;
                framebuffer.pixels[pixel..pixel + 4].copy_from_slice(&color);
            }
        }
    }

    /// Borrow raw CGRAM bytes for tests and regression harnesses.
    #[must_use]
    pub fn cgram(&self) -> &[u8] {
        &self.cgram
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
    fn register_state_changes_frame_output() {
        let mut ppu = Ppu::default();
        let mut frame = FrameBuffer::default();

        ppu.write_register(0x2121, 0x00);
        ppu.write_register(0x2122, 0x00);
        ppu.write_register(0x2122, 0x7C);
        ppu.write_register(0x2105, 0x01);
        ppu.write_register(0x212C, 0x01);
        ppu.render_frame(&mut frame);

        assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 0xFF]);
        assert_ne!(&frame.pixels()[..4], &frame.pixels()[4 * 8..4 * 9]);
    }
}
