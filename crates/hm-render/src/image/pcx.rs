//! ZSoft PCX pictures: 8-bit paletted and 24-bit three-plane images with
//! run-length rows.

use super::Raster;

pub fn decode(d: &[u8]) -> Option<Raster> {
    if d.len() < 128 || d[0] != 0x0A {
        return None;
    }
    let rd = |o: usize| u16::from_le_bytes([d[o], d[o + 1]]) as u32;
    let (bpp, planes) = (d[3] as u32, d[65] as u32);
    let (x0, y0, x1, y1) = (rd(4), rd(6), rd(8), rd(10));
    let (w, h) = (x1.checked_sub(x0)? + 1, y1.checked_sub(y0)? + 1);
    let stride = rd(66) as usize;
    if w as u64 * h as u64 > (1 << 28) || stride == 0 {
        return None;
    }
    // Decode every scan line (all planes) first.
    let line = stride * planes as usize;
    let mut raw = Vec::with_capacity(line * h as usize);
    let mut i = 128;
    while raw.len() < line * h as usize && i < d.len() {
        let b = d[i];
        i += 1;
        if b & 0xC0 == 0xC0 {
            let n = (b & 0x3F) as usize;
            let v = *d.get(i)?;
            i += 1;
            raw.extend(std::iter::repeat_n(v, n));
        } else {
            raw.push(b);
        }
    }
    raw.resize(line * h as usize, 0);
    let mut out = Raster::new(w, h);
    match (bpp, planes) {
        (8, 1) => {
            // The 256-color palette follows a 12 marker at the end.
            let pal = d
                .len()
                .checked_sub(768)
                .filter(|&p| p > 0 && d[p - 1] == 12)?;
            for y in 0..h {
                for x in 0..w {
                    let k = raw[y as usize * line + x as usize] as usize;
                    let c = &d[pal + k * 3..pal + k * 3 + 3];
                    out.set(x, y, [c[0], c[1], c[2], 255]);
                }
            }
        }
        (8, 3) | (8, 4) => {
            for y in 0..h {
                for x in 0..w as usize {
                    let at = |p: usize| raw[y as usize * line + p * stride + x];
                    out.set(x as u32, y, [at(0), at(1), at(2), 255]);
                }
            }
        }
        (1, 1) => {
            for y in 0..h {
                for x in 0..w as usize {
                    let byte = raw[y as usize * line + x / 8];
                    let v = if byte & (0x80 >> (x % 8)) != 0 {
                        255
                    } else {
                        0
                    };
                    out.set(x as u32, y, [v, v, v, 255]);
                }
            }
        }
        (1, 4) | (4, 1) => {
            let pal: Vec<[u8; 3]> = (0..16)
                .map(|k| [d[16 + k * 3], d[17 + k * 3], d[18 + k * 3]])
                .collect();
            for y in 0..h {
                for x in 0..w as usize {
                    let k = if bpp == 4 {
                        let b = raw[y as usize * line + x / 2];
                        if x % 2 == 0 { b >> 4 } else { b & 15 }
                    } else {
                        (0..4).fold(0u8, |acc, p| {
                            let b = raw[y as usize * line + p * stride + x / 8];
                            acc | (((b >> (7 - x % 8)) & 1) << p)
                        })
                    } as usize;
                    let c = pal[k];
                    out.set(x as u32, y, [c[0], c[1], c[2], 255]);
                }
            }
        }
        _ => return None,
    }
    Some(out)
}
