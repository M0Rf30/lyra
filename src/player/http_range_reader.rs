// SPDX-License-Identifier: GPL-3.0

//! HTTP Range-based seekable reader.
//!
//! Wraps an HTTP URL so that it implements `Read + Seek`. Playback starts
//! from the beginning of the response body (streaming), and seeking is
//! achieved by dropping the current connection and re-requesting with an
//! appropriate `Range: bytes=N-` header.
//!
//! This allows rodio/symphonia to decode audio from a remote server with
//! full seek support, as long as the server advertises `Accept-Ranges: bytes`
//! (which Navidrome does for raw / cached-transcoded files).

use std::io::{self, Read, Seek, SeekFrom};

/// A seekable reader backed by HTTP Range requests.
///
/// - `Read::read()` reads from the current HTTP response body.
/// - `Seek::seek()` drops the current connection and opens a new one at
///   the target byte offset via `Range: bytes=N-`.
pub struct HttpRangeReader {
    url: String,
    client: reqwest::blocking::Client,
    response: Option<reqwest::blocking::Response>,
    /// Current byte position in the stream.
    position: u64,
    /// Total content length (from initial HEAD or first GET response).
    content_length: u64,
}

impl HttpRangeReader {
    /// Create a new reader for the given URL.
    ///
    /// Makes an initial GET request to start streaming and reads
    /// `Content-Length` from the response. Returns an error if the
    /// request fails or the server doesn't provide Content-Length.
    pub fn new(url: String) -> Result<Self, String> {
        let client = reqwest::blocking::Client::new();

        // Start streaming from byte 0.
        let response = client
            .get(&url)
            .send()
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let content_length = response.content_length().unwrap_or(0);

        if content_length == 0 {
            tracing::warn!("HTTP stream has unknown or zero Content-Length, seeking may not work");
        }

        tracing::info!(
            "HttpRangeReader: opened {} ({} bytes)",
            url.split('?').next().unwrap_or(&url),
            content_length,
        );

        Ok(Self {
            url,
            client,
            response: Some(response),
            position: 0,
            content_length,
        })
    }

    /// Total content length in bytes (0 if unknown).
    pub fn content_length(&self) -> u64 {
        self.content_length
    }

    /// Open a new HTTP connection starting at `byte_offset`.
    #[tracing::instrument(skip(self), level = "debug")]
    fn open_at(&mut self, byte_offset: u64) -> io::Result<()> {
        // Drop the old response (closes the connection).
        self.response = None;

        let response = self
            .client
            .get(&self.url)
            .header("Range", format!("bytes={byte_offset}-"))
            .send()
            .map_err(|e| io::Error::other(format!("HTTP Range request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() && status.as_u16() != 206 {
            return Err(io::Error::other(format!(
                "HTTP Range request returned {status}"
            )));
        }

        self.response = Some(response);
        self.position = byte_offset;
        Ok(())
    }
}

impl Read for HttpRangeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let response = self
            .response
            .as_mut()
            .ok_or_else(|| io::Error::other("no active HTTP response"))?;

        let n = response.read(buf)?;
        self.position += n as u64;
        Ok(n)
    }
}

impl Seek for HttpRangeReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => {
                if n >= 0 {
                    self.position.saturating_add(n as u64)
                } else {
                    self.position.saturating_sub((-n) as u64)
                }
            }
            SeekFrom::End(n) => {
                if self.content_length == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "SeekFrom::End not supported without Content-Length",
                    ));
                }
                if n >= 0 {
                    self.content_length.saturating_add(n as u64)
                } else {
                    self.content_length.saturating_sub((-n) as u64)
                }
            }
        };

        // Clamp to content bounds.
        let target = if self.content_length > 0 {
            target.min(self.content_length)
        } else {
            target
        };

        // If we're already at the target, no need to reconnect.
        if target == self.position {
            return Ok(self.position);
        }

        // Small forward seeks (< 256KB) can be done by discarding bytes
        // instead of making a new HTTP request — much faster for symphonia's
        // typical small seeks during format probing.
        let forward_delta = target.saturating_sub(self.position);
        if target > self.position
            && forward_delta < 256 * 1024
            && let Some(ref mut response) = self.response
        {
            let mut remaining = forward_delta;
            let mut skip_buf = [0u8; 8192];
            while remaining > 0 {
                let to_read = remaining.min(skip_buf.len() as u64) as usize;
                let n = response.read(&mut skip_buf[..to_read])?;
                if n == 0 {
                    break; // EOF
                }
                remaining -= n as u64;
            }
            self.position = target;
            return Ok(self.position);
        }

        self.open_at(target)?;
        Ok(self.position)
    }
}
