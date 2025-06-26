// SPDX-License-Identifier: MIT
// Copyright 2025 Myst33d

macro_rules! transparent_from_error {
    ($into:ty, $from:ty) => {
        impl From<$from> for $into {
            fn from(value: $from) -> Self {
                <$from>::from(value).into()
            }
        }
    };
}

#[derive(thiserror::Error, Debug)]
pub enum UrlError {
    #[error(transparent)]
    ParseError(#[from] url::ParseError),

    #[error("invalid path")]
    InvalidPathError,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ConnectionError(#[from] reqwest::Error),

    #[error(transparent)]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    RemuxError(#[from] ffmpeg_next::Error),

    #[error(transparent)]
    UrlError(#[from] UrlError),

    #[error(transparent)]
    DeserializationError(#[from] serde_json::Error),

    #[error("unsupported codec")]
    UnsupportedCodecError,

    #[error("no available modules")]
    NoAvailableModules,

    #[error("service error")]
    ServiceError(String),
}

transparent_from_error!(Error, url::ParseError);
