// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

use thiserror::Error;

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    // Library errors
    #[error("service error: {0}")]
    ServiceError(String),

    #[error("unsupported format")]
    UnsupportedFormatError,

    // Foreign errors
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    RemuxError(#[from] ffmpeg_next::Error),

    #[error(transparent)]
    JsonDeserializationError(#[from] serde_json::Error),

    #[error(transparent)]
    RequestError(#[from] reqwest::Error),

    #[error(transparent)]
    UrlParseError(#[from] url::ParseError),

    #[error(transparent)]
    EnvError(#[from] std::env::VarError),
}
