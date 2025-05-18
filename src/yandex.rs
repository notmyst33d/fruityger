use async_trait::async_trait;
use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, RequestBuilder, redirect::Policy};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

use crate::{
    AudioFormat, BytesStream, Module, SearchResults, const_headers,
    error::{Error, UrlError},
};

type HmacSha256 = Hmac<Sha256>;

const SIGN_KEY: &'static [u8] = b"kzqU4XhfCaY6B6JTHODeq5";

#[derive(Clone)]
pub struct Yandex {
    client: reqwest::Client,
    token: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    pub result: T,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    pub tracks: TracksData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFileInfoResponse {
    pub download_info: DownloadInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadInfo {
    pub codec: String,
    pub bitrate: u16,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct TracksData {
    pub results: Vec<Track>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    pub id: u64,
    pub title: String,
    pub duration_ms: usize,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub cover_uri: String,
}

#[derive(Debug, Deserialize)]
struct Album {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
struct Artist {
    pub id: u64,
    pub name: String,
}

impl Yandex {
    pub fn new(token: String) -> Self {
        Self {
            client: Client::builder()
                .redirect(Policy::none())
                .default_headers(const_headers!([
                    ("x-yandex-music-client", "YandexMusicDesktopAppWindows/5.18.2")
                ]))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) YandexMusic/5.18.2 Chrome/122.0.6261.156 Electron/29.4.6 Safari/537.36")
                .build()
                .unwrap(),
            token,
        }
    }

    pub fn builder(&self, method: Method, url: impl Into<String>) -> RequestBuilder {
        self.client
            .request(
                method,
                Url::parse("https://api.music.yandex.net")
                    .unwrap()
                    .join(&url.into())
                    .unwrap(),
            )
            .header("authorization", format!("OAuth {}", self.token))
    }
}

#[async_trait]
impl Module for Yandex {
    fn name(&self) -> &'static str {
        "yandex"
    }

    fn url_supported(&self, url: &str) -> bool {
        url.contains("music.yandex.ru")
    }

    fn supported_formats(&self) -> &'static [AudioFormat] {
        &[
            AudioFormat::Mp3(128),
            AudioFormat::Aac(256),
            AudioFormat::Flac,
        ]
    }

    async fn search(&self, query: &str, page: usize) -> Result<SearchResults, Error> {
        let text = self
            .builder(Method::GET, "/search")
            .query(&[
                ("text", query),
                ("type", "track"),
                ("page", &page.to_string()),
            ])
            .send()
            .await?
            .text()
            .await?;
        let value = serde_json::from_str::<Value>(&text)?;
        let data = serde_json::from_value::<ApiResponse<SearchResponse>>(value)?;
        log::debug!("/search -> {data:#?}");
        Ok(data.into())
    }

    async fn download(&self, url: &str) -> Result<(AudioFormat, BytesStream), Error> {
        let url = Url::parse(url)?;
        let mut it = url
            .path_segments()
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;
        let track_id = it
            .nth(3)
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;

        let ts = Utc::now().timestamp();
        let mut query = [
            ("ts", ts.to_string()),
            ("trackId", track_id.to_string()),
            ("quality", "lossless".to_string()),
            ("codecs", "flac,flac-mp4,aac,aac-mp4,mp3".to_string()),
            ("transports", "raw".to_string()),
            ("sign", "".to_string()),
        ];
        query[5].1 = {
            let mut h = HmacSha256::new_from_slice(SIGN_KEY).unwrap();
            h.update(
                format!(
                    "{}{}{}{}{}",
                    query[0].1,
                    query[1].1,
                    query[2].1,
                    query[3].1.replace(",", ""),
                    query[4].1.replace(",", "")
                )
                .as_bytes(),
            );
            BASE64_STANDARD_NO_PAD.encode(h.finalize().into_bytes())
        };

        let response = self
            .builder(Method::GET, format!("/get-file-info"))
            .query(&query)
            .send()
            .await?
            .json::<ApiResponse<GetFileInfoResponse>>()
            .await?;

        log::debug!("/get-file-info -> {response:#?}");

        let format = match response.result.download_info.codec.as_str() {
            "mp3" => AudioFormat::Mp3(response.result.download_info.bitrate),
            "aac-mp4" => AudioFormat::Aac(response.result.download_info.bitrate),
            "flac-mp4" => AudioFormat::Flac,
            _ => return Err(Error::UnsupportedCodecError),
        };

        let mut response = self
            .client
            .get(response.result.download_info.url)
            .send()
            .await?;

        let (tx, rx) = mpsc::channel(16);
        tokio::task::spawn(async move {
            while let Ok(chunk) = response.chunk().await {
                if let Some(chunk) = chunk {
                    if let Err(_) = tx.send(Ok(chunk)).await {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        Ok((format, Box::pin(ReceiverStream::new(rx))))
    }

    fn box_clone(&self) -> Box<dyn Module> {
        Box::new((*self).clone())
    }
}

impl Into<SearchResults> for ApiResponse<SearchResponse> {
    fn into(self) -> SearchResults {
        SearchResults {
            tracks: self
                .result
                .tracks
                .results
                .into_iter()
                .map(Track::into)
                .collect(),
        }
    }
}

impl Into<crate::Track> for Track {
    fn into(self) -> crate::Track {
        crate::Track {
            id: self.id.to_string(),
            url: format!(
                "https://music.yandex.ru/album/{}/track/{}",
                self.albums[0].id, self.id
            ),
            title: self.title,
            duration_ms: self.duration_ms,
            artists: self.artists.into_iter().map(Artist::into).collect(),
            cover_url: format!("https://{}", self.cover_uri.replace("%%", "orig")),
        }
    }
}

impl Into<crate::Artist> for Artist {
    fn into(self) -> crate::Artist {
        crate::Artist {
            id: self.id.to_string(),
            name: self.name,
        }
    }
}

#[cfg(test)]
mod test {
    use bytes::BytesMut;
    use futures::StreamExt;

    use crate::{Module, yandex::Yandex};

    #[tokio::test]
    async fn all() {
        simple_logger::init().unwrap();
        let query = std::env::var("YANDEX_QUERY").unwrap_or("periphery scarlet".to_string());
        let client = Yandex::new(
            std::env::var("YANDEX_TOKEN").expect("token is required to test this module"),
        );
        let results = client.search(&query, 0).await.unwrap();
        let track = &results.tracks[0];
        let (format, mut stream) = client.download(&track.url).await.unwrap();
        log::info!(
            "downloading {} - {} with format {format:?}",
            track.artists[0].name,
            track.title
        );
        let mut data = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            data.extend_from_slice(&chunk.unwrap());
        }
    }
}
