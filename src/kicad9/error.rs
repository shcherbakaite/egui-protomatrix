//! Error types for KiCad 9 footprint parsing.

use std::fmt;

/// Parse or I/O error.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse { message: String, offset: Option<usize> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::Parse { message, offset } => {
                if let Some(o) = offset {
                    write!(f, "Parse error at offset {}: {}", o, message)
                } else {
                    write!(f, "Parse error: {}", message)
                }
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
