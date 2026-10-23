//! Shared zero-intermediate-copy upload boundary for both the established
//! Starbyte player and the experimental independently scheduled game worker.
//! Rendering can pause without ever controlling the emulator's frame clock.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

/// Upload valid RGBA8 pixels into the same nearest-filtered egui texture.
///
/// Pixel data is borrowed from whichever backend currently owns the frame.
/// The only image allocation is egui's required ColorImage; no cloned
/// framebuffer is introduced into the synchronous legacy player.
pub fn upload_rgba(
    ctx: &egui::Context,
    texture: &mut Option<TextureHandle>,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> bool {
    let Ok(w) = usize::try_from(width) else {
        return false;
    };
    let Ok(h) = usize::try_from(height) else {
        return false;
    };
    if w == 0
        || h == 0
        || w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4)) != Some(rgba.len())
    {
        return false;
    }

    let image = ColorImage::from_rgba_unmultiplied([w, h], rgba);
    if let Some(texture) = texture {
        texture.set(image, TextureOptions::NEAREST);
    } else {
        *texture = Some(ctx.load_texture(name, image, TextureOptions::NEAREST));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_invalid_rgba_without_replacing_existing_texture() {
        let ctx = egui::Context::default();
        let mut texture = None;
        assert!(!upload_rgba(&ctx, &mut texture, "bad", 256, 224, &[]));
        assert!(texture.is_none());
        assert!(!upload_rgba(&ctx, &mut texture, "bad", 1, 1, &[1, 2, 3]));
        assert!(texture.is_none());
        assert!(!upload_rgba(
            &ctx,
            &mut texture,
            "bad",
            u32::MAX,
            u32::MAX,
            &[]
        ));
        assert!(texture.is_none());
    }
}
