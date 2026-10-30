//! Screenshot compression ladder (browser-control.md §5.2): PNG if small enough, else JPEG
//! q80/60/40/20 at 100/75/50/25 % until the base64 payload fits 200 KiB.

use image::ImageEncoder as _;

/// Base64 budget of an image block.
pub(crate) const MAX_B64: usize = 200 * 1024;

/// A compressed image.
#[derive(Debug)]
pub(crate) struct Compressed {
    pub(crate) mime: &'static str,
    pub(crate) data: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

fn b64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

/// Compresses `png` to fit [`MAX_B64`].
pub(crate) fn compress(png: &[u8]) -> Option<Compressed> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let (w, h) = (img.width(), img.height());
    if b64_len(png.len()) <= MAX_B64 {
        return Some(Compressed {
            mime: "image/png",
            data: png.to_vec(),
            width: w,
            height: h,
        });
    }
    let rgb = img.to_rgb8();
    let mut last = None;
    for pct in [100u32, 75, 50, 25] {
        let (sw, sh) = ((w * pct / 100).max(1), (h * pct / 100).max(1));
        let scaled = if pct == 100 {
            rgb.clone()
        } else {
            image::imageops::resize(&rgb, sw, sh, image::imageops::FilterType::Triangle)
        };
        for q in [80u8, 60, 40, 20] {
            let mut out = Vec::new();
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q);
            if enc
                .write_image(scaled.as_raw(), sw, sh, image::ExtendedColorType::Rgb8)
                .is_err()
            {
                continue;
            }
            let fits = b64_len(out.len()) <= MAX_B64;
            last = Some(Compressed {
                mime: "image/jpeg",
                data: out,
                width: sw,
                height: sh,
            });
            if fits {
                return last;
            }
        }
    }
    last
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, noisy: bool) -> Vec<u8> {
        let mut img = image::RgbImage::new(w, h);
        let mut seed = 7u32;
        for p in img.pixels_mut() {
            if noisy {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let b = seed.to_le_bytes();
                *p = image::Rgb([b[1], b[2], b[3]]);
            }
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn ladder() {
        let small = compress(&png(64, 64, false)).unwrap();
        assert_eq!(small.mime, "image/png");
        let big = compress(&png(1280, 800, true)).unwrap();
        assert_eq!(big.mime, "image/jpeg");
        assert!(b64_len(big.data.len()) <= MAX_B64);
        assert!(big.width < 1280);
    }
}
