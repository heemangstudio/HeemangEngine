//! HWPUNIT arithmetic.
//!
//! Layout works in `i32` HU: 7200 per inch, 100 per point. Scaling follows
//! Hancom's MulDiv: a 64-bit product rounded half away from zero, with `-1`
//! for a zero divisor or a result outside `i32`.

/// One HWPUNIT, 1/7200 inch.
pub type Hu = i32;

pub const HU_PER_INCH: i32 = 7200;
pub const HU_PER_POINT: i32 = 100;

/// Hancom's MulDiv: `a * b / c` in 64 bits, rounded half away from zero.
///
/// A negative divisor negates `a` first. Returns `-1` when `c` is zero or the
/// result does not fit in `i32`.
pub fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let (a, c) = if c < 0 {
        (a.wrapping_neg(), -(c as i64))
    } else {
        (a, c as i64)
    };
    let product = a as i64 * b as i64;
    let magnitude = (product.unsigned_abs() as i64 + c / 2) / c;
    let result = if product < 0 { -magnitude } else { magnitude };
    i32::try_from(result).unwrap_or(-1)
}

/// Decodes a packed distance: the value shifted left once, bit 0 marking a
/// percentage of `basis` (character-relative lengths).
///
/// Paragraph margins, indents, paragraph spacing, non-percent line spacing,
/// tab positions and the section default tab use this form, which is why
/// they read as "doubled" HU.
pub fn unpack_distance(raw: i32, basis: impl FnOnce() -> i32) -> Hu {
    let value = raw >> 1;
    if raw & 1 != 0 {
        mul_div(value, basis(), 100)
    } else {
        value
    }
}

/// Packs an absolute distance in HU.
pub fn pack_distance(hu: Hu) -> i32 {
    hu.wrapping_shl(1)
}

/// Millimetres to HU, as Hancom stores paper sizes (rounded to nearest).
pub fn hu_from_mm(mm: f64) -> Hu {
    (mm * HU_PER_INCH as f64 / 25.4).round() as i32
}

/// HU to device pixels at `dpi`, as a float for backends.
pub fn hu_to_px(hu: Hu, dpi: f32) -> f32 {
    hu as f32 * dpi / HU_PER_INCH as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_rounds_half_away_from_zero() {
        assert_eq!(mul_div(5, 1, 2), 3);
        assert_eq!(mul_div(-5, 1, 2), -3);
        assert_eq!(mul_div(600, 250, 1200), 125);
        assert_eq!(mul_div(7, 3, 10), 2);
        assert_eq!(mul_div(1, 1, 0), -1);
        assert_eq!(mul_div(i32::MAX, 2, 1), -1);
        assert_eq!(mul_div(10, 1, -4), -3);
    }

    #[test]
    fn packed_distances() {
        assert_eq!(unpack_distance(8000, || 0), 4000);
        assert_eq!(unpack_distance(-6, || 0), -3);
        assert_eq!(unpack_distance((50 << 1) | 1, || 1000), 500);
        assert_eq!(pack_distance(1500), 3000);
    }
}
