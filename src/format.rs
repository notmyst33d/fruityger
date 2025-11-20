use crate::error::Error;

pub trait Format {
    fn extension(&self) -> &'static str;

    fn mime_type(&self) -> &'static str;
}

#[derive(Debug, Clone)]
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
            Self::Aac(_) => "audio/mp4",
        }
    }
}

impl TryFrom<&str> for AudioFormat {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "audio/flac" | "audio/x-flac" => Ok(AudioFormat::Flac),
            "audio/mpeg" | "audio/mpg" => Ok(AudioFormat::Mp3(0)),
            "audio/mp4" | "video/mp4" | "audio/aac" => Ok(AudioFormat::Aac(0)),
            _ => {
                if value.ends_with(".flac") {
                    Ok(AudioFormat::Flac)
                } else if value.ends_with(".mp3") {
                    Ok(AudioFormat::Mp3(0))
                } else if value.ends_with(".m4a") || value.ends_with(".mp4") {
                    Ok(AudioFormat::Aac(0))
                } else {
                    Err(Error::UnsupportedFormatError)
                }
            }
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
                    Err(Error::UnsupportedFormatError)
                }
            }
        }
    }
}
