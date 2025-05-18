mod error;
mod ffi;
pub mod qobuz;
mod remux;
pub mod yandex;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use serde::Serialize;
use std::pin::Pin;
use tempfile::tempdir;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};
use tokio_stream::wrappers::ReceiverStream;

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

pub type BytesStream = Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>;

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
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3(_) => "mp3",
            AudioFormat::Aac(_) => "m4a",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3(_) => "mp3",
            AudioFormat::Aac(_) => "aac",
        }
    }

    pub fn bitrate(&self) -> u16 {
        match self {
            AudioFormat::Flac => 0,
            AudioFormat::Mp3(bitrate) => *bitrate,
            AudioFormat::Aac(bitrate) => *bitrate,
        }
    }
}

impl Into<String> for &AudioFormat {
    fn into(self) -> String {
        match self {
            AudioFormat::Flac => String::from(self.name()),
            AudioFormat::Mp3(bitrate) => format!("{}_{bitrate}", self.name()),
            AudioFormat::Aac(bitrate) => format!("{}_{bitrate}", self.name()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
}

#[derive(Debug, Serialize)]
pub struct Track {
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration_ms: usize,
    pub artists: Vec<Artist>,
    pub cover_url: String,
}

#[derive(Debug, Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

#[async_trait]
pub trait Module {
    fn name(&self) -> &'static str;

    fn url_supported(&self, url: &str) -> bool;

    fn supported_formats(&self) -> &'static [AudioFormat];

    async fn search(&self, query: &str, page: usize) -> Result<SearchResults, Error>;

    async fn download(&self, url: &str) -> Result<(AudioFormat, BytesStream), Error>;

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
    pub async fn download(&self, url: &str) -> Result<(AudioFormat, BytesStream), Error> {
        for module in &self.modules {
            if !module.url_supported(url) {
                continue;
            }
            return module.download(url).await;
        }
        Err(Error::NoAvailableModules)
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
        mut audio: (AudioFormat, BytesStream),
        mut cover: (CoverFormat, BytesStream),
        metadata: Metadata,
    ) -> Result<(AudioFormat, BytesStream), Error> {
        let temp_dir = tempdir()?;
        let input_audio = temp_dir
            .path()
            .join(format!("audio.{}", audio.0.extension()));
        let mut input_audio_file = File::create(&input_audio).await?;
        while let Some(chunk) = audio.1.next().await {
            input_audio_file.write_all(&chunk?).await?;
        }

        let input_cover = temp_dir
            .path()
            .join(format!("cover.{}", cover.0.extension()));
        let mut input_cover_file = File::create(&input_cover).await?;
        while let Some(chunk) = cover.1.next().await {
            input_cover_file.write_all(&chunk?).await?;
        }

        let out = temp_dir.path().join(format!("out.{}", audio.0.extension()));
        remux::remux(input_audio, input_cover, &out, metadata)?;

        let out_file = File::open(out).await?;
        let (tx, rx) = mpsc::channel(16);
        tokio::task::spawn(async move {
            let mut reader = BufReader::new(out_file);
            let mut bytes = BytesMut::zeroed(65535);
            while reader.read(&mut bytes).await.unwrap_or(0) != 0 {
                let _ = tx.send(Ok(bytes.freeze())).await;
                bytes = BytesMut::zeroed(65535);
            }
        });

        Ok((audio.0, Box::pin(ReceiverStream::new(rx))))
    }
}
