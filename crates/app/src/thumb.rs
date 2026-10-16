//! Square thumbnails of attached images, cropped from their centre.
//!
//! gpui's `ObjectFit::Cover` does not crop: it paints the image larger than
//! its element and relies on an ancestor to clip, and it rounds the corners
//! of that larger image, not the tile's. A wide screenshot in a 56 pt tile
//! drew about 330 pt across the composer. A thumbnail that is already square
//! fits its tile exactly, so the tile's own corners round it.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Pixels on a side: the largest tile (140 pt) on a Retina screen.
pub const EDGE: u32 = 280;

fn dir() -> PathBuf {
    crate::control::socket_dir().join("thumbs")
}

/// Where the thumbnail of `src` is kept: one per file version, so an
/// image changed in place gets a new one.
fn cached(src: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let meta = std::fs::metadata(src)?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().hash(&mut h);
    EDGE.hash(&mut h);
    Ok(dir.join(format!("{:016x}.png", h.finish())))
}

/// The thumbnail of `src`, made if it is not kept yet. Decodes the whole
/// image, so run it off the main thread.
pub fn make(src: &Path) -> anyhow::Result<PathBuf> {
    make_in(src, &dir())
}

fn make_in(src: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let out = cached(src, dir)?;
    if out.exists() {
        return Ok(out);
    }
    let img = image::open(src)?;
    let (x, y, side) = square(img.width(), img.height());
    let mut thumb = img.crop_imm(x, y, side, side);
    if side > EDGE {
        thumb = thumb.resize_exact(EDGE, EDGE, image::imageops::FilterType::Triangle);
    }
    std::fs::create_dir_all(dir)?;
    // Written aside and moved into place, so a half-written file is never
    // the one shown.
    let tmp = out.with_extension(format!("{}.tmp", std::process::id()));
    thumb.save_with_format(&tmp, image::ImageFormat::Png)?;
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}

/// The largest centred square in a `w` × `h` image: its left, top and side.
fn square(w: u32, h: u32) -> (u32, u32, u32) {
    let side = w.min(h);
    ((w - side) / 2, (h - side) / 2, side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_the_centre_square() {
        // The screenshot that spilled across the composer.
        assert_eq!(square(2222, 380), (921, 0, 380));
        assert_eq!(square(300, 900), (0, 300, 300));
        assert_eq!(square(64, 64), (0, 0, 64));
    }

    #[test]
    fn a_wide_image_comes_out_square() {
        let dir = std::env::temp_dir().join(format!("deck-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("wide.png");
        image::RgbImage::from_pixel(2222, 380, image::Rgb([200, 30, 30]))
            .save(&src)
            .unwrap();
        let kept = dir.join("thumbs");
        let out = make_in(&src, &kept).unwrap();
        assert_eq!(image::image_dimensions(&out).unwrap(), (EDGE, EDGE));
        // Kept: asking again finds the same file.
        assert_eq!(make_in(&src, &kept).unwrap(), out);
        let _ = std::fs::remove_dir_all(dir);
    }
}
