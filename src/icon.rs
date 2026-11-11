// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! The pictures the window shows.
//!
//! - The application's icon: "AP" over "DU" on a slate square. `assets/icon.svg` is the source,
//!   with `assets/icon-small.svg` for 16 to 32 px; `assets/icon-256.png` (window, About, help)
//!   and `assets/icon.ico` (the Windows executable, through build.rs) are rendered from them.
//! - Ialto's logo (About): `assets/ialto-logo.svg`, rendered 160 px high as
//!   `assets/ialto-logo.png`. Its white letters have a black outline, so it reads on both
//!   themes. The logo and the Ialto name aren't covered by the code's licence (see NOTICE).

/// The icon, 256 x 256, as PNG.
pub const PNG: &[u8] = include_bytes!("../assets/icon-256.png");
/// Ialto's logo, 160 px high, as PNG.
pub const IALTO_PNG: &[u8] = include_bytes!("../assets/ialto-logo.png");

/// A PNG's pixels (RGBA, not premultiplied) with its width and height; None if it can't be
/// read, in which case the picture is simply left out.
pub fn decode(png: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let img = img.into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

/// The icon's pixels, width and height.
pub fn rgba() -> Option<(Vec<u8>, u32, u32)> {
    decode(PNG)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icon_is_a_square_256_png_with_a_drawn_middle() {
        let Some((px, w, h)) = super::rgba() else {
            panic!("the icon doesn't decode");
        };
        assert_eq!((w, h), (256, 256));
        assert_eq!(px.len(), 256 * 256 * 4);
        let alpha = |x: usize, y: usize| px.get((y * 256 + x) * 4 + 3).copied();
        assert_eq!(alpha(128, 128), Some(255), "the middle is drawn");
        assert_eq!(
            alpha(0, 0),
            Some(0),
            "the corners are transparent (round icon)"
        );
    }

    #[test]
    fn the_ialto_logo_is_a_160_px_high_png() {
        let Some((px, w, h)) = super::decode(super::IALTO_PNG) else {
            panic!("the logo doesn't decode");
        };
        assert_eq!(h, 160);
        assert!(w > 2 * h, "a wide logo: {w} x {h}");
        assert!(
            px.chunks(4).any(|p| p.get(3) == Some(&255)),
            "something is drawn"
        );
    }
}
