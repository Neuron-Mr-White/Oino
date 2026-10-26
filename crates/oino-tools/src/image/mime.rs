//! Image type sniffing (pi `utils/mime.ts`).

/// Bytes read from the start of a file for sniffing (pi `IMAGE_TYPE_SNIFF_BYTES`).
pub const IMAGE_TYPE_SNIFF_BYTES: usize = 4100;

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// pi `detectSupportedImageMimeType`: `image/jpeg`, `image/png` (not animated), `image/gif`,
/// `image/webp` or `image/bmp` from magic bytes.
#[must_use]
pub fn detect_supported_image_mime_type(buffer: &[u8]) -> Option<&'static str> {
    if buffer.starts_with(&[0xff, 0xd8, 0xff]) {
        return (buffer.get(3) != Some(&0xf7)).then_some("image/jpeg");
    }
    if buffer.starts_with(&PNG_SIGNATURE) {
        return (is_png(buffer) && !is_animated_png(buffer)).then_some("image/png");
    }
    if ascii_at(buffer, 0, b"GIF87a") || ascii_at(buffer, 0, b"GIF89a") {
        return Some("image/gif");
    }
    if ascii_at(buffer, 0, b"RIFF") && ascii_at(buffer, 8, b"WEBP") {
        return Some("image/webp");
    }
    if ascii_at(buffer, 0, b"BM") && is_bmp(buffer) {
        return Some("image/bmp");
    }
    None
}

fn ascii_at(buffer: &[u8], offset: usize, text: &[u8]) -> bool {
    buffer
        .get(offset..offset + text.len())
        .is_some_and(|slice| slice == text)
}

fn byte(buffer: &[u8], offset: usize) -> u32 {
    buffer.get(offset).copied().map_or(0, u32::from)
}

fn u16_le(buffer: &[u8], offset: usize) -> u32 {
    byte(buffer, offset) | (byte(buffer, offset + 1) << 8)
}

fn u32_be(buffer: &[u8], offset: usize) -> u64 {
    (u64::from(byte(buffer, offset)) << 24)
        | (u64::from(byte(buffer, offset + 1)) << 16)
        | (u64::from(byte(buffer, offset + 2)) << 8)
        | u64::from(byte(buffer, offset + 3))
}

fn u32_le(buffer: &[u8], offset: usize) -> u64 {
    u64::from(byte(buffer, offset))
        | (u64::from(byte(buffer, offset + 1)) << 8)
        | (u64::from(byte(buffer, offset + 2)) << 16)
        | (u64::from(byte(buffer, offset + 3)) << 24)
}

fn is_png(buffer: &[u8]) -> bool {
    buffer.len() >= 16 && u32_be(buffer, PNG_SIGNATURE.len()) == 13 && ascii_at(buffer, 12, b"IHDR")
}

fn is_animated_png(buffer: &[u8]) -> bool {
    let len = buffer.len() as u64;
    let mut offset = PNG_SIGNATURE.len() as u64;
    while offset + 8 <= len {
        let Ok(at) = usize::try_from(offset) else {
            return false;
        };
        let chunk_length = u32_be(buffer, at);
        if ascii_at(buffer, at + 4, b"acTL") {
            return true;
        }
        if ascii_at(buffer, at + 4, b"IDAT") {
            return false;
        }
        let next = offset + 8 + chunk_length + 4;
        if next <= offset || next > len {
            return false;
        }
        offset = next;
    }
    false
}

fn is_bmp(buffer: &[u8]) -> bool {
    if buffer.len() < 26 {
        return false;
    }
    let declared_size = u32_le(buffer, 2);
    let pixel_offset = u32_le(buffer, 10);
    let dib_size = u32_le(buffer, 14);
    if declared_size != 0 && declared_size < 26 {
        return false;
    }
    if pixel_offset < 14 + dib_size {
        return false;
    }
    if declared_size != 0 && pixel_offset >= declared_size {
        return false;
    }
    let (planes, bits) = if dib_size == 12 {
        (u16_le(buffer, 22), u16_le(buffer, 24))
    } else if (40..=124).contains(&dib_size) {
        if buffer.len() < 30 {
            return false;
        }
        (u16_le(buffer, 26), u16_le(buffer, 28))
    } else {
        return false;
    };
    planes == 1 && [1, 4, 8, 16, 24, 32].contains(&bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_magic() {
        assert_eq!(
            detect_supported_image_mime_type(b"GIF89a...."),
            Some("image/gif")
        );
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xe0]),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xf7]),
            None
        );
        assert_eq!(
            detect_supported_image_mime_type(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(detect_supported_image_mime_type(b"hello"), None);
    }
}
