//! Bootstrap PPU register model and software frame generation.

use serde::{Deserialize, Serialize};

/// Native SNES framebuffer dimensions.
pub const SCREEN_WIDTH: usize = 256;
/// Native SNES framebuffer dimensions.
pub const SCREEN_HEIGHT: usize = 224;

const PPU_REGISTER_COUNT: usize = 0x40;
const CGRAM_BYTES: usize = 512;
const VRAM_BYTES: usize = 64 * 1024;
const BACKGROUND_COUNT: usize = 4;
const OAM_BYTES: usize = 544;
const TILE_BYTES_4BPP: usize = 32;
const TILE_BYTES_2BPP: usize = 16;

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
    oam: Vec<u8>,
    cgram_address: u16,
    vram_address: u16,
    oam_address: u16,
    #[serde(default)]
    oam_byte_address: u16,
    #[serde(default)]
    oam_write_latch: u8,
    vram_increment: u16,
    bg_scroll_x: [u16; BACKGROUND_COUNT],
    bg_scroll_y: [u16; BACKGROUND_COUNT],
    bg_hofs_latch: [Option<u8>; BACKGROUND_COUNT],
    bg_vofs_latch: [Option<u8>; BACKGROUND_COUNT],
}

impl Default for Ppu {
    fn default() -> Self {
        Self {
            registers: vec![0; PPU_REGISTER_COUNT],
            cgram: vec![0; CGRAM_BYTES],
            vram: vec![0; VRAM_BYTES],
            oam: vec![0; OAM_BYTES],
            cgram_address: 0,
            vram_address: 0,
            oam_address: 0,
            oam_byte_address: 0,
            oam_write_latch: 0,
            vram_increment: 1,
            bg_scroll_x: [0; BACKGROUND_COUNT],
            bg_scroll_y: [0; BACKGROUND_COUNT],
            bg_hofs_latch: [None; BACKGROUND_COUNT],
            bg_vofs_latch: [None; BACKGROUND_COUNT],
        }
    }
}

impl Ppu {
    /// Read one CPU-visible PPU register.
    #[must_use]
    pub fn read_register(&self, register: u16) -> u8 {
        match register {
            0x2138 => self.read_oam_data(),
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
            0x210D..=0x2114 => self.write_bg_scroll(register, value),
            0x2102 => {
                self.oam_address = (self.oam_address & 0x0100) | u16::from(value);
                self.reload_oam_byte_address();
            }
            0x2103 => {
                self.oam_address = (self.oam_address & 0x00FF) | (u16::from(value & 0x01) << 8);
                self.reload_oam_byte_address();
            }
            0x2104 => self.write_oam_data(value),
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
        let main_screen_enable = self.registers[0x2C];
        if main_screen_enable == 0 {
            fill_frame(framebuffer, backdrop);
            return;
        }

        fill_frame(framebuffer, backdrop);
        let mut depth = vec![0_u8; framebuffer.width * framebuffer.height];
        let bgmode = self.registers[0x05] & 0x07;
        match bgmode {
            0 => self.render_background_stack(
                framebuffer,
                &mut depth,
                bgmode,
                &[
                    BackgroundConfig::for_mode0(3),
                    BackgroundConfig::for_mode0(2),
                    BackgroundConfig::for_mode0(1),
                    BackgroundConfig::for_mode0(0),
                ],
            ),
            1 => self.render_background_stack(
                framebuffer,
                &mut depth,
                bgmode,
                &[
                    BackgroundConfig::for_mode1(2),
                    BackgroundConfig::for_mode1(1),
                    BackgroundConfig::for_mode1(0),
                ],
            ),
            _ => {}
        }

        if main_screen_enable & 0x10 != 0 {
            self.render_objects(framebuffer, &mut depth, bgmode);
        }
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

    /// Borrow raw OAM bytes for tests and regression harnesses.
    #[must_use]
    pub fn oam(&self) -> &[u8] {
        &self.oam
    }

    fn reload_oam_byte_address(&mut self) {
        let word_address = self.oam_address & 0x00FF;
        let table_select = (self.oam_address & 0x0100) != 0;
        self.oam_byte_address = if table_select {
            512 + ((word_address & 0x000F) << 1)
        } else {
            word_address << 1
        };
    }

    fn read_oam_data(&self) -> u8 {
        self.oam[usize::from(self.oam_byte_address % OAM_BYTES as u16)]
    }

    fn write_oam_data(&mut self, value: u8) {
        let index = usize::from(self.oam_byte_address % OAM_BYTES as u16);
        if index < 512 {
            if index & 1 == 0 {
                self.oam_write_latch = value;
            } else {
                self.oam[index - 1] = self.oam_write_latch;
                self.oam[index] = value;
            }
        } else {
            self.oam[index] = value;
        }

        self.oam_byte_address = (self.oam_byte_address + 1) % OAM_BYTES as u16;
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

    fn render_background_stack(
        &self,
        framebuffer: &mut FrameBuffer,
        depth: &mut [u8],
        mode: u8,
        backgrounds: &[BackgroundConfig],
    ) {
        let enabled = self.registers[0x2C] & 0x0F;
        for background in backgrounds {
            if enabled & (1 << background.index) != 0 {
                self.render_background(
                    framebuffer,
                    depth,
                    mode,
                    self.resolve_background_config(*background),
                );
            }
        }
    }

    fn render_background(
        &self,
        framebuffer: &mut FrameBuffer,
        depth: &mut [u8],
        mode: u8,
        background: BackgroundConfig,
    ) {
        let bg3_high = self.registers[0x05] & 0x08 != 0;
        for y in 0..framebuffer.height {
            for x in 0..framebuffer.width {
                if let Some((pixel, high)) = self.background_pixel(background, x as u16, y as u16) {
                    let index = y * framebuffer.width + x;
                    let rank = background_priority_rank(mode, background.index, high, bg3_high);
                    if rank >= depth[index] {
                        depth[index] = rank;
                        let offset = index * 4;
                        framebuffer.pixels[offset..offset + 4].copy_from_slice(&pixel);
                    }
                }
            }
        }
    }

    fn background_pixel(
        &self,
        background: BackgroundConfig,
        x: u16,
        y: u16,
    ) -> Option<([u8; 4], bool)> {
        let world_x = usize::from(x.wrapping_add(self.bg_scroll_x[background.index]));
        let world_y = usize::from(y.wrapping_add(self.bg_scroll_y[background.index]));
        let size = background.tile_size;
        let entry_index = self.tilemap_entry_index(background, world_x / size, world_y / size);
        let entry = u16::from_le_bytes([
            self.vram[entry_index],
            self.vram[(entry_index + 1) % VRAM_BYTES],
        ]);
        let tile_number = usize::from(entry & 0x03FF);
        let palette = usize::from((entry >> 10) & 0x07);
        let high = entry & 0x2000 != 0;
        let hflip = entry & 0x4000 != 0;
        let vflip = entry & 0x8000 != 0;

        // 16x16 BG tiles use four adjacent 8x8 characters in the tile set;
        // the lower row begins 16 characters after the upper row.
        let fine_x = world_x % size;
        let fine_y = world_y % size;
        let source_x = if hflip { size - 1 - fine_x } else { fine_x };
        let source_y = if vflip { size - 1 - fine_y } else { fine_y };
        let character = (tile_number + (source_y / 8) * 16 + source_x / 8) & 0x03FF;
        let color_index = match background.bits_per_pixel {
            BitsPerPixel::Two => self.tile_pixel_2bpp(
                background.tiledata_base,
                character,
                source_x % 8,
                source_y % 8,
            ),
            BitsPerPixel::Four => self.tile_pixel_4bpp(
                background.tiledata_base,
                character,
                source_x % 8,
                source_y % 8,
            ),
        };
        if color_index == 0 {
            return None;
        }

        let cgram_color = match background.bits_per_pixel {
            BitsPerPixel::Two => background.palette_base + palette * 4 + usize::from(color_index),
            BitsPerPixel::Four => palette * 16 + usize::from(color_index),
        };
        let cgram_index = cgram_color * 2;
        let color = u16::from_le_bytes([
            self.cgram[cgram_index % CGRAM_BYTES],
            self.cgram[(cgram_index + 1) % CGRAM_BYTES],
        ]);
        Some((bgr555_to_rgba(color), high))
    }

    fn tilemap_entry_index(
        &self,
        background: BackgroundConfig,
        tile_x: usize,
        tile_y: usize,
    ) -> usize {
        let screens_wide = if background.size_code & 0x01 != 0 {
            2
        } else {
            1
        };
        let screens_high = if background.size_code & 0x02 != 0 {
            2
        } else {
            1
        };
        let wrapped_tile_x = tile_x % (screens_wide * 32);
        let wrapped_tile_y = tile_y % (screens_high * 32);
        let screen_x = wrapped_tile_x / 32;
        let screen_y = wrapped_tile_y / 32;
        let local_x = wrapped_tile_x % 32;
        let local_y = wrapped_tile_y % 32;
        let screen_index = screen_y * screens_wide + screen_x;
        (background.tilemap_base + screen_index * 0x800 + (local_y * 32 + local_x) * 2) % VRAM_BYTES
    }

    fn tile_pixel_2bpp(&self, tiledata_base: usize, tile_number: usize, x: usize, y: usize) -> u8 {
        let tile_base = (tiledata_base + tile_number * TILE_BYTES_2BPP) % VRAM_BYTES;
        let plane0 = self.vram[(tile_base + y * 2) % VRAM_BYTES];
        let plane1 = self.vram[(tile_base + y * 2 + 1) % VRAM_BYTES];
        let shift = 7 - x;

        ((plane0 >> shift) & 0x01) | (((plane1 >> shift) & 0x01) << 1)
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

    fn render_objects(&self, framebuffer: &mut FrameBuffer, depth: &mut [u8], mode: u8) {
        let objsel = self.registers[0x01];
        let (small_size, large_size) = object_size_pair(objsel >> 5);

        for sprite_index in (0..128).rev() {
            let base = sprite_index * 4;
            let x_low = self.oam[base];
            let y = self.oam[base + 1];
            let tile_number = usize::from(self.oam[base + 2]);
            let attributes = self.oam[base + 3];
            let high_entry = self.oam[512 + (sprite_index / 4)];
            let bit_shift = (sprite_index % 4) * 2;
            let x_high = (high_entry >> bit_shift) & 0x01 != 0;
            let large = (high_entry >> (bit_shift + 1)) & 0x01 != 0;
            let size = if large { large_size } else { small_size };

            let sprite_x = if x_high {
                i16::from(x_low) - 512
            } else {
                i16::from(x_low)
            };
            let sprite_y = i16::from(y);
            let palette = usize::from((attributes >> 1) & 0x07);
            let rank = sprite_priority_rank(mode, (attributes >> 4) & 0x03);
            let name_select = attributes & 0x01 != 0;
            let hflip = attributes & 0x40 != 0;
            let vflip = attributes & 0x80 != 0;
            let tile_base = self.object_tile_base(objsel, name_select);
            let tiles_per_side = usize::from(size / 8);

            for local_y in 0..usize::from(size) {
                let screen_y = sprite_y + local_y as i16;
                if !(0..framebuffer.height as i16).contains(&screen_y) {
                    continue;
                }

                for local_x in 0..usize::from(size) {
                    let screen_x = sprite_x + local_x as i16;
                    if !(0..framebuffer.width as i16).contains(&screen_x) {
                        continue;
                    }

                    let source_x = if hflip {
                        usize::from(size) - 1 - local_x
                    } else {
                        local_x
                    };
                    let source_y = if vflip {
                        usize::from(size) - 1 - local_y
                    } else {
                        local_y
                    };
                    let tile_x = source_x / 8;
                    let tile_y = source_y / 8;
                    let fine_x = source_x % 8;
                    let fine_y = source_y % 8;
                    let tile_offset = tile_y * 16 + tile_x;
                    if tile_x >= tiles_per_side || tile_y >= tiles_per_side {
                        continue;
                    }

                    let color_index =
                        self.tile_pixel_4bpp(tile_base, tile_number + tile_offset, fine_x, fine_y);
                    if color_index == 0 {
                        continue;
                    }

                    let cgram_color = 128 + palette * 16 + usize::from(color_index);
                    let cgram_index = cgram_color * 2;
                    let color = u16::from_le_bytes([
                        self.cgram[cgram_index % CGRAM_BYTES],
                        self.cgram[(cgram_index + 1) % CGRAM_BYTES],
                    ]);
                    let pixel_index = screen_y as usize * framebuffer.width + screen_x as usize;
                    if rank >= depth[pixel_index] {
                        depth[pixel_index] = rank;
                        let offset = pixel_index * 4;
                        framebuffer.pixels[offset..offset + 4]
                            .copy_from_slice(&bgr555_to_rgba(color));
                    }
                }
            }
        }
    }

    fn object_tile_base(&self, objsel: u8, name_select: bool) -> usize {
        let base_words = usize::from(objsel & 0x07) << 13;
        let select_offset_words = (usize::from((objsel >> 3) & 0x03) + 1) << 12;
        let word_address =
            (base_words + if name_select { select_offset_words } else { 0 }) & 0x7FFF;
        (word_address << 1) % VRAM_BYTES
    }

    fn resolve_background_config(&self, background: BackgroundConfig) -> BackgroundConfig {
        let tilemap_register = self.registers[0x07 + background.index];
        let tiledata_nibbles = if background.index < 2 {
            self.registers[0x0B]
        } else {
            self.registers[0x0C]
        };
        let tiledata_base = if background.index % 2 == 0 {
            usize::from(tiledata_nibbles & 0x0F) << 12
        } else {
            usize::from((tiledata_nibbles >> 4) & 0x0F) << 12
        };

        BackgroundConfig {
            tilemap_base: usize::from(tilemap_register & 0xFC) << 8,
            size_code: tilemap_register & 0x03,
            tiledata_base,
            tile_size: if self.registers[0x05] & (0x10 << background.index) != 0 {
                16
            } else {
                8
            },
            ..background
        }
    }

    fn write_bg_scroll(&mut self, register: u16, value: u8) {
        let Some((background, axis_is_vertical)) = (match register {
            0x210D => Some((0, false)),
            0x210E => Some((0, true)),
            0x210F => Some((1, false)),
            0x2110 => Some((1, true)),
            0x2111 => Some((2, false)),
            0x2112 => Some((2, true)),
            0x2113 => Some((3, false)),
            0x2114 => Some((3, true)),
            _ => None,
        }) else {
            return;
        };

        let latches = if axis_is_vertical {
            &mut self.bg_vofs_latch
        } else {
            &mut self.bg_hofs_latch
        };
        let scroll_values = if axis_is_vertical {
            &mut self.bg_scroll_y
        } else {
            &mut self.bg_scroll_x
        };

        if let Some(low) = latches[background].take() {
            scroll_values[background] = u16::from(low) | (u16::from(value) << 8);
        } else {
            latches[background] = Some(value);
        }
    }
}

#[derive(Clone, Copy)]
enum BitsPerPixel {
    Two,
    Four,
}

#[derive(Clone, Copy)]
struct BackgroundConfig {
    index: usize,
    bits_per_pixel: BitsPerPixel,
    tilemap_base: usize,
    tiledata_base: usize,
    size_code: u8,
    tile_size: usize,
    palette_base: usize,
}

fn object_size_pair(select: u8) -> (u8, u8) {
    match select & 0x07 {
        0 => (8, 16),
        1 => (8, 32),
        2 => (8, 64),
        3 => (16, 32),
        4 => (16, 64),
        5 => (32, 64),
        6 => (16, 32),
        7 => (16, 32),
        _ => (8, 8),
    }
}

impl BackgroundConfig {
    fn for_mode0(index: usize) -> Self {
        Self {
            index,
            bits_per_pixel: BitsPerPixel::Two,
            tilemap_base: 0,
            tiledata_base: 0,
            size_code: 0,
            tile_size: 8,
            palette_base: index * 32,
        }
    }

    fn for_mode1(index: usize) -> Self {
        Self {
            index,
            bits_per_pixel: if index == 2 {
                BitsPerPixel::Two
            } else {
                BitsPerPixel::Four
            },
            tilemap_base: 0,
            tiledata_base: 0,
            size_code: 0,
            tile_size: 8,
            palette_base: 0, // BG3 shares the first eight 2bpp palettes in Mode 1.
        }
    }
}

/// Rank the BG tile against other layers according to the Mode 0/1 priority table.
fn background_priority_rank(mode: u8, index: usize, high: bool, bg3_high: bool) -> u8 {
    match (mode, index, high) {
        (0, 0, true) => 11,
        (0, 1, true) => 10,
        (0, 0, false) => 8,
        (0, 1, false) => 7,
        (0, 2, true) => 5,
        (0, 3, true) => 4,
        (0, 2, false) => 2,
        (0, 3, false) => 1,
        (1, 0, true) => 9,
        (1, 1, true) => 8,
        (1, 0, false) => 6,
        (1, 1, false) => 5,
        (1, 2, true) => {
            if bg3_high {
                11
            } else {
                3
            }
        }
        (1, 2, false) => 1,
        _ => 0,
    }
}

fn sprite_priority_rank(mode: u8, priority: u8) -> u8 {
    match mode {
        0 => [3, 6, 9, 12][usize::from(priority & 3)],
        _ => [2, 4, 7, 10][usize::from(priority & 3)],
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

    #[test]
    fn oam_data_port_writes_store_and_advance_address() {
        let mut ppu = Ppu::default();
        ppu.write_register(0x2102, 0x00);
        ppu.write_register(0x2103, 0x00);
        ppu.write_register(0x2104, 0x12);
        ppu.write_register(0x2104, 0x34);

        assert_eq!(&ppu.oam()[..2], &[0x12, 0x34]);
    }

    #[test]
    fn oam_word_address_targets_low_table_pairs() {
        let mut ppu = Ppu::default();
        ppu.write_register(0x2102, 0x01);
        ppu.write_register(0x2103, 0x00);
        ppu.write_register(0x2104, 0x12);
        ppu.write_register(0x2104, 0x34);

        assert_eq!(&ppu.oam()[2..4], &[0x12, 0x34]);
    }

    #[test]
    fn oam_high_table_writes_are_immediate() {
        let mut ppu = Ppu::default();
        ppu.write_register(0x2102, 0x00);
        ppu.write_register(0x2103, 0x01);
        ppu.write_register(0x2104, 0xAA);
        ppu.write_register(0x2104, 0x55);

        assert_eq!(&ppu.oam()[512..514], &[0xAA, 0x55]);
    }

    #[test]
    fn obj_render_draws_sprite_pixels_when_enabled() {
        let mut ppu = Ppu::default();
        let mut frame = FrameBuffer::default();

        write_color(&mut ppu, 0x00, 0x0000);
        write_color(&mut ppu, 0x81, 0x7C00);

        ppu.write_register(0x2116, 0x00);
        ppu.write_register(0x2117, 0x00);
        ppu.write_register(0x2118, 0x80);
        ppu.write_register(0x2119, 0x00);
        for _ in 0..7 {
            ppu.write_register(0x2118, 0x00);
            ppu.write_register(0x2119, 0x00);
        }
        for _ in 0..8 {
            ppu.write_register(0x2118, 0x00);
            ppu.write_register(0x2119, 0x00);
        }

        ppu.write_register(0x2102, 0x00);
        ppu.write_register(0x2103, 0x00);
        ppu.write_register(0x2104, 0x00);
        ppu.write_register(0x2104, 0x00);
        ppu.write_register(0x2104, 0x00);
        ppu.write_register(0x2104, 0x00);
        ppu.write_register(0x212C, 0x10);
        ppu.render_frame(&mut frame);

        assert_eq!(&frame.pixels()[..4], &[248, 0, 0, 0xFF]);
    }
}
