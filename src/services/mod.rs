// SPDX-License-Identifier: GPL-3.0

//! External service clients for metadata lookup (AcoustID, MusicBrainz, Discogs).

pub mod acoustid;
pub mod discogs;
pub mod musicbrainz;

/// A unified lookup result representing a release from any source.
#[derive(Debug, Clone)]
pub struct LookupRelease {
    /// Source-specific ID (MusicBrainz UUID, Discogs numeric ID, etc.).
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: Option<String>,
    pub label: Option<String>,
    pub genres: Vec<String>,
    pub tracks: Vec<LookupTrack>,
    pub cover_url: Option<String>,
    pub source: LookupSource,
}

/// A single track within a lookup release.
#[derive(Debug, Clone)]
pub struct LookupTrack {
    pub position: u32,
    pub title: String,
    pub artist: String,
    pub duration_ms: Option<u64>,
}

/// Which external service provided the lookup result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupSource {
    AcoustId,
    MusicBrainz,
    Discogs,
}

impl std::fmt::Display for LookupSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AcoustId => write!(f, "AcoustID"),
            Self::MusicBrainz => write!(f, "MusicBrainz"),
            Self::Discogs => write!(f, "Discogs"),
        }
    }
}

/// Result of fingerprinting an audio file.
#[derive(Debug, Clone)]
pub struct FingerprintResult {
    pub fingerprint: String,
    pub duration: u32,
}

/// Simple rate limiter that enforces a minimum interval between calls.
pub struct RateLimiter {
    last_call: std::sync::Mutex<Option<std::time::Instant>>,
    min_interval: std::time::Duration,
}

impl RateLimiter {
    pub fn new(min_interval: std::time::Duration) -> Self {
        Self {
            last_call: std::sync::Mutex::new(None),
            min_interval,
        }
    }

    /// Sleep if necessary to enforce the minimum interval since the last call.
    pub fn wait(&self) {
        let mut last = self.last_call.lock().unwrap();
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                std::thread::sleep(self.min_interval - elapsed);
            }
        }
        *last = Some(std::time::Instant::now());
    }
}
