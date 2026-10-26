//! Image handling for `read` (pi `utils/image-process.ts`, `image-resize-core.ts`,
//! `image-convert.ts`, `mime.ts`, `exif-orientation.ts`).
//!
//! pi decodes and re-encodes with Photon (the Rust `image` crate compiled to WASM); Oino uses
//! the `image` crate directly with the same steps: EXIF orientation, Lanczos3 resize, PNG and
//! JPEG candidates at decreasing quality, then shrinking by 0.75 until the encoded size fits.
//! Re-encoded bytes may differ from Photon's; dimensions, MIME types and notes match.

mod exif;
mod mime;

use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use oino_ai::ImageResize;

pub use mime::{IMAGE_TYPE_SNIFF_BYTES, detect_supported_image_mime_type};

/// Default base64 payload limit: 4.5MB (below Anthropic's 5MB).
const DEFAULT_MAX_BYTES: u64 = 4_718_592;
const DEFAULT_MAX_DIMENSION: u32 = 2000;
const DEFAULT_JPEG_QUALITY: u8 = 80;

/// pi `processImage` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessedImage {
    /// An inline image.
    Ok {
        /// Base64 data.
        data: String,
        /// MIME type of `data`.
        mime_type: String,
        /// Notes for the model (conversion, resize scale).
        hints: Vec<String>,
    },
    /// The image could not be inlined; `message` explains why.
    Omitted {
        /// pi's `[Image omitted: …]` text.
        message: String,
    },
}

/// pi `ResizedImage`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ResizedImage {
    data: String,
    mime_type: String,
    original_width: u32,
    original_height: u32,
    width: u32,
    height: u32,
    was_resized: bool,
}

/// Resize limits with pi's defaults filled in.
#[derive(Clone, Copy, Debug)]
struct ResizeLimits {
    max_width: u32,
    max_height: u32,
    max_bytes: u64,
    jpeg_quality: u8,
}

impl ResizeLimits {
    fn from_options(options: Option<&ImageResize>) -> Self {
        Self {
            max_width: options
                .and_then(|o| o.max_width)
                .unwrap_or(DEFAULT_MAX_DIMENSION),
            max_height: options
                .and_then(|o| o.max_height)
                .unwrap_or(DEFAULT_MAX_DIMENSION),
            max_bytes: options
                .and_then(|o| o.max_bytes)
                .unwrap_or(DEFAULT_MAX_BYTES),
            jpeg_quality: options
                .and_then(|o| o.jpeg_quality)
                .unwrap_or(DEFAULT_JPEG_QUALITY),
        }
    }
}

fn base_mime_type(mime: &str) -> String {
    mime.split(';').next().unwrap_or(mime).trim().to_lowercase()
}

fn supported_inline_mime(mime: &str) -> Option<&'static str> {
    match base_mime_type(mime).as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn decode_oriented(bytes: &[u8]) -> Option<DynamicImage> {
    let image = image::load_from_memory(bytes).ok()?;
    Some(exif::apply_exif_orientation(image, bytes))
}

fn encode_png(image: &DynamicImage) -> Option<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let rgb = image.to_rgb8();
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

fn scaled(value: u32, numerator: u32, denominator: u32) -> u32 {
    let result = (f64::from(value) * f64::from(numerator) / f64::from(denominator)).round();
    // Bounded by the original dimensions.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "non-negative and bounded by u32 inputs"
    )]
    let result = result as u32;
    result
}

fn shrink(value: u32) -> u32 {
    if value == 1 {
        return 1;
    }
    let next = (f64::from(value) * 0.75).floor();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "non-negative and below the input"
    )]
    let next = next as u32;
    next.max(1)
}

/// pi `resizeImageInProcess`.
fn resize_image(bytes: &[u8], mime_type: &str, limits: ResizeLimits) -> Option<ResizedImage> {
    let input_base64_size = (bytes.len() as u64).div_ceil(3) * 4;
    let image = decode_oriented(bytes)?;
    let (original_width, original_height) = (image.width(), image.height());
    if original_width <= limits.max_width
        && original_height <= limits.max_height
        && input_base64_size < limits.max_bytes
    {
        return Some(ResizedImage {
            data: BASE64.encode(bytes),
            mime_type: mime_type.to_owned(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            was_resized: false,
        });
    }
    let (mut width, mut height) = (original_width, original_height);
    if width > limits.max_width {
        height = scaled(height, limits.max_width, width);
        width = limits.max_width;
    }
    if height > limits.max_height {
        width = scaled(width, limits.max_height, height);
        height = limits.max_height;
    }
    let mut qualities = vec![limits.jpeg_quality];
    for quality in [85, 70, 55, 40] {
        if !qualities.contains(&quality) {
            qualities.push(quality);
        }
    }
    loop {
        let resized = image.resize_exact(width.max(1), height.max(1), FilterType::Lanczos3);
        let candidates = std::iter::once(encode_png(&resized).map(|data| (data, "image/png")))
            .chain(
                qualities.iter().map(|quality| {
                    encode_jpeg(&resized, *quality).map(|data| (data, "image/jpeg"))
                }),
            );
        for (data, mime) in candidates.flatten() {
            let encoded = BASE64.encode(&data);
            if (encoded.len() as u64) < limits.max_bytes {
                return Some(ResizedImage {
                    data: encoded,
                    mime_type: mime.to_owned(),
                    original_width,
                    original_height,
                    width,
                    height,
                    was_resized: true,
                });
            }
        }
        if width == 1 && height == 1 {
            return None;
        }
        let (next_width, next_height) = (shrink(width), shrink(height));
        if next_width == width && next_height == height {
            return None;
        }
        width = next_width;
        height = next_height;
    }
}

/// pi `formatDimensionNote`.
fn dimension_note(resized: &ResizedImage) -> Option<String> {
    if !resized.was_resized {
        return None;
    }
    let scale = f64::from(resized.original_width) / f64::from(resized.width);
    Some(format!(
        "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {} to map to original image.]",
        resized.original_width,
        resized.original_height,
        resized.width,
        resized.height,
        crate::js::to_fixed(scale, 2)
    ))
}

fn conversion_hint(from: Option<&str>, to: &str) -> Option<String> {
    from.filter(|from| *from != to)
        .map(|from| format!("[Image converted from {from} to {to}.]"))
}

/// pi `processImage(bytes, mimeType, {autoResizeImages, resizeOptions})`.
#[must_use]
pub fn process_image(
    bytes: &[u8],
    mime_type: &str,
    auto_resize: bool,
    resize: Option<&ImageResize>,
) -> ProcessedImage {
    let (bytes, mime, converted_from) = if let Some(mime) = supported_inline_mime(mime_type) {
        (bytes.to_vec(), mime, None)
    } else {
        let Some(png) = decode_oriented(bytes).and_then(|image| encode_png(&image)) else {
            return ProcessedImage::Omitted {
                message:
                    "[Image omitted: could not be converted to a supported inline image format.]"
                        .to_owned(),
            };
        };
        (png, "image/png", Some(base_mime_type(mime_type)))
    };
    if !auto_resize {
        return ProcessedImage::Ok {
            data: BASE64.encode(&bytes),
            mime_type: mime.to_owned(),
            hints: conversion_hint(converted_from.as_deref(), mime)
                .into_iter()
                .collect(),
        };
    }
    let Some(resized) = resize_image(&bytes, mime, ResizeLimits::from_options(resize)) else {
        return ProcessedImage::Omitted {
            message: "[Image omitted: could not be resized below the inline image size limit.]"
                .to_owned(),
        };
    };
    let hints = conversion_hint(converted_from.as_deref(), &resized.mime_type)
        .into_iter()
        .chain(dimension_note(&resized))
        .collect();
    ProcessedImage::Ok {
        data: resized.data,
        mime_type: resized.mime_type,
        hints,
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgba};

    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let buffer = ImageBuffer::from_fn(width, height, |x, y| {
            Rgba([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                0,
                255,
            ])
        });
        encode_png(&DynamicImage::ImageRgba8(buffer)).unwrap()
    }

    #[test]
    fn small_png_is_passed_through() {
        let bytes = png(4, 3);
        let ProcessedImage::Ok {
            data,
            mime_type,
            hints,
        } = process_image(&bytes, "image/png", true, None)
        else {
            panic!("omitted");
        };
        assert_eq!(data, BASE64.encode(&bytes));
        assert_eq!(mime_type, "image/png");
        assert_eq!(hints, Vec::<String>::new());
    }

    #[test]
    fn large_png_is_resized_with_note() {
        let bytes = png(3000, 1000);
        let ProcessedImage::Ok { hints, .. } = process_image(&bytes, "image/png", true, None)
        else {
            panic!("omitted");
        };
        assert_eq!(
            hints,
            vec![
                "[Image: original 3000x1000, displayed at 2000x667. Multiply coordinates by 1.50 to map to original image.]"
            ]
        );
    }

    #[test]
    fn unresizable_image_is_omitted() {
        let bytes = png(20, 20);
        let limits = ImageResize {
            max_bytes: Some(10),
            ..ImageResize::default()
        };
        assert!(matches!(
            process_image(&bytes, "image/png", true, Some(&limits)),
            ProcessedImage::Omitted { .. }
        ));
    }
}
