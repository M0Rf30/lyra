// SPDX-License-Identifier: GPL-3.0

//! MusicBrainz web API client for release/recording search and lookup (blocking).
//!
//! Uses the MusicBrainz JSON web service v2 directly via reqwest.
//! Rate limit: 1 request per second (enforced by caller).

use super::{LookupRelease, LookupSource, LookupTrack, RateLimiter};
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

const MB_API_BASE: &str = "https://musicbrainz.org/ws/2";
const COVER_ART_BASE: &str = "https://coverartarchive.org/release";
const USER_AGENT: &str = "Lyra/0.1.0 (https://github.com/M0Rf30/lyra)";

/// MusicBrainz enforces a strict 1 request/second rate limit.
static RATE_LIMITER: LazyLock<RateLimiter> =
    LazyLock::new(|| RateLimiter::new(Duration::from_millis(1100)));

/// Search for releases matching a text query.
pub fn search_releases(
    client: &reqwest::blocking::Client,
    query: &str,
) -> Result<Vec<LookupRelease>, String> {
    RATE_LIMITER.wait();
    let resp = client
        .get(format!("{MB_API_BASE}/release/"))
        .query(&[("query", query), ("fmt", "json"), ("limit", "20")])
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("MusicBrainz request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("MusicBrainz API error: HTTP {}", resp.status()));
    }

    let body: MbReleaseSearchResponse = resp
        .json()
        .map_err(|e| format!("Failed to parse MusicBrainz response: {e}"))?;

    Ok(body
        .releases
        .into_iter()
        .map(|r| mb_release_to_lookup(r, false))
        .collect())
}

/// Search for recordings matching a text query.
pub fn search_recordings(
    client: &reqwest::blocking::Client,
    query: &str,
) -> Result<Vec<LookupRelease>, String> {
    RATE_LIMITER.wait();
    let resp = client
        .get(format!("{MB_API_BASE}/recording/"))
        .query(&[("query", query), ("fmt", "json"), ("limit", "20")])
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("MusicBrainz request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("MusicBrainz API error: HTTP {}", resp.status()));
    }

    let body: MbRecordingSearchResponse = resp
        .json()
        .map_err(|e| format!("Failed to parse MusicBrainz response: {e}"))?;

    let mut releases = Vec::new();
    for recording in body.recordings {
        let artist = recording
            .artist_credit
            .as_ref()
            .and_then(|ac| ac.first())
            .map(|a| a.artist.name.clone())
            .unwrap_or_default();

        for release in recording.releases.unwrap_or_default() {
            releases.push(LookupRelease {
                id: release.id.clone(),
                title: release.title.unwrap_or_default(),
                artist: artist.clone(),
                year: release
                    .date
                    .as_ref()
                    .and_then(|d| d.get(..4).map(String::from)),
                label: None,
                genres: Vec::new(),
                tracks: vec![LookupTrack {
                    position: 1,
                    title: recording.title.clone(),
                    artist: artist.clone(),
                    duration_ms: recording.length,
                }],
                cover_url: Some(format!("{COVER_ART_BASE}/{}/front-250", release.id)),
                source: LookupSource::MusicBrainz,
            });
        }
    }

    Ok(releases)
}

/// Fetch a release by MusicBrainz ID with full track listing.
pub fn fetch_release(
    client: &reqwest::blocking::Client,
    mbid: &str,
) -> Result<LookupRelease, String> {
    RATE_LIMITER.wait();
    let resp = client
        .get(format!("{MB_API_BASE}/release/{mbid}"))
        .query(&[
            ("inc", "recordings+artists+labels+release-groups"),
            ("fmt", "json"),
        ])
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("MusicBrainz request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("MusicBrainz API error: HTTP {}", resp.status()));
    }

    let release: MbRelease = resp
        .json()
        .map_err(|e| format!("Failed to parse MusicBrainz release: {e}"))?;

    Ok(mb_release_to_lookup(release, true))
}

fn mb_release_to_lookup(r: MbRelease, include_cover: bool) -> LookupRelease {
    let artist = r
        .artist_credit
        .as_ref()
        .and_then(|ac| ac.first())
        .map(|a| a.artist.name.clone())
        .unwrap_or_default();

    let year = r.date.as_ref().and_then(|d| d.get(..4).map(String::from));

    let label = r
        .label_info
        .as_ref()
        .and_then(|li| li.first())
        .and_then(|l| l.label.as_ref())
        .map(|l| l.name.clone());

    let mut tracks = Vec::new();
    for medium in r.media.unwrap_or_default() {
        for track in medium.tracks.unwrap_or_default() {
            let track_artist = track
                .artist_credit
                .as_ref()
                .and_then(|ac| ac.first())
                .map(|a| a.artist.name.clone())
                .unwrap_or_else(|| artist.clone());

            tracks.push(LookupTrack {
                position: track.position.unwrap_or(0),
                title: track.title,
                artist: track_artist,
                duration_ms: track.length,
            });
        }
    }

    let cover_url = if include_cover {
        Some(format!("{COVER_ART_BASE}/{}/front-250", r.id))
    } else {
        None
    };

    let genres = r
        .release_group
        .and_then(|rg| rg.genres)
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.name)
        .collect();

    LookupRelease {
        id: r.id,
        title: r.title,
        artist,
        year,
        label,
        genres,
        tracks,
        cover_url,
        source: LookupSource::MusicBrainz,
    }
}

// --- MusicBrainz JSON response types ---

#[derive(Deserialize)]
struct MbReleaseSearchResponse {
    releases: Vec<MbRelease>,
}

#[derive(Deserialize)]
struct MbRecordingSearchResponse {
    recordings: Vec<MbRecording>,
}

#[derive(Deserialize)]
struct MbRecording {
    title: String,
    length: Option<u64>,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<MbArtistCredit>>,
    releases: Option<Vec<MbReleaseStub>>,
}

#[derive(Deserialize)]
struct MbReleaseStub {
    id: String,
    title: Option<String>,
    date: Option<String>,
}

#[derive(Deserialize)]
struct MbRelease {
    id: String,
    title: String,
    date: Option<String>,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<MbArtistCredit>>,
    #[serde(rename = "label-info")]
    label_info: Option<Vec<MbLabelInfo>>,
    media: Option<Vec<MbMedium>>,
    #[serde(rename = "release-group")]
    release_group: Option<MbReleaseGroup>,
}

#[derive(Deserialize)]
struct MbArtistCredit {
    artist: MbArtist,
}

#[derive(Deserialize)]
struct MbArtist {
    name: String,
}

#[derive(Deserialize)]
struct MbLabelInfo {
    label: Option<MbLabel>,
}

#[derive(Deserialize)]
struct MbLabel {
    name: String,
}

#[derive(Deserialize)]
struct MbMedium {
    tracks: Option<Vec<MbTrack>>,
}

#[derive(Deserialize)]
struct MbTrack {
    position: Option<u32>,
    title: String,
    length: Option<u64>,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<MbArtistCredit>>,
}

#[derive(Deserialize)]
struct MbReleaseGroup {
    genres: Option<Vec<MbGenre>>,
}

#[derive(Deserialize)]
struct MbGenre {
    name: String,
}
