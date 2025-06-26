// SPDX-License-Identifier: MIT
// Copyright 2025 Myst33d

use ffmpeg_next::{Dictionary, codec, encoder, ffi::AV_DISPOSITION_ATTACHED_PIC, format, media};
use serde::Deserialize;
use std::path::Path;

#[derive(Default, Deserialize)]
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

pub(crate) fn remux(
    input_audio: impl AsRef<Path>,
    input_cover: impl AsRef<Path>,
    output: impl AsRef<Path>,
    metadata: Metadata,
) -> Result<(), ffmpeg_next::Error> {
    let mut input_audio = format::input(&input_audio)?;
    let mut input_cover = format::input(&input_cover)?;
    let mut output = format::output(&output)?;

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
        return Err(ffmpeg_next::Error::StreamNotFound);
    }

    let mut dict = Dictionary::new();
    dict.set("title", &metadata.title);
    dict.set("artist", &metadata.artist);
    metadata.album.map(|v| dict.set("album", &v));
    metadata.album_artist.map(|v| dict.set("album_artist", &v));
    metadata.composer.map(|v| dict.set("composer", &v));
    metadata.copyright.map(|v| dict.set("copyright", &v));
    metadata
        .creation_time
        .map(|v| dict.set("creation_time", &v));
    metadata.date.map(|v| dict.set("date", &v));
    metadata.disc.map(|v| dict.set("disc", &v));
    metadata.genre.map(|v| dict.set("genre", &v));
    metadata.language.map(|v| dict.set("language", &v));
    metadata.performer.map(|v| dict.set("performer", &v));
    metadata.publisher.map(|v| dict.set("publisher", &v));
    metadata.track.map(|v| dict.set("track", &v));
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

    Ok(())
}
