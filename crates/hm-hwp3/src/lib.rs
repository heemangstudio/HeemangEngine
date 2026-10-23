//! HWP 3.x reader: the Hangul 97 file format mapped into the shared model.

mod convert;
pub mod file;
mod object;
pub mod text;

#[derive(Debug)]
pub enum Error {
    NotHwp3,
    Truncated,
    TooLarge,
    Encrypted,
    Corrupt(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotHwp3 => f.write_str("not an HWP 3 document"),
            Error::Truncated => f.write_str("truncated"),
            Error::TooLarge => f.write_str("body exceeds the size limit"),
            Error::Encrypted => f.write_str("password-protected document"),
            Error::Corrupt(m) => write!(f, "corrupt: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// True when `bytes` start with the HWP 3 signature.
pub fn is_hwp3(bytes: &[u8]) -> bool {
    bytes.starts_with(file::SIGNATURE)
}

/// Reads an HWP 3 document.
pub fn read(bytes: hm_model::SharedBytes) -> Result<hm_model::Document, Error> {
    let f = file::parse(&bytes)?;
    Ok(convert::document(&f))
}
