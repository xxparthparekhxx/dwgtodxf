use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a DWG file at all.
    NotDwg,
    /// A DWG version we do not read (yet).
    Unsupported(String),
    /// Read past the end of a buffer.
    Eof,
    /// Structurally broken file.
    Corrupt(String),
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotDwg => write!(f, "not a DWG file"),
            Error::Unsupported(s) => write!(f, "unsupported: {s}"),
            Error::Eof => write!(f, "unexpected end of data"),
            Error::Corrupt(s) => write!(f, "corrupt DWG: {s}"),
            Error::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// Initial capacity for a vector whose length comes from the file: never
/// trust it for a large up-front allocation.
#[inline]
pub(crate) fn cap(n: usize) -> usize {
    n.min(1 << 16)
}

/// Largest logical section we accept, relative to the input file size.
/// Real DWG sections compress at most a few hundred times; anything bigger
/// is a corrupt or hostile file.
pub(crate) fn check_section_size(size: u64, file_len: usize) -> Result<usize> {
    let limit = (file_len as u64).saturating_mul(512).saturating_add(64 << 20).min(4 << 30);
    if size > limit {
        return Err(Error::Corrupt(format!("section size {size} is implausible")));
    }
    Ok(size as usize)
}

pub(crate) fn corrupt<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Corrupt(msg.into()))
}
