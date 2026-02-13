// SPDX-License-Identifier: GPL-3.0

//! Lyrics fetching from embedded tags and online sources.

use lofty::prelude::*;
use lofty::probe::Probe;
use std::path::Path;

/// Provides lyrics from various sources.
pub struct LyricsProvider;

impl LyricsProvider {
    /// Extract embedded lyrics from the audio file's tags.
    pub fn from_tags(path: &Path) -> Option<String> {
        let tagged_file = Probe::open(path).ok()?.read().ok()?;

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;

        // Try the standard Lyrics item key
        tag.get_string(&ItemKey::Lyrics)
            .map(|s| s.to_string())
    }

    /// Fetch lyrics from the LRCLIB API (free, no key required).
    /// Returns plain lyrics text if found.
    pub async fn fetch_online(artist: &str, title: &str) -> Option<String> {
        let url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}",
            urlencoded(artist),
            urlencoded(title),
        );

        let response = reqwest::get(&url).await.ok()?;

        if !response.status().is_success() {
            return None;
        }

        let json: serde_json::Value = response.json().await.ok()?;

        // Prefer synced lyrics, fall back to plain
        json.get("syncedLyrics")
            .or_else(|| json.get("plainLyrics"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Look for a .lrc file next to the audio file.
    pub fn from_lrc_file(audio_path: &Path) -> Option<String> {
        let lrc_path = audio_path.with_extension("lrc");
        if lrc_path.exists() {
            std::fs::read_to_string(&lrc_path).ok()
        } else {
            None
        }
    }

    /// Try all sources: embedded tags -> .lrc file -> online.
    pub async fn get_lyrics(path: &Path, artist: &str, title: &str) -> Option<String> {
        // 1. Embedded in tags
        if let Some(lyrics) = Self::from_tags(path)
            && !lyrics.trim().is_empty() {
                return Some(lyrics);
            }

        // 2. .lrc sidecar file
        if let Some(lyrics) = Self::from_lrc_file(path)
            && !lyrics.trim().is_empty() {
                return Some(lyrics);
            }

        // 3. Online (LRCLIB)
        Self::fetch_online(artist, title).await
    }
}

/// Simple URL encoding for query parameters.
fn urlencoded(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
