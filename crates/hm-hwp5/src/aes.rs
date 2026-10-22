//! AES-128 block cipher (FIPS-197), the cipher distribution documents use
//! in ECB mode.

/// Multiplies two elements of GF(2^8) modulo x^8 + x^4 + x^3 + x + 1.
fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let carry = a & 0x80 != 0;
        a <<= 1;
        if carry {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

/// The substitution box and its inverse: each byte's multiplicative
/// inverse put through the cipher's affine transform.
fn boxes() -> ([u8; 256], [u8; 256]) {
    let mut sbox = [0u8; 256];
    let mut inv = [0u8; 256];
    for x in 0..=255u8 {
        let b = if x == 0 {
            0
        } else {
            (1..=255u8).find(|&y| mul(x, y) == 1).unwrap_or(0)
        };
        let s =
            b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63;
        sbox[x as usize] = s;
        inv[s as usize] = x;
    }
    (sbox, inv)
}

/// An AES-128 key schedule.
pub struct Aes128 {
    round_keys: [[u8; 16]; 11],
    sbox: [u8; 256],
    inv_sbox: [u8; 256],
}

impl Aes128 {
    pub fn new(key: &[u8; 16]) -> Self {
        let (sbox, inv_sbox) = boxes();
        let mut w = [[0u8; 4]; 44];
        for (i, word) in w.iter_mut().take(4).enumerate() {
            word.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in 4..44 {
            let mut t = w[i - 1];
            if i % 4 == 0 {
                t = [
                    sbox[t[1] as usize] ^ rcon,
                    sbox[t[2] as usize],
                    sbox[t[3] as usize],
                    sbox[t[0] as usize],
                ];
                rcon = mul(rcon, 2);
            }
            for k in 0..4 {
                w[i][k] = w[i - 4][k] ^ t[k];
            }
        }
        let mut round_keys = [[0u8; 16]; 11];
        for (r, rk) in round_keys.iter_mut().enumerate() {
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Aes128 {
            round_keys,
            sbox,
            inv_sbox,
        }
    }

    fn add_key(state: &mut [u8; 16], key: &[u8; 16]) {
        for (s, k) in state.iter_mut().zip(key) {
            *s ^= k;
        }
    }

    /// Byte `r + 4c` is row `r` of column `c`; row `r` turns left by `r`.
    fn shift_rows(state: &mut [u8; 16], inverse: bool) {
        let old = *state;
        for r in 1..4 {
            for c in 0..4 {
                let from = if inverse {
                    (c + 4 - r) % 4
                } else {
                    (c + r) % 4
                };
                state[r + 4 * c] = old[r + 4 * from];
            }
        }
    }

    fn mix_columns(state: &mut [u8; 16], m: [u8; 4]) {
        for c in 0..4 {
            let a = [
                state[4 * c],
                state[4 * c + 1],
                state[4 * c + 2],
                state[4 * c + 3],
            ];
            for r in 0..4 {
                state[4 * c + r] = (0..4).fold(0, |acc, k| acc ^ mul(m[(k + 4 - r) % 4], a[k]));
            }
        }
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        Self::add_key(block, &self.round_keys[0]);
        for round in 1..=10 {
            for b in block.iter_mut() {
                *b = self.sbox[*b as usize];
            }
            Self::shift_rows(block, false);
            if round < 10 {
                Self::mix_columns(block, [2, 3, 1, 1]);
            }
            Self::add_key(block, &self.round_keys[round]);
        }
    }

    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        Self::add_key(block, &self.round_keys[10]);
        for round in (0..10).rev() {
            Self::shift_rows(block, true);
            for b in block.iter_mut() {
                *b = self.inv_sbox[*b as usize];
            }
            Self::add_key(block, &self.round_keys[round]);
            if round > 0 {
                Self::mix_columns(block, [0x0e, 0x0b, 0x0d, 0x09]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_fips_197_example() {
        let key: [u8; 16] = std::array::from_fn(|i| i as u8);
        let plain: [u8; 16] = std::array::from_fn(|i| (i as u8) * 0x11);
        let aes = Aes128::new(&key);
        let mut block = plain;
        aes.encrypt_block(&mut block);
        assert_eq!(
            block,
            [
                0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
                0xc5, 0x5a
            ]
        );
        aes.decrypt_block(&mut block);
        assert_eq!(block, plain);
    }
}
