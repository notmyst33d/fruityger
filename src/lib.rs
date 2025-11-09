// SPDX-License-Identifier: MIT
// Copyright (C) 2025 Myst33d <myst33d@gmail.com>

pub mod error;
pub mod format;
pub mod hifi;
pub mod qobuz;
pub mod yandex;

use std::path::{Path, PathBuf};

use ffmpeg_next::{
    Dictionary, codec, encoder,
    ffi::AV_DISPOSITION_ATTACHED_PIC,
    format::context::{Input, Output},
    media,
};
use futures::TryStreamExt;
use reqwest::{Response, header};
use tokio::fs::File;

use crate::{
    error::{Error, ErrorKind},
    format::{AudioFormat, CoverFormat, Format},
};

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

macro_rules! metadata_option {
    (($metadata:ident, $dict:ident) $(, $key:ident)*) => {
        $(
            if let Some(v) = $metadata.$key {
                $dict.set(stringify!($key), &v);
            }
        )*
    };
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

#[derive(Debug)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
}

#[derive(Debug)]
pub struct Track {
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration_ms: usize,
    pub artists: Vec<Artist>,
    pub cover_url: String,
}

#[derive(Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

pub struct AudioStream {
    pub response: Response,
    pub format: AudioFormat,
}

pub fn remux(
    dir: &Path,
    audio_path: &Path,
    cover_path: Option<&Path>,
    target_audio_format: AudioFormat,
    target_filename: &str,
    metadata: Metadata,
) -> Result<PathBuf, Error> {
    let input_audio = ffmpeg_next::format::input(&audio_path)?;
    let input_cover = cover_path.and_then(|c| ffmpeg_next::format::input(&c).ok());

    let output_path = dir.join(format!(
        "{}.{}",
        target_filename,
        target_audio_format.extension()
    ));
    let mut output = ffmpeg_next::format::output(&output_path)?;

    let map_first_stream = |input: &Input, output: &mut Output, media_type: media::Type| {
        for stream in input.streams() {
            if stream.parameters().medium() != media_type {
                continue;
            }
            let mut output_stream = output.add_stream(encoder::find(codec::Id::None))?;
            output_stream.set_parameters(stream.parameters());
            unsafe {
                if media_type == media::Type::Video {
                    (*output_stream.as_mut_ptr()).disposition |= AV_DISPOSITION_ATTACHED_PIC;
                }
                (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
            }
            return Ok((stream.index(), output_stream.index()));
        }
        Err(Error::new(ErrorKind::RemuxError, "cannot find stream"))
    };

    let (input_audio_index, output_audio_index) =
        map_first_stream(&input_audio, &mut output, media::Type::Audio)?;
    let mut sources = vec![(input_audio, input_audio_index, output_audio_index)];
    if let Some(input_cover) = input_cover {
        let (input_cover_index, output_cover_index) =
            map_first_stream(&input_cover, &mut output, media::Type::Video)?;
        sources.push((input_cover, input_cover_index, output_cover_index));
    }

    let mut dict = Dictionary::new();
    dict.set("title", &metadata.title);
    dict.set("artist", &metadata.artist);
    metadata_option!(
        (metadata, dict),
        album,
        album_artist,
        composer,
        copyright,
        creation_time,
        date,
        disc,
        genre,
        language,
        performer,
        publisher,
        track
    );
    output.set_metadata(dict);
    output.write_header()?;

    for (mut input, input_index, output_index) in sources {
        for (stream, mut packet) in input.packets() {
            if stream.index() != input_index {
                continue;
            }
            packet.rescale_ts(
                stream.time_base(),
                output.stream(output_index).unwrap().time_base(),
            );
            packet.set_stream(output_index);
            packet.set_position(-1);
            packet.write_interleaved(&mut output)?;
        }
    }

    output.write_trailer()?;
    Ok(output_path)
}

pub async fn save_cover(response: Response, dir: &Path, filename: &str) -> Result<PathBuf, Error> {
    let format = CoverFormat::try_from(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg"),
    )?;
    let path = dir.join(format!("{}.{}", filename, format.extension()));
    save(response, &path).await?;
    Ok(path)
}

pub async fn save_audio_stream(
    audio_stream: AudioStream,
    dir: &Path,
    filename: &str,
) -> Result<PathBuf, Error> {
    let path = dir.join(format!("{}.{}", filename, audio_stream.format.extension()));
    save(audio_stream.response, &path).await?;
    Ok(path)
}

async fn save(response: Response, path: &Path) -> Result<(), Error> {
    let mut stream = response.bytes_stream();
    let mut file = File::create(path).await?;
    while let Some(chunk) = stream.try_next().await? {
        tokio::io::copy(&mut chunk.as_ref(), &mut file).await?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::{Metadata, format::AudioFormat, remux};
    use std::path::Path;

    #[tokio::test]
    async fn remux_test() {
        remux(
            Path::new("/tmp"),
            Path::new(&std::env::var("AUDIO_PATH").unwrap_or("/tmp/audio.flac".to_owned())),
            Some(Path::new(
                &std::env::var("AUDIO_PATH").unwrap_or("/tmp/cover.jpg".to_owned()),
            )),
            AudioFormat::Flac,
            "remux_test",
            Metadata {
                title: "remux test".to_owned(),
                artist: "fruityger".to_owned(),
                ..Default::default()
            },
        )
        .unwrap();
    }
}
