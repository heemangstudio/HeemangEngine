//! Picture decoding and picture effects.
//!
//! Raster formats decode to straight RGBA: PNG through tiny-skia, JPEG and
//! GIF through their crates, BMP and PCX here. Metafiles are played at the
//! size they are drawn. Formats are recognized by their signature, not by
//! the file extension the document records.

mod bmp;
mod pcx;
pub mod wmf;

use tiny_skia::Pixmap;

/// A decoded picture: straight (not premultiplied) RGBA rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    pub fn new(w: u32, h: u32) -> Raster {
        Raster {
            w,
            h,
            rgba: vec![0; w as usize * h as usize * 4],
        }
    }

    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        let o = (y as usize * self.w as usize + x as usize) * 4;
        if let Some(p) = self.rgba.get_mut(o..o + 4) {
            p.copy_from_slice(&c);
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let o = (y as usize * self.w as usize + x as usize) * 4;
        self.rgba
            .get(o..o + 4)
            .map_or([0; 4], |p| [p[0], p[1], p[2], p[3]])
    }

    /// A premultiplied pixmap for drawing.
    pub fn to_pixmap(&self) -> Option<Pixmap> {
        let mut pm = Pixmap::new(self.w, self.h)?;
        for (d, s) in pm
            .data_mut()
            .chunks_exact_mut(4)
            .zip(self.rgba.chunks_exact(4))
        {
            let a = s[3] as u32;
            d[0] = ((s[0] as u32 * a + 127) / 255) as u8;
            d[1] = ((s[1] as u32 * a + 127) / 255) as u8;
            d[2] = ((s[2] as u32 * a + 127) / 255) as u8;
            d[3] = s[3];
        }
        Some(pm)
    }
}

/// The picture formats recognized by signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Pcx,
    Tiff,
    Wmf,
    Emf,
    Unknown,
}

pub fn sniff(d: &[u8]) -> Format {
    match d {
        [0x89, b'P', b'N', b'G', ..] => Format::Png,
        [0xFF, 0xD8, ..] => Format::Jpeg,
        [b'G', b'I', b'F', b'8', ..] => Format::Gif,
        [b'B', b'M', ..] => Format::Bmp,
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => Format::Tiff,
        [0xD7, 0xCD, 0xC6, 0x9A, ..] | [1 | 2, 0, 9, 0, ..] => Format::Wmf,
        [1, 0, 0, 0, ..] if d.get(40..44) == Some(b" EMF") => Format::Emf,
        [0x0A, 0..=5, 1, 1 | 2 | 4 | 8, ..] => Format::Pcx,
        // A bare DIB: its info header size.
        [12 | 40 | 52 | 56 | 108 | 124, 0, 0, 0, ..] => Format::Bmp,
        _ => Format::Unknown,
    }
}

/// Decodes a raster picture. Vector metafiles and unknown data give `None`.
pub fn decode(data: &[u8]) -> Option<Raster> {
    match sniff(data) {
        Format::Png => png(data),
        Format::Jpeg => jpeg(data),
        Format::Gif => gif(data),
        Format::Bmp => bmp::decode(data),
        Format::Pcx => pcx::decode(data),
        _ => None,
    }
}

fn png(data: &[u8]) -> Option<Raster> {
    let pm = Pixmap::decode_png(data).ok()?;
    let mut r = Raster::new(pm.width(), pm.height());
    for (d, s) in r.rgba.chunks_exact_mut(4).zip(pm.pixels()) {
        let c = s.demultiply();
        d.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(r)
}

fn jpeg(data: &[u8]) -> Option<Raster> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(data), opts);
    let pixels = dec.decode().ok()?;
    let (w, h) = dec.dimensions()?;
    let (w, h) = (w as u32, h as u32);
    if pixels.len() != w as usize * h as usize * 4 {
        return None;
    }
    Some(Raster { w, h, rgba: pixels })
}

fn gif(data: &[u8]) -> Option<Raster> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mut dec = opts.read_info(data).ok()?;
    let (w, h) = (dec.width() as u32, dec.height() as u32);
    let mut out = Raster::new(w, h);
    let frame = dec.read_next_frame().ok()??;
    // The first frame may cover part of the logical screen.
    for y in 0..frame.height as u32 {
        for x in 0..frame.width as u32 {
            let o = (y as usize * frame.width as usize + x as usize) * 4;
            let Some(p) = frame.buffer.get(o..o + 4) else {
                continue;
            };
            let (dx, dy) = (x + frame.left as u32, y + frame.top as u32);
            if dx < w && dy < h {
                out.set(dx, dy, [p[0], p[1], p[2], p[3]]);
            }
        }
    }
    Some(out)
}

/// Hancom's picture effects, applied to a decoded picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Effects {
    /// 0 real, 1 grayscale, 2 black and white, 4 pattern.
    pub effect: u8,
    /// Percent, -100 to 100.
    pub brightness: i8,
    pub contrast: i8,
    /// 0 opaque to 255 transparent.
    pub alpha: u8,
}

impl Effects {
    pub fn is_none(&self) -> bool {
        *self == Effects::default()
    }

    pub fn apply(&self, r: &mut Raster) {
        if self.is_none() {
            return;
        }
        let bright = self.brightness as i32 * 255 / 100;
        // Contrast scales around mid gray; +100 doubles, -100 flattens.
        let contrast = 100 + self.contrast as i32;
        let keep = 255 - self.alpha as u32;
        for p in r.rgba.chunks_exact_mut(4) {
            let mut c = [p[0] as i32, p[1] as i32, p[2] as i32];
            if self.effect == 1 || self.effect == 2 {
                let y = (c[0] * 299 + c[1] * 587 + c[2] * 114) / 1000;
                c = [y; 3];
            }
            for v in &mut c {
                *v = ((*v - 128) * contrast / 100 + 128 + bright).clamp(0, 255);
            }
            if self.effect == 2 {
                let v = if c[0] >= 128 { 255 } else { 0 };
                c = [v; 3];
            }
            p[0] = c[0] as u8;
            p[1] = c[1] as u8;
            p[2] = c[2] as u8;
            p[3] = ((p[3] as u32 * keep + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_pick_the_decoder() {
        assert_eq!(sniff(b"\x89PNG\r\n"), Format::Png);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF]), Format::Jpeg);
        assert_eq!(sniff(b"GIF89a"), Format::Gif);
        assert_eq!(sniff(&[0xD7, 0xCD, 0xC6, 0x9A, 0, 0]), Format::Wmf);
    }

    #[test]
    fn grayscale_keeps_luma() {
        let mut r = Raster::new(1, 1);
        r.set(0, 0, [255, 0, 0, 255]);
        Effects {
            effect: 1,
            ..Default::default()
        }
        .apply(&mut r);
        assert_eq!(r.get(0, 0), [76, 76, 76, 255]);
    }
}
