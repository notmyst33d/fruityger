// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

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

impl From<url::ParseError> for Error {
    fn from(value: url::ParseError) -> Self {
        Self::UrlError(UrlError::ParseError(value))
    }
}
