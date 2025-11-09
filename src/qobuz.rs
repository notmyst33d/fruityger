// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

use chrono::Utc;
use md5::{Digest, Md5};
use reqwest::{Client, Method, RequestBuilder, redirect::Policy};
use url::Url;

use crate::{AudioFormat, AudioStream, Error, ErrorKind, SearchResults, const_headers};

#[derive(Clone)]
pub struct Qobuz {
    client: reqwest::Client,
    config: Config,
}

#[derive(Clone)]
pub struct Config {
    token: String,
    app_id: String,
    app_secret: String,
}

impl Qobuz {
    pub fn new(config: Config) -> Self {
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
            config,
        }
    }

    fn builder<S: AsRef<str>>(&self, method: Method, url: S) -> RequestBuilder {
        self.client
            .request(
                method,
                Url::parse("http://www.qobuz.com")
                    .and_then(|u| u.join(&format!("/api.json/0.2/{}", url.as_ref())))
                    .unwrap(),
            )
            .header("x-user-auth-token", &self.config.token)
            .query(&[("app_id", &self.config.app_id)])
    }

    pub async fn search(&self, query: &str, page: usize) -> Result<SearchResults, Error> {
        let response = self
            .builder(Method::GET, "/catalog/search")
            .query(&[
                ("query", query),
                ("limit", "20"),
                ("offset", &(page * 20).to_string()),
            ])
            .send()
            .await?;
        response
            .json::<data::ApiResponse<data::SearchResponse>>()
            .await?
            .into()
    }

    pub async fn get_stream(&self, url: &str) -> Result<AudioStream, Error> {
        let url = Url::parse(url)?;
        let mut it = url
            .path_segments()
            .ok_or(Error::from(ErrorKind::InvalidUrlError))?;
        let track_id = it.nth(1).ok_or(Error::from(ErrorKind::InvalidUrlError))?;
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
                query[0].1, query[1].1, query[2].1, query[3].1, ts, self.config.app_secret
            ));
            format!("{:x}", h.finalize())
        };
        let response = match self
            .builder(Method::GET, "/track/getFileUrl")
            .query(&query)
            .send()
            .await?
            .json::<data::ApiResponse<data::GetFileUrlResponse>>()
            .await?
        {
            data::ApiResponse::Ok(v) => v,
            data::ApiResponse::Err { message, .. } => {
                return Err(Error::new(ErrorKind::ServiceError, &message));
            }
        };

        if response.sample {
            return Err(Error::new(ErrorKind::ServiceError, "cannot get full song"));
        }

        let format = match response.mime_type.as_str() {
            "audio/flac" => AudioFormat::Flac,
            _ => return Err(Error::new(ErrorKind::UnsupportedCodecError, "")),
        };

        Ok(AudioStream {
            response: self.client.get(response.url).send().await?,
            format,
        })
    }
}

mod data {
    use crate::{
        SearchResults,
        error::{Error, ErrorKind},
    };
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    pub enum ApiResponse<T> {
        Ok(T),
        Err { message: String },
    }

    #[derive(Debug, Deserialize)]
    pub struct SearchResponse {
        pub tracks: Results<Track>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Results<T> {
        pub items: Vec<T>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Track {
        pub id: u64,
        pub title: String,
        pub duration: usize,
        pub performer: Performer,
        pub album: Album,
    }

    #[derive(Debug, Deserialize)]
    pub struct Performer {
        pub id: u64,
        pub name: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct Album {
        pub image: Image,
    }

    #[derive(Debug, Deserialize)]
    pub struct Image {
        pub large: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct GetFileUrlResponse {
        pub url: String,
        pub mime_type: String,
        pub sample: bool,
    }

    impl From<ApiResponse<SearchResponse>> for Result<SearchResults, Error> {
        fn from(value: ApiResponse<SearchResponse>) -> Self {
            match value {
                ApiResponse::Ok(v) => Ok(v.into()),
                ApiResponse::Err { message, .. } => {
                    Err(Error::new(ErrorKind::ServiceError, &message))
                }
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
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use crate::{
        qobuz::{Config, Qobuz},
        save_audio_stream,
    };

    #[tokio::test]
    async fn all() {
        let query = std::env::var("QOBUZ_QUERY").unwrap_or("periphery scarlet".to_string());
        let client = Qobuz::new(Config {
            token: std::env::var("QOBUZ_TOKEN")
                .expect("QOBUZ_TOKEN is required to test this module"),
            app_id: std::env::var("QOBUZ_APP_ID")
                .expect("QOBUZ_APP_ID is required to test this module"),
            app_secret: std::env::var("QOBUZ_APP_SECRET")
                .expect("QOBUZ_APP_SECRET is required to test this module"),
        });
        let results = client.search(&query, 0).await.unwrap();
        let stream = client.get_stream(&results.tracks[0].url).await.unwrap();
        save_audio_stream(stream, Path::new("/tmp"), "qobuz_test")
            .await
            .unwrap();
    }
}
