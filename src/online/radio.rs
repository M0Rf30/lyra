// SPDX-License-Identifier: GPL-3.0

//! Internet radio directory search (radio-browser.info) and PLS/M3U
//! playlist resolution for Shoutcast/Icecast stations that publish a
//! playlist file instead of a direct stream URL.

use serde::Deserialize;

/// A radio-browser.info directory search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationSearchResult {
    pub name: String,
    pub url: String,
    pub homepage: String,
    pub favicon: String,
    pub tags: String,
    pub codec: String,
    pub bitrate: u32,
}

#[derive(Debug, Deserialize)]
struct StationRaw {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    url_resolved: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    favicon: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
}

/// Search the radio-browser.info station directory by name.
pub fn search_stations(
    client: &reqwest::blocking::Client,
    query: &str,
) -> Result<Vec<StationSearchResult>, String> {
    let url = format!(
        "https://all.api.radio-browser.info/json/stations/search?name={}&limit=50&hidebroken=true",
        urlencoding::encode(query)
    );
    let response = client
        .get(&url)
        .header("User-Agent", "lyra/0.1")
        .send()
        .map_err(|e| format!("Radio search failed: {e}"))?;
    let raw: Vec<StationRaw> = response
        .json()
        .map_err(|e| format!("Radio response parse failed: {e}"))?;
    Ok(map_station_results(raw))
}

fn map_station_results(raw: Vec<StationRaw>) -> Vec<StationSearchResult> {
    raw.into_iter()
        .map(|s| {
            let url = if s.url_resolved.is_empty() { s.url } else { s.url_resolved };
            StationSearchResult {
                name: s.name,
                url,
                homepage: s.homepage,
                favicon: s.favicon,
                tags: s.tags,
                codec: s.codec,
                bitrate: s.bitrate,
            }
        })
        .filter(|s| !s.url.is_empty())
        .collect()
}

/// Playlist container format, detected by file extension or content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistFormat {
    Pls,
    M3u,
}

/// Detect the playlist format of `url` from its extension, falling back to
/// `content_type` (e.g. from a `Content-Type` response header) when the
/// extension is inconclusive. Returns `None` when neither indicates a
/// playlist container, meaning `url` is presumably already a direct stream.
pub fn sniff_playlist_format(url: &str, content_type: Option<&str>) -> Option<PlaylistFormat> {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".pls") {
        return Some(PlaylistFormat::Pls);
    }
    if path.ends_with(".m3u") || path.ends_with(".m3u8") {
        return Some(PlaylistFormat::M3u);
    }
    let ct = content_type?.to_ascii_lowercase();
    if ct.contains("scpls") {
        Some(PlaylistFormat::Pls)
    } else if ct.contains("mpegurl") {
        Some(PlaylistFormat::M3u)
    } else {
        None
    }
}

/// Extract the lowest-numbered `FileN=` entry from a PLS playlist body.
pub fn parse_pls(body: &str) -> Option<String> {
    let mut best: Option<(u32, String)> = None;
    for line in body.lines() {
        let line = line.trim();
        let Some(eq_idx) = line.find('=') else { continue };
        let key = &line[..eq_idx];
        if key.len() <= 4 || !key[..4].eq_ignore_ascii_case("file") {
            continue;
        }
        let Ok(n) = key[4..].parse::<u32>() else { continue };
        let value = line[eq_idx + 1..].trim();
        if value.is_empty() {
            continue;
        }
        if best.as_ref().is_none_or(|(best_n, _)| n < *best_n) {
            best = Some((n, value.to_string()));
        }
    }
    best.map(|(_, v)| v)
}

/// Extract the first non-comment, non-blank line from an M3U playlist body.
pub fn parse_m3u(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
}

/// Parse a playlist body of the given format into its first stream URL.
pub fn parse_playlist(body: &str, format: PlaylistFormat) -> Option<String> {
    match format {
        PlaylistFormat::Pls => parse_pls(body),
        PlaylistFormat::M3u => parse_m3u(body),
    }
}

/// Resolve a station URL to a directly playable stream URL. If `url` looks
/// like a `.pls`/`.m3u`/`.m3u8` playlist (by extension, or by the response's
/// `Content-Type` once fetched), fetches it and extracts the first stream
/// entry; otherwise returns `url` unchanged.
pub fn resolve_stream_url(client: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    let Some(extension_hint) = sniff_playlist_format(url, None) else {
        return Ok(url.to_string());
    };
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to fetch playlist: {e}"))?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let format = sniff_playlist_format(url, content_type.as_deref()).unwrap_or(extension_hint);
    let body = response
        .text()
        .map_err(|e| format!("Failed to read playlist: {e}"))?;
    parse_playlist(&body, format).ok_or_else(|| "Playlist contained no stream URL".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_playlist_format_from_extension() {
        assert_eq!(
            sniff_playlist_format("https://x.example/station.pls", None),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            sniff_playlist_format("https://x.example/station.m3u8?x=1", None),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(sniff_playlist_format("https://x.example/live", None), None);
    }

    #[test]
    fn sniff_playlist_format_from_content_type() {
        assert_eq!(
            sniff_playlist_format("https://x.example/live", Some("audio/x-scpls; charset=utf-8")),
            Some(PlaylistFormat::Pls)
        );
        assert_eq!(
            sniff_playlist_format("https://x.example/live", Some("audio/x-mpegurl")),
            Some(PlaylistFormat::M3u)
        );
        assert_eq!(
            sniff_playlist_format("https://x.example/live", Some("audio/mpeg")),
            None
        );
    }

    #[test]
    fn parse_pls_picks_lowest_numbered_file_entry() {
        let body = "[playlist]\nNumberOfEntries=2\nFile2=https://x.example/b.mp3\nFile1=https://x.example/a.mp3\nTitle1=A\nVersion=2\n";
        assert_eq!(parse_pls(body), Some("https://x.example/a.mp3".to_string()));
    }

    #[test]
    fn parse_pls_is_case_insensitive_and_ignores_other_keys() {
        let body = "[Playlist]\nfile1=https://x.example/a.mp3\nLength1=-1\n";
        assert_eq!(parse_pls(body), Some("https://x.example/a.mp3".to_string()));
    }

    #[test]
    fn parse_pls_returns_none_without_file_entries() {
        assert_eq!(parse_pls("[playlist]\nNumberOfEntries=0\n"), None);
    }

    #[test]
    fn parse_m3u_skips_comments_and_blank_lines() {
        let body = "#EXTM3U\n#EXTINF:-1,Station Name\n\nhttps://x.example/stream.mp3\n";
        assert_eq!(parse_m3u(body), Some("https://x.example/stream.mp3".to_string()));
    }

    #[test]
    fn parse_m3u_returns_none_for_comments_only() {
        assert_eq!(parse_m3u("#EXTM3U\n#EXTINF:-1,Nothing\n"), None);
    }

    #[test]
    fn radio_browser_json_maps_url_resolved_with_fallback() {
        const BODY: &str = r#"[
            {"name": "Resolved Station", "url": "http://x.example/orig", "url_resolved": "http://x.example/resolved",
             "homepage": "http://x.example", "favicon": "http://x.example/favicon.ico",
             "tags": "jazz,chill", "codec": "MP3", "bitrate": 128},
            {"name": "Unresolved Station", "url": "http://y.example/live", "url_resolved": "",
             "tags": "", "codec": "AAC", "bitrate": 64},
            {"name": "No URL At All", "url": "", "url_resolved": ""}
        ]"#;
        let raw: Vec<StationRaw> = serde_json::from_str(BODY).unwrap();
        let results = map_station_results(raw);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].url, "http://x.example/resolved");
        assert_eq!(results[0].bitrate, 128);
        assert_eq!(results[1].url, "http://y.example/live");
        assert_eq!(results[1].codec, "AAC");
    }
}
