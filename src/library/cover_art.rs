// SPDX-License-Identifier: GPL-3.0

//! Cover art extraction from audio files and directory images.

use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::Path;

/// Handles cover art extraction and caching.
pub struct CoverArt;

impl CoverArt {
    /// Extract embedded cover art from an audio file.
    /// Returns the raw image bytes (JPEG/PNG) if found.
    pub fn extract_from_file(path: &Path) -> Option<Vec<u8>> {
        let tagged_file = Probe::open(path).ok()?.read().ok()?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;

        // Prefer front cover, but take any picture
        let pictures = tag.pictures();
        let pic = pictures
            .iter()
            .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
            .or_else(|| pictures.first())?;

        Some(pic.data().to_vec())
    }

    /// Look for cover art files in the same directory as the audio file.
    /// Common names: cover.jpg, folder.jpg, front.jpg, album.jpg, etc.
    pub fn find_in_directory(audio_path: &Path) -> Option<Vec<u8>> {
        let dir = audio_path.parent()?;

        let cover_names = [
            "cover", "folder", "front", "album", "artwork", "art", "thumb",
        ];
        let extensions = ["jpg", "jpeg", "png", "webp", "bmp"];

        for name in &cover_names {
            for ext in &extensions {
                let candidate = dir.join(format!("{name}.{ext}"));
                if candidate.exists() {
                    return std::fs::read(&candidate).ok();
                }
                // Also check uppercase
                let candidate_upper = dir.join(format!("{}.{ext}", name.to_uppercase()));
                if candidate_upper.exists() {
                    return std::fs::read(&candidate_upper).ok();
                }
            }
        }

        None
    }

    /// Get cover art for a track: try embedded first, then directory.
    pub fn get_cover_art(audio_path: &Path) -> Option<Vec<u8>> {
        Self::extract_from_file(audio_path).or_else(|| Self::find_in_directory(audio_path))
    }

    /// Generate an album key for caching (artist + album).
    pub fn album_key(artist: &str, album: &str) -> String {
        format!("{artist}||{album}")
    }
}
