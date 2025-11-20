// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

use crate::{AudioFormat, AudioStream, Error, SearchResults};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

#[derive(Clone)]
pub struct Hifi {
    client: reqwest::Client,
    config: Config,
}

#[derive(Clone, Deserialize)]
pub struct Host {
    base_url: String,
}

#[derive(Clone, Deserialize)]
pub struct Config(pub Vec<Host>);

impl Hifi {
    pub fn new(config: Config) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    fn builder<S: AsRef<str>>(
        &self,
        url: &str,
        method: Method,
        path: S,
    ) -> Result<RequestBuilder, Error> {
        Ok(self
            .client
            .request(method, Url::parse(url).and_then(|u| u.join(path.as_ref()))?))
    }

    pub async fn try_send(
        &self,
        build_request: impl Fn(&str) -> Result<RequestBuilder, Error>,
    ) -> Result<Response, Error> {
        for host in &self.config.0 {
            if let Ok(response) = build_request(&host.base_url)?.send().await
                && response.status() == StatusCode::OK
            {
                return Ok(response);
            }
        }
        Err(Error::ServiceError("cannot find usable server".to_owned()))
    }

    pub async fn search(&self, query: &str, _page: usize) -> Result<SearchResults, Error> {
        let response = self
            .try_send(|url| {
                Ok(self
                    .builder(url, Method::GET, "/search/")?
                    .query(&[("s", query)]))
            })
            .await?;
        Ok(response.json::<data::SearchResponse>().await?.into())
    }

    pub async fn get_stream(&self, id: &str) -> Result<AudioStream, Error> {
        let response = self
            .try_send(|url| {
                Ok(self
                    .builder(url, Method::GET, "/track/")?
                    .query(&[("id", id), ("quality", "LOSSLESS")]))
            })
            .await?
            .json::<Vec<Value>>()
            .await?;

        let Some(track_response) = response
            .get(2)
            .and_then(|v| serde_json::from_value::<data::TrackResponse>(v.clone()).ok())
        else {
            return Err(Error::ServiceError(
                "service did not return valid json".to_owned(),
            ));
        };

        Ok(AudioStream {
            response: self
                .client
                .get(track_response.original_track_url)
                .send()
                .await?,
            format: AudioFormat::Flac,
        })
    }
}

mod data {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct SearchResponse {
        pub items: Vec<Track>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Track {
        pub id: u64,
        pub title: String,
        pub url: String,
        pub duration: usize,
        pub artist: Artist,
        pub album: Album,
    }

    #[derive(Debug, Deserialize)]
    pub struct Artist {
        pub id: u64,
        pub name: String,
    }

    #[derive(Debug, Deserialize)]
    pub struct Album {
        pub id: u64,
        pub title: String,
        pub cover: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    pub struct TrackResponse {
        pub original_track_url: String,
    }

    impl From<SearchResponse> for crate::SearchResults {
        fn from(value: SearchResponse) -> Self {
            Self {
                tracks: value.items.into_iter().map(crate::Track::from).collect(),
            }
        }
    }

    impl From<Track> for crate::Track {
        fn from(value: Track) -> Self {
            Self {
                id: value.id.to_string(),
                url: value.url,
                title: value.title,
                duration_ms: value.duration * 1000,
                artists: vec![crate::Artist::from(value.artist)],
                cover_url: format!(
                    "https://resources.tidal.com/images/{}/750x750.jpg",
                    value.album.cover.replace("-", "/")
                ),
            }
        }
    }

    impl From<Artist> for crate::Artist {
        fn from(value: Artist) -> Self {
            Self {
                id: value.id.to_string(),
                name: value.name,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{hifi::Hifi, save_audio_stream, save_cover};
    use std::path::Path;
    use tokio::fs;

    #[tokio::test]
    async fn all() {
        let query = std::env::var("FRUITYGER_HIFI_QUERY").unwrap_or("periphery scarlet".to_owned());
        let client = Hifi::new(
            serde_json::from_slice(
                &fs::read(std::env::var("FRUITYGER_HIFI_CONFIG").unwrap_or("config.json".to_owned()))
                    .await
                    .unwrap(),
            )
            .unwrap(),
        );
        let results = client.search(&query, 0).await.unwrap();
        let track = &results.tracks[0];
        let stream = client.get_stream(&track.id).await.unwrap();
        let _ = save_audio_stream(stream, Path::new("/tmp"), "hifi_test")
            .await
            .unwrap();
        let _ = save_cover(
            reqwest::get(&track.cover_url).await.unwrap(),
            Path::new("/tmp"),
            "cover",
        )
        .await
        .unwrap();
    }
}
