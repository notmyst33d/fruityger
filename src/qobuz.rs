// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use futures::TryStreamExt;
use md5::{Digest, Md5};
use reqwest::{Client, Method, RequestBuilder, redirect::Policy};
use serde::Deserialize;
use tokio::fs::File;
use url::Url;

use crate::{
    AudioFormat, Module, SearchResults, const_headers,
    error::{Error, UrlError},
};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiResponse<T> {
    Ok(T),
    Err { message: String },
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    tracks: Results<Track>,
}

#[derive(Debug, Deserialize)]
struct Results<T> {
    items: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct Track {
    id: u64,
    title: String,
    duration: usize,
    performer: Performer,
    album: Album,
}

#[derive(Debug, Deserialize)]
struct Performer {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct Album {
    image: Image,
}

#[derive(Debug, Deserialize)]
struct Image {
    large: String,
}

#[derive(Debug, Deserialize)]
struct GetFileUrlResponse {
    url: String,
    mime_type: String,
}

#[derive(Clone)]
pub struct Qobuz {
    client: reqwest::Client,
    token: String,
    app_id: String,
    app_secret: String,
}

impl Qobuz {
    pub fn new(token: String, app_id: String, app_secret: String) -> Self {
        Self {
            client: Client::builder()
                .redirect(Policy::none())
                .default_headers(const_headers!([
                    ("x-device-platform", "android"),
                    ("x-device-model", "Pixel 3"),
                    ("x-device-os-version", "10"),
                    ("x-device-manufacturer-id", "ffffffff-5783-1f51-ffff-ffffef05ac4a"),
                    ("x-app-version", "5.16.1.5"),
                ]))
                .user_agent("Dalvik/2.1.0 (Linux; U; Android 10; Pixel 3 Build/QP1A.190711.020)) QobuzMobileAndroid/5.16.1.5-b21041415")
                .build()
                .unwrap(),
            token,
            app_id,
            app_secret,
        }
    }

    fn builder(&self, method: Method, url: impl Into<String>) -> RequestBuilder {
        self.client
            .request(
                method,
                Url::parse("http://www.qobuz.com")
                    .and_then(|u| u.join(&format!("/api.json/0.2/{}", url.into())))
                    .unwrap(),
            )
            .header("x-user-auth-token", &self.token)
            .query(&[("app_id", &self.app_id)])
    }
}

#[async_trait]
impl Module for Qobuz {
    fn name(&self) -> &'static str {
        "qobuz"
    }

    fn url_supported(&self, url: &str) -> bool {
        url.contains("open.qobuz.com")
    }

    fn supported_formats(&self) -> &'static [AudioFormat] {
        &[AudioFormat::Flac]
    }

    async fn search(&self, query: &str, page: usize) -> Result<SearchResults, Error> {
        let response = self
            .builder(Method::GET, "/catalog/search")
            .query(&[
                ("query", query),
                ("limit", "20"),
                ("offset", &(page * 20).to_string()),
            ])
            .send()
            .await?;
        response.json::<ApiResponse<SearchResponse>>().await?.into()
    }

    async fn download(
        &self,
        workdir: &Path,
        filename_without_ext: &str,
        url: &str,
    ) -> Result<(AudioFormat, PathBuf), Error> {
        let url = Url::parse(url)?;
        let mut it = url
            .path_segments()
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;
        let track_id = it
            .nth(1)
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;
        let ts = Utc::now().timestamp();
        let mut query = [
            ("format_id", "6".to_string()),
            ("intent", "stream".to_string()),
            ("sample", "false".to_string()),
            ("track_id", track_id.to_string()),
            ("request_ts", ts.to_string()),
            ("request_sig", String::new()),
        ];
        query[5].1 = {
            let mut h = Md5::new();
            h.update(format!(
                "trackgetFileUrlformat_id{}intent{}sample{}track_id{}{}{}",
                query[0].1, query[1].1, query[2].1, query[3].1, ts, self.app_secret
            ));
            format!("{:x}", h.finalize())
        };
        let response = match self
            .builder(Method::GET, "/track/getFileUrl")
            .query(&query)
            .send()
            .await?
            .json::<ApiResponse<GetFileUrlResponse>>()
            .await?
        {
            ApiResponse::Ok(v) => v,
            ApiResponse::Err { message, .. } => return Err(Error::ServiceError(message)),
        };

        let format = match response.mime_type.as_str() {
            "audio/flac" => AudioFormat::Flac,
            _ => return Err(Error::UnsupportedCodecError),
        };

        let mut stream = self.client.get(response.url).send().await?.bytes_stream();

        let out = workdir.join(format!("{filename_without_ext}.{}", format.extension()));
        let mut file = File::create(&out).await?;
        while let Some(chunk) = stream.try_next().await? {
            tokio::io::copy(&mut chunk.as_ref(), &mut file).await?;
        }
        Ok((format, out))
    }

    fn box_clone(&self) -> Box<dyn Module> {
        Box::new((*self).clone())
    }
}

impl From<ApiResponse<SearchResponse>> for Result<SearchResults, Error> {
    fn from(value: ApiResponse<SearchResponse>) -> Self {
        match value {
            ApiResponse::Ok(v) => Ok(v.into()),
            ApiResponse::Err { message, .. } => Err(Error::ServiceError(message)),
        }
    }
}

impl From<SearchResponse> for crate::SearchResults {
    fn from(value: SearchResponse) -> Self {
        Self {
            tracks: value.tracks.items.into_iter().map(Track::into).collect(),
        }
    }
}

impl From<Track> for crate::Track {
    fn from(value: Track) -> Self {
        Self {
            id: value.id.to_string(),
            url: format!("https://open.qobuz.com/track/{}", value.id),
            title: value.title,
            duration_ms: value.duration * 1000,
            artists: vec![crate::Artist {
                id: value.performer.id.to_string(),
                name: value.performer.name,
            }],
            cover_url: value.album.image.large,
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use crate::{Module, qobuz::Qobuz};

    #[tokio::test]
    async fn all() {
        let query = std::env::var("QOBUZ_QUERY").unwrap_or("periphery scarlet".to_string());
        let client = Qobuz::new(
            std::env::var("QOBUZ_TOKEN").expect("QOBUZ_TOKEN is required to test this module"),
            std::env::var("QOBUZ_APP_ID").expect("QOBUZ_APP_ID is required to test this module"),
            std::env::var("QOBUZ_APP_SECRET")
                .expect("QOBUZ_APP_SECRET is required to test this module"),
        );
        let results = client.search(&query, 0).await.unwrap();
        let _ = client
            .download(Path::new("."), "qobuz_audio", &results.tracks[0].url)
            .await
            .unwrap();
    }
}
