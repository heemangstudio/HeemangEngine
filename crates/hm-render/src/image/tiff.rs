//! TIFF pictures: the first image of the file, in strips or tiles,
//! uncompressed, PackBits, LZW or Deflate, with the horizontal predictor;
//! bilevel, gray, palette, RGB and CMYK samples of 1 to 16 bits, with or
//! without alpha.

use super::Raster;

struct Reader<'a> {
    d: &'a [u8],
    le: bool,
}

impl Reader<'_> {
    fn u16(&self, o: usize) -> Option<u32> {
        let b = self.d.get(o..o + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        } as u32)
    }

    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    /// The values of an IFD entry: SHORT and LONG arrays, inline or at an
    /// offset.
    fn values(&self, entry: usize) -> Option<Vec<u32>> {
        let kind = self.u16(entry + 2)?;
        let count = self.u32(entry + 4)? as usize;
        let size = match kind {
            3 => 2,
            4 => 4,
            1 => 1,
            _ => return None,
        };
        if count > 1 << 24 {
            return None;
        }
        let at = if count * size <= 4 {
            entry + 8
        } else {
            self.u32(entry + 8)? as usize
        };
        (0..count)
            .map(|k| match size {
                1 => self.d.get(at + k).map(|&b| b as u32),
                2 => self.u16(at + 2 * k),
                _ => self.u32(at + 4 * k),
            })
            .collect()
    }
}

/// The resolution the file declares, in dots per inch.
pub fn resolution(d: &[u8]) -> Option<(f32, f32)> {
    let r = Reader {
        d,
        le: d.starts_with(b"II"),
    };
    let ifd = r.u32(4)? as usize;
    let (mut x, mut y, mut unit) = (None, None, 2);
    for k in 0..r.u16(ifd)? as usize {
        let e = ifd + 2 + 12 * k;
        let rational = || {
            let at = r.u32(e + 8)? as usize;
            let (n, dd) = (r.u32(at)?, r.u32(at + 4)?);
            (dd != 0 && n != 0).then(|| n as f32 / dd as f32)
        };
        match r.u16(e)? {
            282 => x = rational(),
            283 => y = rational(),
            296 => unit = r.u16(e + 8)?,
            _ => {}
        }
    }
    let per_inch = match unit {
        3 => 2.54,
        2 => 1.0,
        _ => return None,
    };
    Some((x? * per_inch, y.or(x)? * per_inch))
}

pub fn decode(d: &[u8]) -> Option<Raster> {
    let le = d.starts_with(b"II");
    let r = Reader { d, le };
    let ifd = r.u32(4)? as usize;
    let n = r.u16(ifd)? as usize;
    let mut tags = std::collections::HashMap::new();
    for k in 0..n {
        let e = ifd + 2 + 12 * k;
        if let (Some(tag), Some(v)) = (r.u16(e), r.values(e)) {
            tags.insert(tag, v);
        }
    }
    let one = |t: u32, default: u32| {
        tags.get(&t)
            .and_then(|v| v.first().copied())
            .unwrap_or(default)
    };
    let (w, h) = (one(256, 0), one(257, 0));
    if w == 0 || h == 0 || w as u64 * h as u64 > 1 << 28 {
        return None;
    }
    let spp = one(277, 1).clamp(1, 8) as usize;
    let bps = one(258, 1) as usize;
    if !matches!(bps, 1 | 2 | 4 | 8 | 16) {
        return None;
    }
    let compression = one(259, 1);
    let photometric = one(262, 1);
    let planar = one(284, 1);
    let predictor = one(317, 1);
    if planar != 1 && spp > 1 {
        return None;
    }
    // Strips or tiles: (offsets, byte counts, piece width, piece height).
    let (offsets, counts, pw, ph) = if let Some(o) = tags.get(&324) {
        (o.clone(), tags.get(&325)?.clone(), one(322, w), one(323, h))
    } else {
        let rows = one(278, h).min(h).max(1);
        (tags.get(&273)?.clone(), tags.get(&279)?.clone(), w, rows)
    };
    let row_bytes = (pw as usize * spp * bps).div_ceil(8);
    let mut samples = vec![0u16; w as usize * h as usize * spp];
    let across = w.div_ceil(pw);
    for (k, (&off, &len)) in offsets.iter().zip(&counts).enumerate() {
        let src = d.get(off as usize..(off as usize + len as usize).min(d.len()))?;
        let want = row_bytes * ph as usize;
        let mut data = match compression {
            1 => src.to_vec(),
            5 => lzw(src, want)?,
            8 | 32946 => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(src, want)
                .ok()
                .or_else(|| miniz_oxide::inflate::decompress_to_vec_with_limit(src, want).ok())?,
            32773 => packbits(src, want),
            _ => return None,
        };
        data.resize(want, 0);
        if predictor == 2 {
            unpredict(&mut data, row_bytes, spp, bps);
        }
        let (px, py) = ((k as u32 % across) * pw, (k as u32 / across) * ph);
        for row in 0..ph {
            let y = py + row;
            if y >= h {
                break;
            }
            let line = &data[row as usize * row_bytes..(row as usize + 1) * row_bytes];
            for col in 0..pw {
                let x = px + col;
                if x >= w {
                    break;
                }
                for s in 0..spp {
                    let i = col as usize * spp + s;
                    let v = match bps {
                        16 => {
                            let b = &line[2 * i..2 * i + 2];
                            if le {
                                u16::from_le_bytes([b[0], b[1]])
                            } else {
                                u16::from_be_bytes([b[0], b[1]])
                            }
                        }
                        8 => line[i] as u16,
                        _ => {
                            let bit = i * bps;
                            ((line[bit / 8] >> (8 - bps - bit % 8)) & ((1 << bps) - 1) as u8) as u16
                        }
                    };
                    samples[(y as usize * w as usize + x as usize) * spp + s] = v;
                }
            }
        }
    }
    let max = (1u32 << bps) - 1;
    let scale = |v: u16| (v as u32 * 255 / max) as u8;
    let map = tags.get(&320);
    let alpha = tags.contains_key(&338);
    let mut out = Raster::new(w, h);
    for (k, px) in samples.chunks_exact(spp).enumerate() {
        let (x, y) = (k as u32 % w, k as u32 / w);
        let c = match photometric {
            0 => {
                let g = 255 - scale(px[0]);
                [g, g, g, 255]
            }
            3 => {
                let n = 1usize << bps;
                let m = map?;
                let at =
                    |s: usize| (m.get(s * n + px[0] as usize).copied().unwrap_or(0) >> 8) as u8;
                [at(0), at(1), at(2), 255]
            }
            2 | 6 if spp >= 3 => {
                let a = if alpha && spp >= 4 { scale(px[3]) } else { 255 };
                [scale(px[0]), scale(px[1]), scale(px[2]), a]
            }
            5 if spp >= 4 => {
                let k = 255 - scale(px[3]) as u32;
                let f = |v: u16| ((255 - scale(v) as u32) * k / 255) as u8;
                [f(px[0]), f(px[1]), f(px[2]), 255]
            }
            _ => {
                let g = scale(px[0]);
                let a = if alpha && spp >= 2 { scale(px[1]) } else { 255 };
                [g, g, g, a]
            }
        };
        out.set(x, y, c);
    }
    Some(out)
}

/// TIFF's LZW: most significant bit first, code size growing one code
/// early.
fn lzw(src: &[u8], want: usize) -> Option<Vec<u8>> {
    let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    let mut out = Vec::with_capacity(want);
    let r = dec.into_vec(&mut out).decode(src);
    if r.status.is_err() && out.is_empty() {
        return None;
    }
    Some(out)
}

fn packbits(src: &[u8], want: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(want);
    let mut i = 0;
    while i < src.len() && out.len() < want {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let len = n as usize + 1;
            out.extend_from_slice(&src[i..(i + len).min(src.len())]);
            i += len;
        } else if n != -128 {
            if let Some(&b) = src.get(i) {
                out.extend(std::iter::repeat_n(b, (1 - n as isize) as usize));
            }
            i += 1;
        }
    }
    out
}

/// Undoes horizontal differencing of 8- and 16-bit samples.
fn unpredict(data: &mut [u8], row_bytes: usize, spp: usize, bps: usize) {
    for row in data.chunks_exact_mut(row_bytes) {
        match bps {
            8 => {
                for i in spp..row.len() {
                    row[i] = row[i].wrapping_add(row[i - spp]);
                }
            }
            16 => {
                for i in (2 * spp..row.len() - 1).step_by(2) {
                    let prev = u16::from_be_bytes([row[i - 2 * spp], row[i - 2 * spp + 1]]);
                    let cur = u16::from_be_bytes([row[i], row[i + 1]]).wrapping_add(prev);
                    row[i..i + 2].copy_from_slice(&cur.to_be_bytes());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncompressed_rgb_strip() {
        // Little-endian 2x1 RGB: red then blue.
        let mut d = b"II*\0".to_vec();
        d.extend(8u32.to_le_bytes());
        let entries: [(u16, u16, u32, u32); 7] = [
            (256, 3, 1, 2),
            (257, 3, 1, 1),
            (258, 3, 1, 8),
            (262, 3, 1, 2),
            (273, 4, 1, 8 + 2 + 7 * 12 + 4),
            (277, 3, 1, 3),
            (279, 4, 1, 6),
        ];
        d.extend((entries.len() as u16).to_le_bytes());
        for (t, k, c, v) in entries {
            d.extend(t.to_le_bytes());
            d.extend(k.to_le_bytes());
            d.extend(c.to_le_bytes());
            d.extend(v.to_le_bytes());
        }
        d.extend(0u32.to_le_bytes());
        d.extend([255, 0, 0, 0, 0, 255]);
        let r = decode(&d).unwrap();
        assert_eq!(r.get(0, 0), [255, 0, 0, 255]);
        assert_eq!(r.get(1, 0), [0, 0, 255, 255]);
    }
}
