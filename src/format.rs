use crate::error::{Error, ErrorKind};
pub trait Format {
    fn extension(&self) -> &'static str;

    fn mime_type(&self) -> &'static str;
}

#[derive(Debug)]
pub enum AudioFormat {
    Flac,
    Mp3(u16),
    Aac(u16),
}

impl Format for AudioFormat {
    fn extension(&self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Mp3(_) => "mp3",
            Self::Aac(_) => "m4a",
        }
    }

    fn mime_type(&self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Mp3(_) => "audio/mpeg",
            Self::Aac(_) => "audio/aac",
        }
    }
}

#[derive(Debug)]
pub enum CoverFormat {
    Png,
    Jpeg,
}

impl Format for CoverFormat {
    fn extension(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    fn mime_type(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

impl TryFrom<&str> for CoverFormat {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "image/jpeg" => Ok(CoverFormat::Jpeg),
            "image/png" => Ok(CoverFormat::Png),
            _ => {
                if value.ends_with(".jpg") {
                    Ok(CoverFormat::Jpeg)
                } else if value.ends_with(".png") {
                    Ok(CoverFormat::Png)
                } else {
                    Err(Error::new(ErrorKind::Other, "unknown cover format"))
                }
            }
        }
    }
}
