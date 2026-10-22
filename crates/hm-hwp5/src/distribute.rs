//! Distribution documents (배포용 문서).
//!
//! A distribution document keeps its body in `ViewText/SectionN` instead of
//! `BodyText/SectionN`. Each protected stream starts with a 256-byte
//! distribution data record whose first four bytes seed the Microsoft C
//! runtime's `rand()`. The generator fills a 256-byte mask: an odd call
//! gives a byte value, the following even call how many times (1 to 16) it
//! repeats. The record XOR the mask holds, `(seed & 15) + 4` bytes in, the
//! 80-byte SHA-1 code of the document's password and two option bytes (1
//! no copying, 2 no printing). The first 16 bytes of that code are the
//! AES-128 key; the rest of the stream is the section record stream,
//! compressed as the document says, in AES-128 ECB.

use crate::aes::Aes128;

/// HWPTAG_DISTRIBUTE_DOC_DATA.
const TAG: u32 = 28;
const DATA_LEN: usize = 256;

/// The Microsoft C runtime's `rand()` from `srand(seed)`.
struct CRand(u32);

impl CRand {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);
        (self.0 >> 16) & 0x7FFF
    }
}

fn mask(seed: u32) -> [u8; DATA_LEN] {
    let mut out = [0u8; DATA_LEN];
    let mut rng = CRand(seed);
    let mut i = 0;
    while i < DATA_LEN {
        let value = (rng.next() & 0xFF) as u8;
        let count = (rng.next() & 0x0F) as usize + 1;
        for _ in 0..count {
            if i == DATA_LEN {
                break;
            }
            out[i] = value;
            i += 1;
        }
    }
    out
}

/// What a distribution data record protects: the password's SHA-1 code
/// (its first 16 bytes the AES key) and the option flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Protection {
    pub code: [u8; 80],
    pub options: u16,
}

impl Protection {
    fn cipher(&self) -> Aes128 {
        let key: [u8; 16] = self.code[..16].try_into().unwrap();
        Aes128::new(&key)
    }
}

/// Decodes a 256-byte distribution data record.
fn read_data(data: &[u8; DATA_LEN]) -> Protection {
    let seed = u32::from_le_bytes(data[..4].try_into().unwrap());
    let m = mask(seed);
    let plain: Vec<u8> = data.iter().zip(&m).map(|(d, k)| d ^ k).collect();
    let at = (seed & 0x0F) as usize + 4;
    Protection {
        code: plain[at..at + 80].try_into().unwrap(),
        options: u16::from_le_bytes([plain[at + 80], plain[at + 81]]),
    }
}

/// Builds a distribution data record for `p` from `seed`. Bytes outside the
/// code and options come from the mask itself.
fn write_data(p: &Protection, seed: u32) -> [u8; DATA_LEN] {
    let m = mask(seed);
    let mut plain = [0u8; DATA_LEN];
    let at = (seed & 0x0F) as usize + 4;
    plain[at..at + 80].copy_from_slice(&p.code);
    plain[at + 80..at + 82].copy_from_slice(&p.options.to_le_bytes());
    let mut data = [0u8; DATA_LEN];
    for i in 0..DATA_LEN {
        data[i] = plain[i] ^ m[i];
    }
    data[..4].copy_from_slice(&seed.to_le_bytes());
    data
}

/// Opens a protected stream: its protection and the decrypted payload (the
/// record stream, still compressed when the document is). `None` when the
/// stream does not start with a distribution data record.
pub fn open(raw: &[u8]) -> Option<(Protection, Vec<u8>)> {
    let header = u32::from_le_bytes(raw.get(..4)?.try_into().ok()?);
    if header & 0x3FF != TAG || (header >> 20) as usize != DATA_LEN {
        return None;
    }
    let data: [u8; DATA_LEN] = raw.get(4..4 + DATA_LEN)?.try_into().ok()?;
    let p = read_data(&data);
    let aes = p.cipher();
    let mut body = raw[4 + DATA_LEN..].to_vec();
    for block in body.chunks_exact_mut(16) {
        let b: &mut [u8; 16] = block.try_into().unwrap();
        aes.decrypt_block(b);
    }
    Some((p, body))
}

/// Seals a payload as a protected stream with a fresh distribution data
/// record seeded by `seed`, padding the payload to whole AES blocks.
pub fn seal(p: &Protection, seed: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + DATA_LEN + payload.len() + 16);
    out.extend_from_slice(&(TAG | (DATA_LEN as u32) << 20).to_le_bytes());
    out.extend_from_slice(&write_data(p, seed));
    let aes = p.cipher();
    let mut body = payload.to_vec();
    body.resize(payload.len().div_ceil(16) * 16, 0);
    for block in body.chunks_exact_mut(16) {
        let b: &mut [u8; 16] = block.try_into().unwrap();
        aes.encrypt_block(b);
    }
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_streams_open_again() {
        let p = Protection {
            code: std::array::from_fn(|i| (i * 7) as u8),
            options: 3,
        };
        let payload = b"section records".to_vec();
        let sealed = seal(&p, 0x4d3d_1806, &payload);
        let (q, body) = open(&sealed).unwrap();
        assert_eq!(q, p);
        assert_eq!(&body[..payload.len()], &payload[..]);
        assert!(body[payload.len()..].iter().all(|&b| b == 0));
    }
}
