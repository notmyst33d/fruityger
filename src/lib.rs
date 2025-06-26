// SPDX-License-Identifier: MIT
// Copyright 2025 Myst33d

mod error;
pub mod qobuz;
mod remux;
pub mod yandex;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use reqwest::header;
use serde::Serialize;

pub use crate::{error::Error, remux::Metadata};

#[macro_export]
macro_rules! const_headers {
    ($slice:expr) => {{
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        for (k, v) in $slice {
            headers.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
        }
        headers
    }};
}

#[derive(Debug, Serialize, Clone)]
pub enum AudioFormat {
    Flac,
    Mp3(u16),
    Aac(u16),
}

#[derive(Debug, Serialize, Clone)]
pub enum CoverFormat {
    Png,
    Jpg,
}

impl CoverFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            CoverFormat::Png => "png",
            CoverFormat::Jpg => "jpg",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            CoverFormat::Png => "image/png",
            CoverFormat::Jpg => "image/jpeg",
        }
    }
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3(_) => "mp3",
            AudioFormat::Aac(_) => "m4a",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "audio/flac",
            AudioFormat::Mp3(_) => "audio/mpeg",
            AudioFormat::Aac(_) => "audio/mp4",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration_ms: usize,
    pub artists: Vec<Artist>,
    pub cover_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

#[async_trait]
pub trait Module: Send + Sync {
    fn name(&self) -> &'static str;

    fn url_supported(&self, url: &str) -> bool;

    fn supported_formats(&self) -> &'static [AudioFormat];

    async fn search(&self, query: &str, page: usize) -> Result<SearchResults, Error>;

    async fn download(
        &self,
        workdir: &Path,
        filename: &str,
        url: &str,
    ) -> Result<(AudioFormat, PathBuf), Error>;

    fn box_clone(&self) -> Box<dyn Module>;
}

#[derive(Clone)]
pub struct Client {
    modules: Vec<Box<dyn Module>>,
}

impl Clone for Box<dyn Module> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

impl Client {
    pub fn new() -> Self {
        Self { modules: vec![] }
    }

    pub fn add_module(&mut self, module: impl Module + 'static) {
        self.modules.push(Box::new(module));
    }

    pub fn module_exists(&self, module_name: &str) -> bool {
        for module in &self.modules {
            if module.name() == module_name {
                return true;
            }
        }
        false
    }

    pub async fn download(
        &self,
        workdir: &Path,
        filename: &str,
        url: &str,
    ) -> Result<(AudioFormat, PathBuf), Error> {
        for module in &self.modules {
            if !module.url_supported(url) {
                continue;
            }
            return module.download(workdir, filename, url).await;
        }
        Err(Error::NoAvailableModules)
    }

    pub async fn download_cover(
        &self,
        workdir: &Path,
        url: &str,
    ) -> Result<(CoverFormat, PathBuf), Error> {
        let response = reqwest::Client::new().get(url).send().await?;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg");
        let format = match content_type {
            "image/jpeg" => CoverFormat::Jpg,
            "image/png" => CoverFormat::Png,
            _ => return Err(Error::ServiceError("unsupported format".to_string())),
        };
        let cover_file = workdir.join(format!("cover.{}", format.extension()));
        Ok((format, cover_file))
    }

    pub async fn search(
        &self,
        module_name: &str,
        query: &str,
        page: usize,
    ) -> Result<SearchResults, Error> {
        for module in &self.modules {
            if module.name() != module_name {
                continue;
            }
            return module.search(query, page).await;
        }
        Err(Error::NoAvailableModules)
    }

    pub async fn remux(
        &self,
        workdir: &PathBuf,
        filename: &str,
        audio: (AudioFormat, PathBuf),
        cover: (CoverFormat, PathBuf),
        metadata: Metadata,
    ) -> Result<(AudioFormat, PathBuf), Error> {
        let out = workdir.join(filename);
        remux::remux(audio.1, cover.1, &out, metadata)?;
        Ok((audio.0, out))
    }
}
