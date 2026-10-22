//! The seven script slots of a character shape.

/// Hancom's language slots. A character shape stores a face, ratio, spacing,
/// relative size and offset for each; every character is measured with the
/// values of its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Script {
    Hangul = 0,
    Latin = 1,
    Hanja = 2,
    Japanese = 3,
    Other = 4,
    Symbol = 5,
    User = 6,
}

impl Script {
    pub const ALL: [Script; 7] = [
        Script::Hangul,
        Script::Latin,
        Script::Hanja,
        Script::Japanese,
        Script::Other,
        Script::Symbol,
        Script::User,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Script {
        Script::ALL.get(i).copied().unwrap_or(Script::User)
    }
}
