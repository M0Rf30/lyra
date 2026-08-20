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
    if !response.status().is_success() {
        return Err(format!("Radio search returned HTTP {}", response.status()));
    }
    let body = super::read_capped_body(response, MAX_JSON_RESPONSE_BYTES)?;
    let raw: Vec<StationRaw> =
        serde_json::from_slice(&body).map_err(|e| format!("Radio response parse failed: {e}"))?;
    Ok(map_station_results(raw))
}

/// Fetch the globally most-clicked stations from the radio-browser.info
/// directory, letting users discover popular stations without already
/// knowing a name to search for.
pub fn popular_stations(
    client: &reqwest::blocking::Client,
    limit: u32,
) -> Result<Vec<StationSearchResult>, String> {
    let url = format!(
        "https://all.api.radio-browser.info/json/stations/topclick/{limit}?hidebroken=true"
    );
    let response = client
        .get(&url)
        .header("User-Agent", "lyra/0.1")
        .send()
        .map_err(|e| format!("Radio search failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Radio search returned HTTP {}", response.status()));
    }
    let body = super::read_capped_body(response, MAX_JSON_RESPONSE_BYTES)?;
    let raw: Vec<StationRaw> =
        serde_json::from_slice(&body).map_err(|e| format!("Radio response parse failed: {e}"))?;
    Ok(map_station_results(raw))
}

/// Cap on the radio-browser.info JSON response body. The directory's own
/// `limit=`/count params bound normal responses to well under this, so
/// hitting the cap means a hostile or broken server, not a legitimate
/// large result set.
const MAX_JSON_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

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
        // `key.get(..4)`/`key.get(4..)` (not byte-slicing) so a key
        // containing multi-byte UTF-8 within its first 4 bytes — from a
        // malformed/hostile playlist — can never land mid-character and
        // panic; it just fails to match "file" and the line is skipped.
        let Some(prefix) = key.get(..4) else { continue };
        if !prefix.eq_ignore_ascii_case("file") {
            continue;
        }
        let Some(suffix) = key.get(4..) else { continue };
        let Ok(n) = suffix.parse::<u32>() else { continue };
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

/// Maximum number of chained playlist fetches `resolve_stream_url` will
/// follow. Some Shoutcast/Icecast directories publish a playlist whose
/// first (lowest-numbered/first-listed) entry is itself another playlist —
/// e.g. a `.pls` wrapping a mirrored `.m3u` — so one fetch-and-parse pass
/// doesn't always land on a direct stream. The cap bounds the number of
/// network round-trips so a pathological or self-referential chain can
/// never recurse indefinitely or block the caller forever.
const MAX_PLAYLIST_HOPS: u32 = 3;

/// Resolve a station URL to a directly playable stream URL. If `url` looks
/// like a `.pls`/`.m3u`/`.m3u8` playlist (by extension, or by the response's
/// `Content-Type` once fetched), fetches it and extracts the first stream
/// entry; otherwise returns `url` unchanged. Some directories chain
/// playlists (the extracted entry is itself another playlist), so this
/// repeats the fetch-and-extract step for up to `MAX_PLAYLIST_HOPS` hops,
/// stopping as soon as a hop's result no longer looks like a playlist. If
/// the chain is still unresolved at the cap, that's a clear error rather
/// than silently handing back a playlist URL as if it were a direct stream.
pub fn resolve_stream_url(client: &reqwest::blocking::Client, url: &str) -> Result<String, String> {
    /// Playlist bodies (.pls/.m3u) are always small hand-written text
    /// files; a hostile or broken server returning an unbounded body must
    /// not be read into memory in full.
    const MAX_PLAYLIST_RESPONSE_BYTES: u64 = 1024 * 1024;

    let mut current = url.to_string();
    for _ in 0..MAX_PLAYLIST_HOPS {
        let Some(extension_hint) = sniff_playlist_format(&current, None) else {
            return Ok(current);
        };
        let response = client
            .get(&current)
            .send()
            .map_err(|e| format!("Failed to fetch playlist: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("Failed to fetch playlist: HTTP {}", response.status()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let format =
            sniff_playlist_format(&current, content_type.as_deref()).unwrap_or(extension_hint);
        let bytes = super::read_capped_body(response, MAX_PLAYLIST_RESPONSE_BYTES)?;
        let body = String::from_utf8(bytes)
            .map_err(|e| format!("Playlist was not valid UTF-8: {e}"))?;
        current = parse_playlist(&body, format)
            .ok_or_else(|| "Playlist contained no stream URL".to_string())?;
    }
    if sniff_playlist_format(&current, None).is_some() {
        Err(format!(
            "Playlist resolution exceeded {MAX_PLAYLIST_HOPS} hops without reaching a stream URL"
        ))
    } else {
        Ok(current)
    }
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
    fn parse_pls_ignores_multibyte_key_without_panicking() {
        // A hostile/malformed playlist could put multi-byte UTF-8 right
        // before the first 4 bytes of a key; byte-slicing at index 4 would
        // land mid-character and panic. This must just skip the line.
        let body = "fïle1=https://x.example/a.mp3\nFile1=https://x.example/b.mp3\n";
        assert_eq!(parse_pls(body), Some("https://x.example/b.mp3".to_string()));
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
