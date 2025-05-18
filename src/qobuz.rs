use async_trait::async_trait;
use chrono::Utc;
use md5::{Digest, Md5};
use reqwest::{Client, Method, RequestBuilder, redirect::Policy};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

use crate::{
    AudioFormat, BytesStream, Module, SearchResults, const_headers,
    error::{Error, UrlError},
};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ApiResponse<T> {
    Ok(T),
    Err { status: String, message: String },
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Album {
    id: String,
    title: String,
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

    pub fn builder(&self, method: Method, url: impl Into<String>) -> RequestBuilder {
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

    async fn download(&self, url: &str) -> Result<(AudioFormat, BytesStream), Error> {
        let url = Url::parse(url)?;
        let mut it = url
            .path_segments()
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;
        let track_id = it
            .nth(1)
            .ok_or(Error::UrlError(UrlError::InvalidPathError))?;
        let ts = Utc::now().timestamp();
        let mut query = [
            ("format_id", "27".to_string()),
            ("intent", "stream".to_string()),
            ("sample", "false".to_string()),
            ("track_id", track_id.to_string()),
            ("request_ts", ts.to_string()),
            ("request_sig", "".to_string()),
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

        let mut response = self.client.get(response.url).send().await?;

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

impl Into<Result<SearchResults, Error>> for ApiResponse<SearchResponse> {
    fn into(self) -> Result<SearchResults, Error> {
        match self {
            ApiResponse::Ok(v) => Ok(v.into()),
            ApiResponse::Err { message, .. } => Err(Error::ServiceError(message)),
        }
    }
}

impl Into<crate::SearchResults> for SearchResponse {
    fn into(self) -> crate::SearchResults {
        crate::SearchResults {
            tracks: self.tracks.items.into_iter().map(Track::into).collect(),
        }
    }
}

impl Into<crate::Track> for Track {
    fn into(self) -> crate::Track {
        crate::Track {
            id: self.id.to_string(),
            url: format!("https://open.qobuz.com/track/{}", self.id),
            title: self.title,
            duration_ms: self.duration * 1000,
            artists: vec![crate::Artist {
                id: self.performer.id.to_string(),
                name: self.performer.name,
            }],
            cover_url: self.album.image.large,
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{Module, qobuz::Qobuz};

    #[tokio::test]
    async fn all() {
        simple_logger::init().unwrap();
        let query = std::env::var("QOBUZ_QUERY").unwrap_or("periphery scarlet".to_string());
        let client = Qobuz::new(
            std::env::var("QOBUZ_TOKEN").expect("token is required to test this module"),
            std::env::var("QOBUZ_APP_ID").expect("app_id is required to test this module"),
            std::env::var("QOBUZ_APP_SECRET").expect("app_secret is required to test this module"),
        );
        let results = client.search(&query, 0).await.unwrap();
        let _ = client.download(&results.tracks[0].url).await.unwrap();
    }
}
