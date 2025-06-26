// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

mod error;
pub mod qobuz;
pub mod yandex;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use ffmpeg_next::{Dictionary, codec, encoder, ffi::AV_DISPOSITION_ATTACHED_PIC, format, media};
use futures::TryStreamExt;
use reqwest::header;
use tokio::fs::File;

pub use crate::error::Error;

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

#[derive(Default)]
pub struct Metadata {
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub artist: String,
    pub composer: Option<String>,
    pub copyright: Option<String>,
    pub creation_time: Option<String>,
    pub date: Option<String>,
    pub disc: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub performer: Option<String>,
    pub publisher: Option<String>,
    pub title: String,
    pub track: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AudioFormat {
    Flac,
    Mp3(u16),
    Aac(u16),
}

#[derive(Debug, Clone)]
pub enum CoverFormat {
    Png,
    Jpg,
}

impl CoverFormat {
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }

    pub const fn mime_type(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpg => "image/jpeg",
        }
    }
}

impl AudioFormat {
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Mp3(_) => "mp3",
            Self::Aac(_) => "m4a",
        }
    }

    pub const fn mime_type(&self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Mp3(_) => "audio/mpeg",
            Self::Aac(_) => "audio/mp4",
        }
    }
}

#[derive(Debug)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration_ms: usize,
    pub artists: Vec<Artist>,
    pub cover_url: String,
}

#[derive(Debug, Clone)]
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
        filename_without_ext: &str,
        url: &str,
    ) -> Result<(AudioFormat, PathBuf), Error>;

    fn box_clone(&self) -> Box<dyn Module>;
}

#[derive(Clone, Default)]
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
        Self::default()
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
        filename_without_ext: &str,
        url: &str,
    ) -> Result<(AudioFormat, PathBuf), Error> {
        for module in &self.modules {
            if !module.url_supported(url) {
                continue;
            }
            return module.download(workdir, filename_without_ext, url).await;
        }
        Err(Error::NoAvailableModules)
    }

    pub async fn download_cover(
        &self,
        workdir: &Path,
        filename_without_ext: &str,
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
        let cover = workdir.join(format!("{filename_without_ext}.{}", format.extension()));
        let mut stream = response.bytes_stream();
        let mut file = File::create(&cover).await?;
        while let Some(chunk) = stream.try_next().await? {
            tokio::io::copy(&mut chunk.as_ref(), &mut file).await?;
        }
        Ok((format, cover))
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

    pub fn remux(
        &self,
        workdir: &Path,
        filename_without_ext: &str,
        audio: (AudioFormat, PathBuf),
        cover: &Path,
        metadata: Metadata,
    ) -> Result<(AudioFormat, PathBuf), Error> {
        let output_path = workdir.join(format!("{filename_without_ext}.{}", audio.0.extension()));
        let mut input_audio = format::input(&audio.1)?;
        let mut input_cover = format::input(&cover)?;
        let mut output = format::output(&output_path)?;

        let mut input_audio_stream = -1;
        for (i, stream) in input_audio.streams().enumerate() {
            if stream.parameters().medium() != media::Type::Audio {
                continue;
            }
            let mut output_stream = output.add_stream(encoder::find(codec::Id::None))?;
            output_stream.set_parameters(stream.parameters());
            unsafe {
                (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
            }
            input_audio_stream = i as i32;
            break;
        }

        let mut input_cover_stream = -1;
        for (i, stream) in input_cover.streams().enumerate() {
            if stream.parameters().medium() != media::Type::Video {
                continue;
            }
            let mut output_stream = output.add_stream(encoder::find(codec::Id::None))?;
            output_stream.set_parameters(stream.parameters());
            unsafe {
                (*output_stream.as_mut_ptr()).disposition |= AV_DISPOSITION_ATTACHED_PIC;
                (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
            }
            input_cover_stream = i as i32;
            break;
        }

        if input_audio_stream == -1 || input_cover_stream == -1 {
            return Err(Error::RemuxError(ffmpeg_next::Error::StreamNotFound));
        }

        let mut dict = Dictionary::new();
        dict.set("title", &metadata.title);
        dict.set("artist", &metadata.artist);
        if let Some(v) = metadata.album {
            dict.set("album", &v);
        }
        if let Some(v) = metadata.album_artist {
            dict.set("album_artist", &v);
        }
        if let Some(v) = metadata.composer {
            dict.set("composer", &v);
        }
        if let Some(v) = metadata.copyright {
            dict.set("copyright", &v);
        }
        if let Some(v) = metadata.creation_time {
            dict.set("creation_time", &v);
        }
        if let Some(v) = metadata.date {
            dict.set("date", &v);
        }
        if let Some(v) = metadata.disc {
            dict.set("disc", &v);
        }
        if let Some(v) = metadata.genre {
            dict.set("genre", &v);
        }
        if let Some(v) = metadata.language {
            dict.set("language", &v);
        }
        if let Some(v) = metadata.performer {
            dict.set("performer", &v);
        }
        if let Some(v) = metadata.publisher {
            dict.set("publisher", &v);
        }
        if let Some(v) = metadata.track {
            dict.set("track", &v);
        }
        output.set_metadata(dict);
        output.write_header()?;

        for (stream, mut packet) in input_audio.packets() {
            if stream.index() != input_audio_stream as usize {
                continue;
            }
            packet.rescale_ts(stream.time_base(), output.stream(0).unwrap().time_base());
            packet.set_stream(0);
            packet.set_position(-1);
            packet.write_interleaved(&mut output)?;
        }

        for (stream, mut packet) in input_cover.packets() {
            if stream.index() != input_cover_stream as usize {
                continue;
            }
            packet.rescale_ts(stream.time_base(), output.stream(1).unwrap().time_base());
            packet.set_stream(1);
            packet.set_position(-1);
            packet.write_interleaved(&mut output)?;
        }

        output.write_trailer()?;
        Ok((audio.0, output_path))
    }
}
