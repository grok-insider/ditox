use std::fmt;

#[derive(Debug)]
pub enum Error {
    Sqlite(String),
    Io(String),
    Clipboard(String),
    PasteBack(String),
    Config(String),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(detail) => write!(f, "database: {detail}"),
            Error::Io(detail) => write!(f, "file: {detail}"),
            Error::Clipboard(detail) => write!(f, "clipboard: {detail}"),
            Error::PasteBack(detail) => write!(f, "paste: {detail}"),
            Error::Config(detail) => write!(f, "config: {detail}"),
            Error::Invalid(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Sqlite(err.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
