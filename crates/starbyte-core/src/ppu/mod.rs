//! PPU scaffolding and framebuffer representation.

use serde::{Deserialize, Serialize};

/// Native SNES framebuffer dimensions.
pub const SCREEN_WIDTH: usize = 256;
/// Native SNES framebuffer dimensions.
pub const SCREEN_HEIGHT: usize = 224;

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
