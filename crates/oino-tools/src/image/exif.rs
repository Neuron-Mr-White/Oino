//! EXIF orientation (pi `utils/exif-orientation.ts`), applied before resizing.

use image::DynamicImage;

/// Byte at a possibly out-of-range index (JavaScript `undefined` → `0` in bit operations).
fn at(bytes: &[u8], index: i64) -> i64 {
    usize::try_from(index)
        .ok()
        .and_then(|index| bytes.get(index))
        .map_or(0, |byte| i64::from(*byte))
}

fn len(bytes: &[u8]) -> i64 {
    i64::try_from(bytes.len()).unwrap_or(i64::MAX)
}

fn read_orientation_from_tiff(bytes: &[u8], tiff_start: i64) -> u8 {
    if tiff_start + 8 > len(bytes) {
        return 1;
    }
    let little_endian = ((at(bytes, tiff_start) << 8) | at(bytes, tiff_start + 1)) == 0x4949;
    let read16 = |pos: i64| {
        if little_endian {
            at(bytes, pos) | (at(bytes, pos + 1) << 8)
        } else {
            (at(bytes, pos) << 8) | at(bytes, pos + 1)
        }
    };
    let read32 = |pos: i64| {
        if little_endian {
            // JavaScript `|` yields a signed 32-bit value here.
            let value = at(bytes, pos)
                | (at(bytes, pos + 1) << 8)
                | (at(bytes, pos + 2) << 16)
                | (at(bytes, pos + 3) << 24);
            i64::from(
                u32::try_from(value & 0xffff_ffff)
                    .unwrap_or(0)
                    .cast_signed(),
            )
        } else {
            (at(bytes, pos) << 24)
                | (at(bytes, pos + 1) << 16)
                | (at(bytes, pos + 2) << 8)
                | at(bytes, pos + 3)
        }
    };
    let ifd_start = tiff_start + read32(tiff_start + 4);
    if ifd_start + 2 > len(bytes) {
        return 1;
    }
    let entries = read16(ifd_start);
    for index in 0..entries {
        let entry = ifd_start + 2 + index * 12;
        if entry + 12 > len(bytes) {
            return 1;
        }
        if read16(entry) == 0x0112 {
            let value = read16(entry + 8);
            return u8::try_from(value)
                .ok()
                .filter(|v| (1..=8).contains(v))
                .unwrap_or(1);
        }
    }
    1
}

fn has_exif_header(bytes: &[u8], offset: i64) -> bool {
    [0x45, 0x78, 0x69, 0x66, 0x00, 0x00]
        .iter()
        .enumerate()
        .all(|(index, expected)| at(bytes, offset + i64::try_from(index).unwrap_or(0)) == *expected)
}

fn find_jpeg_tiff_offset(bytes: &[u8]) -> Option<i64> {
    let mut offset = 2;
    while offset < len(bytes) - 1 {
        if at(bytes, offset) != 0xff {
            return None;
        }
        let marker = at(bytes, offset + 1);
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if marker == 0xe1 {
            if offset + 4 >= len(bytes) {
                return None;
            }
            let segment_start = offset + 4;
            if segment_start + 6 > len(bytes) {
                return None;
            }
            if has_exif_header(bytes, segment_start) {
                return Some(segment_start + 6);
            }
        }
        if offset + 4 > len(bytes) {
            return None;
        }
        let length = (at(bytes, offset + 2) << 8) | at(bytes, offset + 3);
        offset += 2 + length;
    }
    None
}

fn find_webp_tiff_offset(bytes: &[u8]) -> Option<i64> {
    let mut offset = 12;
    while offset + 8 <= len(bytes) {
        let id: Vec<i64> = (0..4).map(|i| at(bytes, offset + i)).collect();
        let size = at(bytes, offset + 4)
            | (at(bytes, offset + 5) << 8)
            | (at(bytes, offset + 6) << 16)
            | (at(bytes, offset + 7) << 24);
        let data_start = offset + 8;
        if id == [0x45, 0x58, 0x49, 0x46] {
            if data_start + size > len(bytes) {
                return None;
            }
            return Some(if size >= 6 && has_exif_header(bytes, data_start) {
                data_start + 6
            } else {
                data_start
            });
        }
        offset = data_start + size + (size % 2);
    }
    None
}

/// pi `getExifOrientation`: 1–8, `1` when absent.
#[must_use]
pub(crate) fn exif_orientation(bytes: &[u8]) -> u8 {
    let tiff = if bytes.starts_with(&[0xff, 0xd8]) {
        find_jpeg_tiff_offset(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        find_webp_tiff_offset(bytes)
    } else {
        None
    };
    tiff.map_or(1, |offset| read_orientation_from_tiff(bytes, offset))
}

/// pi `applyExifOrientation`.
#[must_use]
pub(crate) fn apply_exif_orientation(image: DynamicImage, original: &[u8]) -> DynamicImage {
    match exif_orientation(original) {
        2 => image.fliph(),
        3 => image.fliph().flipv(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        _ => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_jpeg_orientation() {
        // SOI, APP1 with Exif header, big-endian TIFF, one IFD entry: orientation 6.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1, 0x00, 0x22];
        bytes.extend_from_slice(b"Exif\0\0");
        bytes.extend_from_slice(&[0x4d, 0x4d, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x08]);
        bytes.extend_from_slice(&[
            0x00, 0x01, 0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00,
        ]);
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(exif_orientation(&bytes), 6);
        assert_eq!(exif_orientation(b"plain"), 1);
    }
}
