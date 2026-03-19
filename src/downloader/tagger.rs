use std::io::Read as _;
use std::path::Path;

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{Accessor, Tag, TagType, ItemKey};
use lofty::picture::{Picture, PictureType, MimeType};

use crate::spotify::api::SpotifyTrack;

/// Tag a downloaded audio file with Spotify metadata.
pub fn tag_file(path: &Path, track: &SpotifyTrack) -> Result<(), String> {
    let mut tagged_file = lofty::read_from_path(path)
        .map_err(|e| format!("Failed to read audio file for tagging: {e}"))?;

    // Get or create a tag (prefer vorbis comments for opus, ID3v2 for mp3)
    let tag_type = tagged_file
        .primary_tag()
        .map(|t| t.tag_type())
        .unwrap_or(TagType::VorbisComments);

    if tagged_file.tag(tag_type).is_none() {
        tagged_file.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged_file.tag_mut(tag_type).unwrap();

    tag.set_title(track.name.clone());
    tag.set_artist(track.artists.join(", "));
    tag.set_album(track.album_name.clone());
    tag.set_track(track.track_number);

    // Album artist (first artist or join all)
    tag.insert(lofty::tag::TagItem::new(
        ItemKey::AlbumArtist,
        lofty::tag::ItemValue::Text(track.artists.first().cloned().unwrap_or_default()),
    ));

    // Download and embed album art
    if let Some(ref art_url) = track.album_art_url {
        match download_image(art_url) {
            Ok(image_data) => {
                let mime = if art_url.contains(".png") {
                    MimeType::Png
                } else {
                    MimeType::Jpeg
                };
                let picture = Picture::unchecked(image_data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(mime)
                    .build();
                tag.push_picture(picture);
            }
            Err(e) => {
                eprintln!("[tagger] Failed to download album art: {e}");
            }
        }
    }

    tagged_file
        .save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("Failed to write tags: {e}"))?;

    eprintln!(
        "[tagger] Tagged: {} - {} ({})",
        track.artists.join(", "),
        track.name,
        track.album_name
    );

    Ok(())
}

fn download_image(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::new_with_defaults();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("Image download failed: {e}"))?;

    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read image: {e}"))?;

    Ok(bytes)
}
