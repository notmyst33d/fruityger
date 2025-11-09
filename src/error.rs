// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    error: Box<dyn std::error::Error + Send + Sync>,
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    ServiceError,
    RemuxError,
    InvalidUrlError,
    NoAvailableModules,
    UnsupportedCodecError,
    Other,
}

impl<E> From<E> for Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn from(value: E) -> Self {
        Self {
            kind: ErrorKind::Other,
            error: Box::new(value),
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(value: ErrorKind) -> Self {
        Self {
            kind: value,
            error: Box::from(""),
        }
    }
}

impl Error {
    pub fn new(kind: ErrorKind, err: &str) -> Self {
        Self {
            kind,
            error: Box::from(err),
        }
    }
}