use futures::StreamExt;
use rand::Rng;
use reqwest::header;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    mem::{ManuallyDrop, forget},
    str::FromStr,
};
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc,
};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    AudioFormat, BytesStream, Client, CoverFormat, error::Error, qobuz::Qobuz, remux::Metadata,
    yandex::Yandex,
};

/// # Context
/// The context of the FFI client
///
/// ## Thread Safety
/// This struct is not thread safe
pub struct Context {
    rt: Runtime,
    client: Client,
    track_downloads: HashMap<u64, (AudioFormat, BytesStream)>,
    cover_downloads: HashMap<u64, (CoverFormat, BytesStream)>,
}

#[derive(Deserialize)]
pub enum FfiRequest {
    AddModule {
        name: String,
        params: HashMap<String, String>,
    },
    Search {
        service: String,
        query: String,
        page: usize,
    },
    DownloadTrack {
        url: String,
    },
    DownloadCover {
        url: String,
    },
    Remux {
        track_download_id: u64,
        cover_download_id: u64,
        metadata: Metadata,
    },
}

#[derive(Serialize)]
pub struct DownloadTrackResponse {
    id: u64,
    format: AudioFormat,
}

#[derive(Serialize)]
pub struct DownloadCoverResponse {
    id: u64,
    format: CoverFormat,
}

#[derive(Serialize)]
pub enum FfiResponse<T> {
    Ok(T),
    Err(String),
}

#[derive(Serialize)]
struct Json<T>(T);

impl<T: Serialize> IntoJson for Json<T> {
    fn serialize(&self) -> String {
        serde_json::to_string(&self.0).unwrap()
    }
}

trait IntoJson {
    fn serialize(&self) -> String;
}

#[repr(C)]
pub enum ErrorCode {
    Success,
    NotFound,
    IteratorEnd,
}

/// ## Create new context
///
/// ## Safety
/// Return value must be freed using [free_context]
#[unsafe(no_mangle)]
pub extern "C" fn create_context() -> *mut Context {
    Box::into_raw(Box::new(Context {
        rt: Builder::new_current_thread().enable_all().build().unwrap(),
        client: Client::new(),
        track_downloads: HashMap::new(),
        cover_downloads: HashMap::new(),
    }))
}

/// ## Send JSON request via FFI
///
/// ## Safety
/// Return value must be freed using [free_cstring]
///
/// ## Thread Safety
/// This method is not thread safe, use [clone_context] if you want to use the same client on multiple threads
#[unsafe(no_mangle)]
pub extern "C" fn send_json(ctx: &mut Context, request: *const c_char) -> *const c_char {
    let request = match serde_json::from_str(unsafe { CStr::from_ptr(request).to_str().unwrap() }) {
        Ok(v) => v,
        Err(e) => {
            return CString::from_str(&IntoJson::serialize(&Json(FfiResponse::<()>::Err(
                e.to_string(),
            ))))
            .unwrap()
            .into_raw();
        }
    };
    let response: Box<dyn IntoJson> = match request {
        FfiRequest::AddModule { name, params } => Box::new(add_module(ctx, name, params)),
        FfiRequest::Search {
            service,
            query,
            page,
        } => Box::new(search(ctx, service, query, page)),
        FfiRequest::DownloadTrack { url } => Box::new(download_track(ctx, url)),
        FfiRequest::DownloadCover { url } => Box::new(download_cover(ctx, url)),
        FfiRequest::Remux {
            track_download_id,
            cover_download_id,
            metadata,
        } => Box::new(remux(ctx, track_download_id, cover_download_id, metadata)),
    };
    CString::from_str(&response.serialize()).unwrap().into_raw()
}

fn add_module(ctx: &mut Context, name: String, params: HashMap<String, String>) -> impl IntoJson {
    match name.as_str() {
        "yandex" => {
            let Some(token) = params.get("token") else {
                return Json(FfiResponse::Err("token not provided".to_string()));
            };
            ctx.client.add_module(Yandex::new(token.to_string()));
        }
        "qobuz" => {
            let Some(token) = params.get("token") else {
                return Json(FfiResponse::Err("token not provided".to_string()));
            };
            let Some(app_id) = params.get("app_id") else {
                return Json(FfiResponse::Err("app_id not provided".to_string()));
            };
            let Some(app_secret) = params.get("app_secret") else {
                return Json(FfiResponse::Err("app_secret not provided".to_string()));
            };
            ctx.client.add_module(Qobuz::new(
                token.to_string(),
                app_id.to_string(),
                app_secret.to_string(),
            ));
        }
        _ => return Json(FfiResponse::Err("module not available".to_string())),
    }
    Json(FfiResponse::Ok(()))
}

fn search(ctx: &Context, service: String, query: String, page: usize) -> impl IntoJson {
    ctx.rt.block_on(async {
        match ctx.client.search(&service, &query, page).await {
            Ok(v) => Json(FfiResponse::Ok(v)),
            Err(e) => Json(FfiResponse::Err(format!("{e:?}"))),
        }
    })
}

fn download_track(ctx: &mut Context, url: String) -> impl IntoJson {
    ctx.rt.block_on(async {
        match ctx.client.download(&url).await {
            Ok(v) => {
                let id = rand::rng().random::<u64>();
                let format = v.0.clone();
                ctx.track_downloads.insert(id, v);
                Json(FfiResponse::Ok(DownloadTrackResponse { id, format }))
            }
            Err(e) => Json(FfiResponse::Err(e.to_string())),
        }
    })
}

fn download_cover(ctx: &mut Context, url: String) -> impl IntoJson {
    ctx.rt.block_on(async {
        let Ok(mut response) = reqwest::Client::new().get(url).send().await else {
            return Json(FfiResponse::Err("download error".to_string()));
        };
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg");
        let format = match content_type {
            "image/jpeg" => CoverFormat::Jpg,
            "image/png" => CoverFormat::Png,
            _ => return Json(FfiResponse::Err("unsupported content type".to_string())),
        };
        let id = rand::rng().random();
        let (tx, rx) = mpsc::channel(16);
        let stream = ReceiverStream::new(rx);
        ctx.cover_downloads
            .insert(id, (format.clone(), Box::pin(stream)));
        tokio::task::spawn(async move {
            loop {
                match response.chunk().await {
                    Ok(v) => {
                        if let Some(v) = v {
                            let _ = tx.send(Ok(v)).await;
                        } else {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(Error::ConnectionError(e))).await;
                        return;
                    }
                };
            }
        });
        Json(FfiResponse::Ok(DownloadCoverResponse { id, format }))
    })
}

fn remux(
    ctx: &mut Context,
    track_download_id: u64,
    cover_download_id: u64,
    metadata: Metadata,
) -> impl IntoJson {
    ctx.rt.block_on(async {
        let Some(track) = ctx.track_downloads.remove(&track_download_id) else {
            return Json(FfiResponse::Err("not found".to_string()));
        };
        let Some(cover) = ctx.cover_downloads.remove(&cover_download_id) else {
            return Json(FfiResponse::Err("not found".to_string()));
        };
        let remuxed_track = match ctx.client.remux(track, cover, metadata).await {
            Ok(v) => v,
            Err(e) => return Json(FfiResponse::Err(format!("{e:?}"))),
        };
        let id = rand::rng().random();
        let format = remuxed_track.0.clone();
        ctx.track_downloads.insert(id, remuxed_track);
        Json(FfiResponse::Ok(DownloadTrackResponse { id, format }))
    })
}

macro_rules! poll_impl {
    ($fnname:ident, $field:ident) => {
        /// ## Safety
        /// `out` must be freed using [free_vec]
        ///
        /// ## Thread Safety
        /// This method is not thread safe, you should poll for data on the same thread
        #[unsafe(no_mangle)]
        pub extern "C" fn $fnname(
            ctx: &mut Context,
            id: u64,
            out: &mut *mut c_char,
            out_len: &mut usize,
            out_cap: &mut usize,
        ) -> ErrorCode {
            let Some(stream) = ctx.$field.get_mut(&id) else {
                return ErrorCode::NotFound;
            };
            let chunk = ctx.rt.block_on(stream.1.next());
            if let Some(chunk) = chunk {
                let Ok(chunk) = chunk else {
                    return ErrorCode::IteratorEnd;
                };
                let chunk = chunk.to_vec();
                let mut chunk = ManuallyDrop::new(chunk);
                let (ptr, len, cap) = (chunk.as_mut_ptr(), chunk.len(), chunk.capacity());
                forget(chunk);
                *out = ptr as *mut i8;
                *out_len = len;
                *out_cap = cap
            } else {
                return ErrorCode::IteratorEnd;
            }
            ErrorCode::Success
        }
    };
}

poll_impl!(poll_download_track, track_downloads);
poll_impl!(poll_download_cover, cover_downloads);

#[unsafe(no_mangle)]
pub extern "C" fn free_cstring(s: *const c_char) {
    unsafe { drop(CString::from_raw(s as *mut c_char)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn free_vec(v: *mut c_char, len: usize, cap: usize) {
    unsafe { drop(Vec::from_raw_parts(v, len, cap)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn free_context(p: *mut Context) {
    unsafe { drop(Box::from_raw(p)) }
}

/// ## Clone specified context
///
/// ## Safety
/// Return value must be freed using [free_context]
///
/// ## Notes
/// This does not clone the current download state
#[unsafe(no_mangle)]
pub extern "C" fn clone_context(p: &mut Context) -> *mut Context {
    Box::into_raw(Box::new(Context {
        rt: Builder::new_current_thread().enable_all().build().unwrap(),
        client: (*p).client.clone(),
        track_downloads: HashMap::new(),
        cover_downloads: HashMap::new(),
    }))
}
