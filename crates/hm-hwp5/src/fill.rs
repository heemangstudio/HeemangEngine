//! The fill description (HWP 5 table 28) shared by border fills and
//! drawing objects.
//!
//! Hancom writes the gradient with a one-byte kind and 32-bit fields and the
//! picture's contrast before its brightness (HWPX twins of the same document
//! settle the order), keeps the gradient step center in the length-prefixed
//! extra attributes and appends one alpha byte per present part.

use hm_model::{Fill, Gradient, ImageFill, SolidFill};

use crate::Error;
use crate::record::{Reader, Writer};

/// Reads a fill. Parts are read in stored order until one does not fit;
/// what was read is returned and the reader stops before the rest.
pub fn read_fill(r: &mut Reader<'_>) -> Result<Fill, Error> {
    let mut f = Fill {
        kind: r.u32()?,
        ..Fill::default()
    };
    // Parts that do not fit stay unread for the caller's tail.
    let _ = read_parts(r, &mut f);
    Ok(f)
}

fn read_parts(r: &mut Reader<'_>, f: &mut Fill) -> Result<(), Error> {
    if f.kind & Fill::SOLID != 0 {
        f.solid = Some(r.try_read(|r| {
            Ok(SolidFill {
                background: r.u32()?,
                pattern_color: r.u32()?,
                pattern: r.i32()?,
            })
        })?);
    }
    if f.kind & Fill::GRADIENT != 0 {
        f.gradient = Some(r.try_read(|r| {
            let mut g = Gradient {
                kind: r.u8()?,
                angle: r.i32()?,
                center_x: r.i32()?,
                center_y: r.i32()?,
                step: r.i32()?,
                ..Gradient::default()
            };
            let n = r.i32()?;
            if !(0..=256).contains(&n) {
                return Err(Error::Truncated);
            }
            if n > 2 {
                g.positions = (0..n).map(|_| r.i32()).collect::<Result<_, _>>()?;
            }
            g.colors = (0..n).map(|_| r.u32()).collect::<Result<_, _>>()?;
            Ok(g)
        })?);
    }
    if f.kind & Fill::IMAGE != 0 {
        f.image = Some(r.try_read(|r| {
            Ok(ImageFill {
                mode: r.u8()?,
                contrast: r.i8()?,
                brightness: r.i8()?,
                effect: r.u8()?,
                bin_id: r.u16()?,
            })
        })?);
    }
    let extra = r.try_read(|r| {
        let n = r.u32()? as usize;
        r.bytes(n)
    })?;
    if let (Some(g), Some(&c)) = (f.gradient.as_mut(), extra.first()) {
        g.step_center = Some(c);
    }
    f.extra = Some(extra);
    let parts = [f.solid.is_some(), f.gradient.is_some(), f.image.is_some()];
    for _ in parts.iter().filter(|p| **p) {
        if r.remaining() == 0 {
            break;
        }
        f.alpha.push(r.u8()?);
    }
    Ok(())
}

/// Writes a fill as [`read_fill`] reads it.
pub fn write_fill(w: &mut Writer, f: &Fill) {
    w.u32(f.kind);
    if let Some(s) = &f.solid {
        w.u32(s.background);
        w.u32(s.pattern_color);
        w.i32(s.pattern);
    }
    if let Some(g) = &f.gradient {
        w.u8(g.kind);
        w.i32(g.angle);
        w.i32(g.center_x);
        w.i32(g.center_y);
        w.i32(g.step);
        w.i32(g.colors.len() as i32);
        if g.colors.len() > 2 {
            for k in 0..g.colors.len() {
                w.i32(g.positions.get(k).copied().unwrap_or(0));
            }
        }
        g.colors.iter().for_each(|&c| w.u32(c));
    }
    if let Some(i) = &f.image {
        w.u8(i.mode);
        w.i8(i.contrast);
        w.i8(i.brightness);
        w.u8(i.effect);
        w.u16(i.bin_id);
    }
    if let Some(e) = &f.extra {
        w.u32(e.len() as u32);
        w.bytes(e);
    }
    w.bytes(&f.alpha);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_model::SharedBytes;

    fn round_trip(bytes: &[u8]) -> Fill {
        let b = SharedBytes::from(bytes);
        let mut r = Reader::new(&b);
        let f = read_fill(&mut r).unwrap();
        assert_eq!(r.remaining(), 0, "fill fully read");
        let mut w = Writer::default();
        write_fill(&mut w, &f);
        assert_eq!(w.0, bytes);
        f
    }

    #[test]
    fn gradient_reads_as_hancom_writes_it() {
        // Linear, 90 degrees, 255 steps, two colors, step center 50, alpha 0.
        let mut b = vec![4, 0, 0, 0, 1, 90, 0, 0, 0];
        b.extend([0; 8]);
        b.extend([255, 0, 0, 0, 2, 0, 0, 0]);
        b.extend([0xef, 0xfe, 0xf9, 0, 0xbf, 0xfc, 0xde, 0]);
        b.extend([1, 0, 0, 0, 50, 0]);
        let f = round_trip(&b);
        let g = f.gradient.unwrap();
        assert_eq!(
            (g.kind, g.angle, g.step, g.step_center),
            (1, 90, 255, Some(50))
        );
        assert_eq!(g.colors, vec![0x00f9feef, 0x00defcbf]);
        assert_eq!(f.alpha, vec![0]);
    }

    #[test]
    fn solid_with_alpha_and_truncated_fills() {
        let mut b = vec![1, 0, 0, 0];
        b.extend(0x00c08000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        b.extend([0, 0, 0, 0, 0]);
        let f = round_trip(&b);
        assert_eq!(f.background(), Some(0x00c08000));

        // A record cut inside the solid part keeps only the kind.
        let cut = SharedBytes::from(&b[..10]);
        let mut r = Reader::new(&cut);
        let f = read_fill(&mut r).unwrap();
        assert!(f.solid.is_none());
        assert_eq!(r.pos(), 4);
    }
}
