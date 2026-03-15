// SPDX-License-Identifier: GPL-3.0

//! Discogs REST API client for release search and metadata lookup (blocking).
//!
//! Uses a personal access token for authentication.
//! Rate limit: 60 requests per minute for authenticated requests.

use super::{LookupRelease, LookupSource, LookupTrack};
use serde::Deserialize;

const DISCOGS_API_BASE: &str = "https://api.discogs.com";
const USER_AGENT: &str = "Lyra/0.1.0 +https://github.com/M0Rf30/lyra";

/// Search for releases on Discogs.
pub fn search_releases(
    client: &reqwest::blocking::Client,
    query: &str,
    token: &str,
) -> Result<Vec<LookupRelease>, String> {
    let resp = client
        .get(format!("{DISCOGS_API_BASE}/database/search"))
        .query(&[("q", query), ("type", "release"), ("per_page", "20")])
        .header("User-Agent", USER_AGENT)
        .header("Authorization", format!("Discogs token={token}"))
        .send()
        .map_err(|e| format!("Discogs request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Discogs API error: HTTP {status}: {body}"));
    }

    let body: DiscogsSearchResponse = resp
        .json()
        .map_err(|e| format!("Failed to parse Discogs response: {e}"))?;

    Ok(body
        .results
        .into_iter()
        .map(|r| {
            let parts: Vec<&str> = r.title.splitn(2, " - ").collect();
            let artist = parts.first().unwrap_or(&"").to_string();
            let title = parts.get(1).unwrap_or(&r.title.as_str()).to_string();
            LookupRelease {
                id: r.id.to_string(),
                title,
                artist,
                year: r.year.map(|y: u32| y.to_string()),
                label: r.label.and_then(|l: Vec<String>| l.into_iter().next()),
                genres: r.genre.unwrap_or_default(),
                tracks: Vec::new(),
                cover_url: r.cover_image,
                source: LookupSource::Discogs,
            }
        })
        .collect())
}

/// Fetch a release by Discogs ID with full track listing.
pub fn fetch_release(
    client: &reqwest::blocking::Client,
    id: u64,
    token: &str,
) -> Result<LookupRelease, String> {
    let resp = client
        .get(format!("{DISCOGS_API_BASE}/releases/{id}"))
        .header("User-Agent", USER_AGENT)
        .header("Authorization", format!("Discogs token={token}"))
        .send()
        .map_err(|e| format!("Discogs request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Discogs API error: HTTP {}", resp.status()));
    }

    let release: DiscogsRelease = resp
        .json()
        .map_err(|e| format!("Failed to parse Discogs release: {e}"))?;

    let artist = release
        .artists
        .as_ref()
        .and_then(|a| a.first())
        .map(|a| a.name.clone())
        .unwrap_or_default();

    let tracks: Vec<LookupTrack> = release
        .tracklist
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, t)| LookupTrack {
            position: t
                .position
                .as_ref()
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(i as u32 + 1),
            title: t.title,
            artist: t
                .artists
                .as_ref()
                .and_then(|a| a.first())
                .map(|a| a.name.clone())
                .unwrap_or_else(|| artist.clone()),
            duration_ms: parse_duration(&t.duration.unwrap_or_default()),
        })
        .collect();

    let year = release.year.map(|y| y.to_string());

    let label = release
        .labels
        .as_ref()
        .and_then(|l| l.first())
        .map(|l| l.name.clone());

    let cover_url = release
        .images
        .as_ref()
        .and_then(|imgs| imgs.iter().find(|i| i.image_type == "primary"))
        .or_else(|| release.images.as_ref().and_then(|imgs| imgs.first()))
        .map(|i| i.uri.clone());

    Ok(LookupRelease {
        id: id.to_string(),
        title: release.title,
        artist,
        year,
        label,
        genres: release.genres.unwrap_or_default(),
        tracks,
        cover_url,
        source: LookupSource::Discogs,
    })
}

/// Parse a Discogs duration string like "3:45" into milliseconds.
fn parse_duration(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            let mins: u64 = parts[0].parse().ok()?;
            let secs: u64 = parts[1].parse().ok()?;
            Some((mins * 60 + secs) * 1000)
        }
        3 => {
            let hours: u64 = parts[0].parse().ok()?;
            let mins: u64 = parts[1].parse().ok()?;
            let secs: u64 = parts[2].parse().ok()?;
            Some((hours * 3600 + mins * 60 + secs) * 1000)
        }
        _ => None,
    }
}

// --- Discogs JSON response types ---

#[derive(Deserialize)]
struct DiscogsSearchResponse {
    results: Vec<DiscogsSearchResult>,
}

#[derive(Deserialize)]
struct DiscogsSearchResult {
    id: u64,
    title: String,
    year: Option<u32>,
    label: Option<Vec<String>>,
    genre: Option<Vec<String>>,
    cover_image: Option<String>,
}

#[derive(Deserialize)]
struct DiscogsRelease {
    title: String,
    year: Option<u32>,
    artists: Option<Vec<DiscogsArtist>>,
    labels: Option<Vec<DiscogsLabel>>,
    genres: Option<Vec<String>>,
    tracklist: Option<Vec<DiscogsTrack>>,
    images: Option<Vec<DiscogsImage>>,
}

#[derive(Deserialize)]
struct DiscogsArtist {
    name: String,
}

#[derive(Deserialize)]
struct DiscogsLabel {
    name: String,
}

#[derive(Deserialize)]
struct DiscogsTrack {
    position: Option<String>,
    title: String,
    duration: Option<String>,
    artists: Option<Vec<DiscogsArtist>>,
}

#[derive(Deserialize)]
struct DiscogsImage {
    #[serde(rename = "type")]
    image_type: String,
    uri: String,
}
