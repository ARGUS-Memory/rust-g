use std::{
    io,
    num::{ParseFloatError, ParseIntError},
    result,
    str::Utf8Error,
};

#[cfg(feature = "png")]
use image::error::ImageError;
#[cfg(feature = "png")]
use png::{DecodingError, EncodingError};

#[cfg(feature = "unzip")]
use zip::result::ZipError;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Null,
    Utf8 { source: Utf8Error, position: usize },
    InvalidFilename,
    Io(io::Error),
    InvalidAlgorithm,
    #[cfg(feature = "png")]
    ImageDecoding(DecodingError),
    #[cfg(feature = "png")]
    ImageEncoding(EncodingError),
    #[cfg(any(feature = "iconforge", feature = "toml"))]
    JsonSerialization(serde_json::Error),
    ParseInt(ParseIntError),
    ParseFloat(ParseFloatError),
    #[cfg(feature = "png")]
    GenericImage(ImageError),
    #[cfg(feature = "png")]
    InvalidPngData,
    #[cfg(feature = "http")]
    Request(Box<ureq::Error>),
    #[cfg(feature = "sound_len")]
    SoundLen(String),
    #[cfg(feature = "toml")]
    TomlDeserialization(toml_dep::de::Error),
    #[cfg(feature = "toml")]
    TomlSerialization(toml_dep::ser::Error),
    #[cfg(feature = "unzip")]
    Unzip(ZipError),
    #[cfg(feature = "hash")]
    BadSeed,
    #[cfg(feature = "hash")]
    BadDigits,
    #[cfg(feature = "iconforge")]
    IconForge(String),
    #[cfg(feature = "dice")]
    DiceRoll(caith::RollError),
    Formatting(std::fmt::Error),
    #[cfg(feature = "dmi")]
    Dmi(dmi::error::DmiError),
    Panic(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => write!(f, "Illegal null character in string."),
            Self::Utf8 { position, .. } => write!(f, "Invalid UTF-8 character at position {position}."),
            Self::InvalidFilename => write!(f, "Invalid or empty filename specified."),
            Self::Io(e) => write!(f, "{e}"),
            Self::InvalidAlgorithm => write!(f, "Invalid algorithm specified."),
            #[cfg(feature = "png")]
            Self::ImageDecoding(e) => write!(f, "{e}"),
            #[cfg(feature = "png")]
            Self::ImageEncoding(e) => write!(f, "{e}"),
            #[cfg(any(feature = "iconforge", feature = "toml"))]
            Self::JsonSerialization(e) => write!(f, "{e}"),
            Self::ParseInt(e) => write!(f, "{e}"),
            Self::ParseFloat(e) => write!(f, "{e}"),
            #[cfg(feature = "png")]
            Self::GenericImage(e) => write!(f, "{e}"),
            #[cfg(feature = "png")]
            Self::InvalidPngData => write!(f, "Invalid png data."),
            #[cfg(feature = "http")]
            Self::Request(e) => write!(f, "{e}"),
            #[cfg(feature = "sound_len")]
            Self::SoundLen(s) => write!(f, "SoundLen error: {s}"),
            #[cfg(feature = "toml")]
            Self::TomlDeserialization(e) => write!(f, "{e}"),
            #[cfg(feature = "toml")]
            Self::TomlSerialization(e) => write!(f, "{e}"),
            #[cfg(feature = "unzip")]
            Self::Unzip(e) => write!(f, "{e}"),
            #[cfg(feature = "hash")]
            Self::BadSeed => write!(f, "TOTP seed is invalid length or not valid base32."),
            #[cfg(feature = "hash")]
            Self::BadDigits => write!(f, "TOTP may not be more than 8 digits."),
            #[cfg(feature = "iconforge")]
            Self::IconForge(s) => write!(f, "IconForge error: {s}"),
            #[cfg(feature = "dice")]
            Self::DiceRoll(e) => write!(f, "{e}"),
            Self::Formatting(e) => write!(f, "{e}"),
            #[cfg(feature = "dmi")]
            Self::Dmi(e) => write!(f, "{e}"),
            Self::Panic(s) => write!(f, "Panic during function execution: {s}"),
        }
    }
}

impl std::error::Error for Error {}

// From impls
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

impl From<ParseIntError> for Error {
    fn from(e: ParseIntError) -> Self { Self::ParseInt(e) }
}

impl From<ParseFloatError> for Error {
    fn from(e: ParseFloatError) -> Self { Self::ParseFloat(e) }
}

impl From<std::fmt::Error> for Error {
    fn from(e: std::fmt::Error) -> Self { Self::Formatting(e) }
}

#[cfg(feature = "dmi")]
impl From<dmi::error::DmiError> for Error {
    fn from(e: dmi::error::DmiError) -> Self { Self::Dmi(e) }
}

#[cfg(feature = "png")]
impl From<DecodingError> for Error {
    fn from(e: DecodingError) -> Self { Self::ImageDecoding(e) }
}

#[cfg(feature = "png")]
impl From<EncodingError> for Error {
    fn from(e: EncodingError) -> Self { Self::ImageEncoding(e) }
}

#[cfg(feature = "png")]
impl From<ImageError> for Error {
    fn from(e: ImageError) -> Self { Self::GenericImage(e) }
}

#[cfg(any(feature = "iconforge", feature = "toml"))]
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self { Self::JsonSerialization(e) }
}

#[cfg(feature = "http")]
impl From<Box<ureq::Error>> for Error {
    fn from(e: Box<ureq::Error>) -> Self { Self::Request(e) }
}

#[cfg(feature = "toml")]
impl From<toml_dep::de::Error> for Error {
    fn from(e: toml_dep::de::Error) -> Self { Self::TomlDeserialization(e) }
}

#[cfg(feature = "toml")]
impl From<toml_dep::ser::Error> for Error {
    fn from(e: toml_dep::ser::Error) -> Self { Self::TomlSerialization(e) }
}

#[cfg(feature = "unzip")]
impl From<ZipError> for Error {
    fn from(e: ZipError) -> Self { Self::Unzip(e) }
}

#[cfg(feature = "dice")]
impl From<caith::RollError> for Error {
    fn from(e: caith::RollError) -> Self { Self::DiceRoll(e) }
}

impl From<Utf8Error> for Error {
    fn from(source: Utf8Error) -> Self {
        Self::Utf8 {
            position: source.valid_up_to(),
            source,
        }
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

impl From<Error> for Vec<u8> {
    fn from(error: Error) -> Self {
        error.to_string().into_bytes()
    }
}
