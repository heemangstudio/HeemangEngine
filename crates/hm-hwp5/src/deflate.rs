//! Raw deflate as HWP 5 streams use it.

use miniz_oxide::inflate::{self, TINFLStatus};

use crate::Error;

/// Inflates a raw deflate stream, rejecting output beyond `limit` bytes.
///
/// Bytes after the final block (Hancom's CRC-32 and length trailer) are ignored.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    match inflate::decompress_to_vec_with_limit(data, limit) {
        Ok(v) => Ok(v),
        Err(e) if e.status == TINFLStatus::HasMoreOutput => Err(Error::TooLarge),
        Err(e) => {
            // Some writers truncate the final block; keep what decoded.
            if !e.output.is_empty() && e.status == TINFLStatus::FailedCannotMakeProgress {
                Ok(e.output)
            } else {
                Err(Error::Corrupt(format!("deflate: {:?}", e.status)))
            }
        }
    }
}

/// Deflates `data` (level 9, raw) and appends Hancom's trailer: CRC-32 of the
/// inflated bytes, then the inflated length.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut out = miniz_oxide::deflate::compress_to_vec(data, 9);
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// CRC-32 (IEEE 802.3, reflected).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_crc() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let data: Vec<u8> = (0..10000u32).map(|i| (i % 13) as u8).collect();
        let z = deflate(&data);
        assert_eq!(inflate(&z, 1 << 20).unwrap(), data);
        assert!(matches!(inflate(&z, 100), Err(Error::TooLarge)));
    }
}
