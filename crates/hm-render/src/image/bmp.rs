//! Windows bitmaps (BMP files and bare DIBs): 1, 4, 8, 16, 24 and 32 bits
//! per pixel, uncompressed, RLE4, RLE8 and bit fields.

use super::Raster;

fn u16le(b: &[u8], o: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as u32)
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Decodes a BMP file or a DIB starting with its info header.
pub fn decode(data: &[u8]) -> Option<Raster> {
    let (dib, pixels_at) = if data.starts_with(b"BM") {
        (&data[14..], u32le(data, 10).map(|o| o as usize))
    } else {
        (data, None)
    };
    decode_dib(dib, pixels_at.map(|o| o.checked_sub(14)).unwrap_or(None))
}

/// Decodes a DIB (info header, color table, pixels). `pixels_at` is the
/// pixel offset from the header start when a file header gave it.
pub fn decode_dib(b: &[u8], pixels_at: Option<usize>) -> Option<Raster> {
    let header = u32le(b, 0)? as usize;
    let (width, height, bpp, compression, colors_used) = if header == 12 {
        (
            u16le(b, 4)? as i32,
            u16le(b, 6)? as i16 as i32,
            u16le(b, 10)?,
            0,
            0,
        )
    } else {
        (
            u32le(b, 4)? as i32,
            u32le(b, 8)? as i32,
            u16le(b, 14)?,
            u32le(b, 16)?,
            u32le(b, 32).unwrap_or(0),
        )
    };
    let top_down = height < 0;
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    if w == 0 || h == 0 || (w as u64 * h as u64) > (1 << 28) {
        return None;
    }
    // Bit-field masks follow a 40-byte header, or live inside larger ones.
    let mut masks = [0u32; 4];
    let mut table_at = header;
    if compression == 3 || compression == 6 {
        if header >= 52 {
            for (k, m) in masks
                .iter_mut()
                .enumerate()
                .take(if header >= 56 { 4 } else { 3 })
            {
                *m = u32le(b, 40 + 4 * k)?;
            }
        } else {
            for (k, m) in masks.iter_mut().enumerate().take(3) {
                *m = u32le(b, header + 4 * k)?;
            }
            table_at += 12;
        }
    }
    let entry = if header == 12 { 3 } else { 4 };
    let palette_len = if bpp <= 8 {
        if colors_used == 0 {
            1usize << bpp
        } else {
            colors_used.min(256) as usize
        }
    } else {
        0
    };
    let mut palette = Vec::with_capacity(palette_len);
    for k in 0..palette_len {
        let o = table_at + k * entry;
        match b.get(o..o + 3) {
            Some(c) => palette.push([c[2], c[1], c[0], 255]),
            None => palette.push([0, 0, 0, 255]),
        }
    }
    let pixels = pixels_at.unwrap_or(table_at + palette_len * entry);
    let src = b.get(pixels..)?;
    let mut out = Raster::new(w, h);
    let row_order = |y: u32| if top_down { y } else { h - 1 - y };
    match (bpp, compression) {
        (8, 1) | (4, 2) => rle(src, bpp, &palette, &mut out, &row_order),
        (1 | 2 | 4 | 8, _) => {
            let stride = ((w as usize * bpp as usize).div_ceil(32)) * 4;
            for y in 0..h {
                let row = src.get(y as usize * stride..(y as usize + 1) * stride)?;
                let dy = row_order(y);
                for x in 0..w {
                    let bit = x as usize * bpp as usize;
                    let byte = row[bit / 8];
                    let shift = 8 - bpp as usize - bit % 8;
                    let idx = ((byte >> shift) as usize) & ((1 << bpp) - 1);
                    let c = palette.get(idx).copied().unwrap_or([0, 0, 0, 255]);
                    out.set(x, dy, c);
                }
            }
        }
        (16 | 24 | 32, _) => {
            let bytes = bpp as usize / 8;
            let stride = (w as usize * bytes).div_ceil(4) * 4;
            let fields = compression == 3 || compression == 6;
            let m = if fields {
                masks
            } else if bpp == 16 {
                [0x7C00, 0x03E0, 0x001F, 0]
            } else {
                [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0]
            };
            // A 32-bit image whose alpha bytes are all zero is opaque.
            let mut any_alpha = false;
            let alpha_mask = if bpp == 32 && !fields {
                0xFF00_0000
            } else {
                m[3]
            };
            for y in 0..h {
                let row = src.get(y as usize * stride..)?;
                let dy = row_order(y);
                for x in 0..w as usize {
                    let o = x * bytes;
                    let v = match bytes {
                        2 => u16le(row, o)?,
                        3 => {
                            let p = row.get(o..o + 3)?;
                            p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16
                        }
                        _ => u32le(row, o)?,
                    };
                    let a = channel(v, alpha_mask);
                    any_alpha |= alpha_mask != 0 && a != 0;
                    out.set(
                        x as u32,
                        dy,
                        [
                            channel(v, m[0]),
                            channel(v, m[1]),
                            channel(v, m[2]),
                            if alpha_mask == 0 { 255 } else { a },
                        ],
                    );
                }
            }
            if alpha_mask != 0 && !any_alpha {
                out.rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            }
        }
        _ => return None,
    }
    Some(out)
}

/// Extracts a masked channel scaled to 8 bits.
fn channel(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let raw = (v & mask) >> shift;
    if bits >= 8 {
        (raw >> (bits - 8)) as u8
    } else {
        ((raw * 255) / ((1 << bits) - 1)) as u8
    }
}

/// Run-length encoded 4- and 8-bit bitmaps.
fn rle(src: &[u8], bpp: u32, palette: &[[u8; 4]], out: &mut Raster, row: &dyn Fn(u32) -> u32) {
    let (w, h) = (out.w, out.h);
    let (mut x, mut y) = (0u32, 0u32);
    let mut i = 0;
    let color = |k: usize| palette.get(k).copied().unwrap_or([0, 0, 0, 255]);
    let put = |out: &mut Raster, x: &mut u32, y: u32, idx: usize| {
        if *x < w && y < h {
            out.set(*x, row(y), color(idx));
        }
        *x += 1;
    };
    while i + 1 < src.len() && y < h {
        let (n, v) = (src[i] as usize, src[i + 1]);
        i += 2;
        if n > 0 {
            for k in 0..n {
                let idx = if bpp == 8 {
                    v as usize
                } else if k % 2 == 0 {
                    (v >> 4) as usize
                } else {
                    (v & 15) as usize
                };
                put(out, &mut x, y, idx);
            }
            continue;
        }
        match v {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                if i + 1 >= src.len() {
                    break;
                }
                x += src[i] as u32;
                y += src[i + 1] as u32;
                i += 2;
            }
            n => {
                let n = n as usize;
                let bytes = if bpp == 8 { n } else { n.div_ceil(2) };
                for k in 0..n {
                    let Some(&b) = src.get(i + if bpp == 8 { k } else { k / 2 }) else {
                        break;
                    };
                    let idx = if bpp == 8 {
                        b as usize
                    } else if k % 2 == 0 {
                        (b >> 4) as usize
                    } else {
                        (b & 15) as usize
                    };
                    put(out, &mut x, y, idx);
                }
                i += bytes + bytes % 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_up_24_bit() {
        // 2x2, bottom row red and green, top row blue and white.
        let mut b = Vec::new();
        b.extend(b"BM");
        b.extend(70u32.to_le_bytes());
        b.extend([0; 4]);
        b.extend(54u32.to_le_bytes());
        b.extend(40u32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(24u16.to_le_bytes());
        b.extend([0; 24]);
        b.extend([0, 0, 255, 0, 255, 0, 0, 0]);
        b.extend([255, 0, 0, 255, 255, 255, 0, 0]);
        let r = decode(&b).unwrap();
        assert_eq!((r.w, r.h), (2, 2));
        assert_eq!(r.get(0, 1), [255, 0, 0, 255]);
        assert_eq!(r.get(1, 1), [0, 255, 0, 255]);
        assert_eq!(r.get(0, 0), [0, 0, 255, 255]);
        assert_eq!(r.get(1, 0), [255, 255, 255, 255]);
    }
}
