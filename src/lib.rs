// SPDX-License-Identifier: MIT
// Copyright 2025 Myst33d

mod error;
pub mod qobuz;
pub mod remux;
pub mod yandex;

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::header;
use serde::Serialize;
use tempfile::tempdir;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::{error::Error, remux::Metadata};

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

    async fn download(&self, url: &str) -> Result<(AudioFormat, Bytes), Error>;

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

    pub async fn download(&self, url: &str) -> Result<(AudioFormat, Bytes), Error> {
        for module in &self.modules {
            if !module.url_supported(url) {
                continue;
            }
            return module.download(url).await;
        }
        Err(Error::NoAvailableModules)
    }

    pub async fn download_cover(&self, url: &str) -> Result<(CoverFormat, Bytes), Error> {
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
        Ok((format, response.bytes().await?))
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
        audio: (AudioFormat, Bytes),
        cover: (CoverFormat, Bytes),
        metadata: Metadata,
    ) -> Result<(AudioFormat, Bytes), Error> {
        let temp_dir = tempdir()?;
        let input_audio = temp_dir
            .path()
            .join(format!("audio.{}", audio.0.extension()));
        let mut input_audio_file = File::create(&input_audio).await?;
        input_audio_file.write_all(&audio.1).await?;

        let input_cover = temp_dir
            .path()
            .join(format!("cover.{}", cover.0.extension()));
        let mut input_cover_file = File::create(&input_cover).await?;
        input_cover_file.write_all(&cover.1).await?;

        let out = temp_dir.path().join(format!("out.{}", audio.0.extension()));
        remux::remux(input_audio, input_cover, &out, metadata)?;

        Ok((audio.0, {
            let mut b = vec![];
            File::open(out).await?.read_to_end(&mut b).await?;
            b.into()
        }))
    }
}
