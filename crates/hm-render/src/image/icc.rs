//! Embedded gray ICC profiles. Hancom converts a gray picture through its
//! profile's tone curve to sRGB, so a dot-gain profile lightens mid grays.

/// The sRGB value of each gray level under a gray profile, or `None` for
/// other profiles and curves this reader does not know.
pub fn gray_to_srgb(profile: &[u8]) -> Option<[u8; 256]> {
    let be32 = |i: usize| Some(u32::from_be_bytes(profile.get(i..i + 4)?.try_into().ok()?));
    if profile.get(16..20)? != b"GRAY" {
        return None;
    }
    let count = be32(128)? as usize;
    let trc = (0..count.min(256)).find_map(|t| {
        let at = 132 + 12 * t;
        (profile.get(at..at + 4)? == b"kTRC")
            .then(|| Some((be32(at + 4)? as usize, be32(at + 8)? as usize)))
            .flatten()
    })?;
    let tag = profile.get(trc.0..trc.0.checked_add(trc.1)?)?;
    let curve = Curve::parse(tag)?;
    let mut lut = [0u8; 256];
    for (v, out) in lut.iter_mut().enumerate() {
        let y = curve.eval(v as f64 / 255.0).clamp(0.0, 1.0);
        let e = if y <= 0.003_130_8 {
            12.92 * y
        } else {
            1.055 * y.powf(1.0 / 2.4) - 0.055
        };
        *out = (e * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    Some(lut)
}

enum Curve {
    Gamma(f64),
    Table(Vec<f64>),
    /// ICC parametric curve: function type and its parameters.
    Para(u16, [f64; 7]),
}

impl Curve {
    fn parse(tag: &[u8]) -> Option<Curve> {
        let be16 = |i: usize| Some(u16::from_be_bytes(tag.get(i..i + 2)?.try_into().ok()?));
        let be32 = |i: usize| Some(u32::from_be_bytes(tag.get(i..i + 4)?.try_into().ok()?));
        match tag.get(0..4)? {
            b"curv" => match be32(8)? as usize {
                0 => Some(Curve::Gamma(1.0)),
                1 => Some(Curve::Gamma(f64::from(be16(12)?) / 256.0)),
                n => (0..n)
                    .map(|i| be16(12 + 2 * i).map(|v| f64::from(v) / 65535.0))
                    .collect::<Option<Vec<_>>>()
                    .map(Curve::Table),
            },
            b"para" => {
                let kind = be16(8)?;
                let n = [1, 3, 4, 5, 7].get(usize::from(kind))?;
                let mut p = [0.0; 7];
                for (i, v) in p.iter_mut().take(*n).enumerate() {
                    *v = f64::from(be32(12 + 4 * i)? as i32) / 65536.0;
                }
                Some(Curve::Para(kind, p))
            }
            _ => None,
        }
    }

    fn eval(&self, x: f64) -> f64 {
        match self {
            Curve::Gamma(g) => x.powf(*g),
            Curve::Table(t) => {
                let pos = x * (t.len() - 1) as f64;
                let i = (pos.floor() as usize).min(t.len().saturating_sub(2));
                let f = pos - i as f64;
                t[i] + (t.get(i + 1).unwrap_or(&t[i]) - t[i]) * f
            }
            &Curve::Para(kind, [g, a, b, c, d, e, f]) => match kind {
                0 => x.powf(g),
                1 if x >= -b / a => (a * x + b).powf(g),
                1 => 0.0,
                2 if x >= -b / a => (a * x + b).powf(g) + c,
                2 => c,
                3 if x >= d => (a * x + b).powf(g),
                3 => c * x,
                _ if x >= d => (a * x + b).powf(g) + e,
                _ => c * x + f,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gray profile holding only the tone curve tag.
    fn gray_profile(curve: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 144];
        p[16..20].copy_from_slice(b"GRAY");
        p[128..132].copy_from_slice(&1u32.to_be_bytes());
        p[132..136].copy_from_slice(b"kTRC");
        p[136..140].copy_from_slice(&144u32.to_be_bytes());
        p[140..144].copy_from_slice(&(curve.len() as u32).to_be_bytes());
        p.extend_from_slice(curve);
        p
    }

    #[test]
    fn a_linear_gray_profile_lifts_mid_grays_to_srgb() {
        let lut = gray_to_srgb(&gray_profile(b"curv\0\0\0\0\0\0\0\0")).unwrap();
        assert_eq!([lut[0], lut[64], lut[128], lut[255]], [0, 137, 188, 255]);
    }

    #[test]
    fn tabulated_tone_curves_interpolate_between_entries() {
        // Two entries: the identity curve in linear light.
        let mut curve = b"curv\0\0\0\0\0\0\0\x02".to_vec();
        curve.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        let table = gray_to_srgb(&gray_profile(&curve)).unwrap();
        assert_eq!(
            table,
            gray_to_srgb(&gray_profile(b"curv\0\0\0\0\0\0\0\0")).unwrap()
        );
    }

    #[test]
    fn color_profiles_leave_pictures_alone() {
        let mut p = gray_profile(b"curv\0\0\0\0\0\0\0\0");
        p[16..20].copy_from_slice(b"RGB ");
        assert!(gray_to_srgb(&p).is_none());
    }
}
