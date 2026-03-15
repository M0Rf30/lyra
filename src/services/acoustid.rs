// SPDX-License-Identifier: GPL-3.0

//! AcoustID integration: audio fingerprinting via `fpcalc` and AcoustID web API lookups.

use super::{FingerprintResult, LookupRelease, LookupSource};
use serde::Deserialize;
use std::path::Path;

const ACOUSTID_API_URL: &str = "https://api.acoustid.org/v2/lookup";

/// Generate an audio fingerprint using the `fpcalc` command-line tool (from chromaprint).
///
/// Returns the compressed fingerprint string and duration in seconds.
pub fn fingerprint_file(path: &Path) -> Result<FingerprintResult, String> {
    let output = std::process::Command::new("fpcalc")
        .arg("-json")
        .arg(path.as_os_str())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "fpcalc not found. Install chromaprint (e.g. `sudo pacman -S chromaprint` or `sudo apt install libchromaprint-tools`).".to_string()
            } else {
                format!("Failed to run fpcalc: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("fpcalc failed: {stderr}"));
    }

    let json: FpcalcOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse fpcalc output: {e}"))?;

    Ok(FingerprintResult {
        fingerprint: json.fingerprint,
        duration: json.duration as u32,
    })
}

/// Look up a fingerprint on the AcoustID service (blocking).
pub fn lookup(
    client: &reqwest::blocking::Client,
    api_key: &str,
    fingerprint: &str,
    duration: u32,
) -> Result<Vec<LookupRelease>, String> {
    let resp = client
        .get(ACOUSTID_API_URL)
        .query(&[
            ("client", api_key),
            ("duration", &duration.to_string()),
            ("fingerprint", fingerprint),
            ("meta", "recordings releases"),
            ("format", "json"),
        ])
        .send()
        .map_err(|e| format!("AcoustID request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("AcoustID API error: HTTP {}", resp.status()));
    }

    let body: AcoustIdResponse = resp
        .json()
        .map_err(|e| format!("Failed to parse AcoustID response: {e}"))?;

    if body.status != "ok" {
        return Err(format!(
            "AcoustID error: {}",
            body.error.as_deref().unwrap_or("unknown")
        ));
    }

    let mut releases = Vec::new();
    for result in body.results.unwrap_or_default() {
        for recording in result.recordings.unwrap_or_default() {
            for release in recording.releases.unwrap_or_default() {
                releases.push(LookupRelease {
                    id: release.id.clone(),
                    title: release.title.unwrap_or_default(),
                    artist: recording
                        .artists
                        .as_ref()
                        .and_then(|a: &Vec<AcoustIdArtist>| a.first())
                        .map(|a| a.name.clone())
                        .unwrap_or_default(),
                    year: release
                        .date
                        .as_ref()
                        .and_then(|d: &AcoustIdDate| d.year.map(|y| y.to_string())),
                    label: None,
                    genres: Vec::new(),
                    tracks: Vec::new(),
                    cover_url: None,
                    source: LookupSource::AcoustId,
                });
            }
        }
    }

    // Deduplicate by release ID
    releases.sort_by(|a, b| a.id.cmp(&b.id));
    releases.dedup_by(|a, b| a.id == b.id);

    Ok(releases)
}

/// Submit a fingerprint to AcoustID (blocking, requires a submission API key).
pub fn submit(
    client: &reqwest::blocking::Client,
    api_key: &str,
    user_key: &str,
    fingerprint: &str,
    duration: u32,
    mbid: &str,
) -> Result<(), String> {
    let resp = client
        .post("https://api.acoustid.org/v2/submit")
        .form(&[
            ("client", api_key),
            ("user", user_key),
            ("duration", &duration.to_string()),
            ("fingerprint", fingerprint),
            ("mbid", mbid),
            ("format", "json"),
        ])
        .send()
        .map_err(|e| format!("AcoustID submit failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("AcoustID submit error: HTTP {}", resp.status()));
    }

    let body: AcoustIdResponse = resp
        .json()
        .map_err(|e| format!("Failed to parse submit response: {e}"))?;

    if body.status != "ok" {
        return Err(format!(
            "AcoustID submit error: {}",
            body.error.as_deref().unwrap_or("unknown")
        ));
    }

    Ok(())
}

// --- JSON response types ---

#[derive(Deserialize)]
struct FpcalcOutput {
    duration: f64,
    fingerprint: String,
}

#[derive(Deserialize)]
struct AcoustIdResponse {
    status: String,
    error: Option<String>,
    results: Option<Vec<AcoustIdResult>>,
}

#[derive(Deserialize)]
struct AcoustIdResult {
    #[allow(dead_code)]
    score: Option<f64>,
    recordings: Option<Vec<AcoustIdRecording>>,
}

#[derive(Deserialize)]
struct AcoustIdRecording {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    title: Option<String>,
    artists: Option<Vec<AcoustIdArtist>>,
    releases: Option<Vec<AcoustIdRelease>>,
}

#[derive(Deserialize)]
struct AcoustIdArtist {
    name: String,
}

#[derive(Deserialize)]
struct AcoustIdRelease {
    id: String,
    title: Option<String>,
    date: Option<AcoustIdDate>,
}

#[derive(Deserialize)]
struct AcoustIdDate {
    year: Option<u32>,
}
