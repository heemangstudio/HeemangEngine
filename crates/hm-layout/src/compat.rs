//! The compatibility profile in force for a document.
//!
//! A document stores a target program and five flag words (character,
//! paragraph, section, object, field). Hancom does not apply the stored
//! words as they are: it looks through its built-in profiles of the same
//! target, in order, and takes the first one whose words all equal the
//! stored ones or any of whose words is numerically lower than the stored
//! word in the same place. Only when no profile qualifies do the stored
//! words apply.

use hm_model::DocInfo;

/// Hancom's built-in profiles in catalog order: target and the five words.
const PROFILES: [(u32, [u32; 5]); 4] = [
    // None.
    (0, [0, 0, 0, 0, 0]),
    // Hangul 2007 (Hwp70).
    (1, [0x20c, 0x700, 0x30, 0x13, 1]),
    // MS Word.
    (2, [0x1e3, 0xcf_f8ff, 0x4f, 0x1ec, 0]),
    // Hunminjeongeum.
    (3, [0, 0x31_0001, 0, 0x24, 0]),
];

/// The five flag words in force for a stored target and stored words.
pub fn select(target: u32, stored: [u32; 5]) -> [u32; 5] {
    PROFILES
        .iter()
        .filter(|(t, _)| *t == target)
        .map(|(_, words)| words)
        .find(|words| **words == stored || words.iter().zip(&stored).any(|(c, s)| c < s))
        .copied()
        .unwrap_or(stored)
}

/// The flag words in force for a document.
pub fn effective(info: &DocInfo) -> [u32; 5] {
    let target = info.compatible.as_ref().map_or(0, |c| c.target);
    let stored = info.layout_compat.as_ref().map_or([0; 5], |c| c.flags);
    select(target, stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_hancom_order() {
        // A current-target document with any flag set falls to the empty
        // profile.
        assert_eq!(select(0, [0, 4, 0, 0, 0]), [0; 5]);
        // The Word profile itself selects itself.
        let word = PROFILES[2].1;
        assert_eq!(select(2, word), word);
        // Stored words below every Word word keep the stored words.
        assert_eq!(select(2, [0x1e3, 0x100, 0, 0, 0]), [0x1e3, 0x100, 0, 0, 0]);
        // One stored word above the profile's selects the profile.
        assert_eq!(select(2, [0, 0, 0, 0, 1]), word);
        // An unknown target keeps what it stored.
        assert_eq!(select(9, [1, 2, 3, 4, 5]), [1, 2, 3, 4, 5]);
    }
}
