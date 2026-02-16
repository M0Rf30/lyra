// SPDX-License-Identifier: GPL-3.0

//! Parsers for AutoEQ file formats (INDEX.md and FixedBandEQ.txt).

use super::{AutoEQError, AutoEQProfile, AutoEQProfileMetadata, Result};
use regex::Regex;

/// Parse the AutoEQ INDEX.md file to extract profile metadata.
///
/// The INDEX.md file contains markdown links in the format:
/// `- [Display Name](./path/to/profile)`
pub fn parse_index(content: &str) -> Result<Vec<AutoEQProfileMetadata>> {
    let link_regex = Regex::new(r"-\s*\[([^\]]+)\]\(\./([^)]+)\)")
        .map_err(|e| AutoEQError::InvalidFormat(format!("Regex error: {}", e)))?;

    let mut profiles = Vec::new();

    for line in content.lines() {
        if let Some(captures) = link_regex.captures(line) {
            let name = captures.get(1).unwrap().as_str();
            let path = captures.get(2).unwrap().as_str();

            // URL-decode name and path
            let name = urlencoding::decode(name)
                .unwrap_or_else(|_| std::borrow::Cow::Borrowed(name))
                .to_string();
            let path_str = urlencoding::decode(path)
                .unwrap_or_else(|_| std::borrow::Cow::Borrowed(path))
                .to_string();

            // Extract source and type from path (format: source/type/name)
            let parts: Vec<&str> = path_str.split('/').collect();
            let (source, type_) = if parts.len() >= 2 {
                (
                    urlencoding::decode(parts[0])
                        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(parts[0]))
                        .to_string(),
                    urlencoding::decode(parts[1])
                        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(parts[1]))
                        .to_string(),
                )
            } else {
                (String::new(), String::new())
            };

            profiles.push(AutoEQProfileMetadata {
                name,
                path: path.to_string(),
                source,
                type_,
            });
        }
    }

    Ok(profiles)
}

/// Parse a FixedBandEQ.txt file to extract equalizer settings.
///
/// Format:
/// ```text
/// Preamp: -6.5 dB
/// Filter 1: ON PK Fc 31 Hz Gain 5.0 dB Q 0.70
/// ...
/// Filter 10: ON PK Fc 16000 Hz Gain -2.0 dB Q 0.70
/// ```
pub fn parse_fixed_band_eq(path: &str, content: &str) -> Result<AutoEQProfile> {
    // Parse preamp
    let preamp_regex = Regex::new(r"Preamp:\s*([-+]?\d+\.?\d*)\s*dB")
        .map_err(|e| AutoEQError::InvalidFormat(format!("Regex error: {}", e)))?;

    let preamp = preamp_regex
        .captures(content)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f32>().ok())
        .ok_or_else(|| AutoEQError::InvalidFormat("Preamp not found or invalid".to_string()))?;

    // Parse filters
    let filter_regex =
        Regex::new(r"Filter\s+\d+:.*?Fc\s+(\d+)\s+Hz.*?Gain\s+([-+]?\d+\.?\d*)\s*dB")
            .map_err(|e| AutoEQError::InvalidFormat(format!("Regex error: {}", e)))?;

    let mut gains = Vec::new();
    for captures in filter_regex.captures_iter(content) {
        let freq: u32 = captures.get(1).unwrap().as_str().parse().unwrap();
        let gain: f32 = captures.get(2).unwrap().as_str().parse().unwrap();
        gains.push((freq, gain));
    }

    if gains.len() != 10 {
        return Err(AutoEQError::InvalidFormat(format!(
            "Expected 10 filters, found {}",
            gains.len()
        )));
    }

    // Verify frequencies match expected (with tolerance)
    let expected_freqs = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    for (i, (freq, _)) in gains.iter().enumerate() {
        if *freq != expected_freqs[i] {
            tracing::warn!(
                "AutoEQ filter {} has unexpected frequency {} Hz (expected {} Hz)",
                i + 1,
                freq,
                expected_freqs[i]
            );
        }
    }

    // Extract bands (just gains, ordered)
    let bands: [f32; 10] = gains
        .iter()
        .map(|(_, gain)| *gain)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();

    // Extract name, source, type from path
    let decoded_path = urlencoding::decode(path)
        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(path))
        .to_string();

    let parts: Vec<&str> = decoded_path.split('/').collect();
    let (name, source, type_) = if parts.len() >= 3 {
        (
            parts[parts.len() - 1].to_string(),
            parts[0].to_string(),
            parts[1].to_string(),
        )
    } else {
        (decoded_path.clone(), String::new(), String::new())
    };

    Ok(AutoEQProfile {
        name,
        path: path.to_string(),
        source,
        type_,
        preamp,
        bands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_index() {
        let content = r#"
# Index

- [Sennheiser HD 650](./oratory1990/over-ear/Sennheiser%20HD%20650)
- [Sony WH-1000XM4](./rtings/over-ear/Sony%20WH-1000XM4)
        "#;

        let profiles = parse_index(content).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Sennheiser HD 650");
        assert_eq!(profiles[0].source, "oratory1990");
        assert_eq!(profiles[0].type_, "over-ear");
    }

    #[test]
    fn test_parse_fixed_band_eq() {
        let content = r#"
Preamp: -6.5 dB
Filter 1: ON PK Fc 31 Hz Gain 5.0 dB Q 0.70
Filter 2: ON PK Fc 62 Hz Gain 4.0 dB Q 0.70
Filter 3: ON PK Fc 125 Hz Gain 3.0 dB Q 0.70
Filter 4: ON PK Fc 250 Hz Gain 2.0 dB Q 0.70
Filter 5: ON PK Fc 500 Hz Gain 1.0 dB Q 0.70
Filter 6: ON PK Fc 1000 Hz Gain 0.0 dB Q 0.70
Filter 7: ON PK Fc 2000 Hz Gain -1.0 dB Q 0.70
Filter 8: ON PK Fc 4000 Hz Gain -2.0 dB Q 0.70
Filter 9: ON PK Fc 8000 Hz Gain -3.0 dB Q 0.70
Filter 10: ON PK Fc 16000 Hz Gain -4.0 dB Q 0.70
        "#;

        let profile = parse_fixed_band_eq("oratory1990/over-ear/Test", content).unwrap();
        assert_eq!(profile.preamp, -6.5);
        assert_eq!(profile.bands[0], 5.0);
        assert_eq!(profile.bands[9], -4.0);
        assert_eq!(profile.name, "Test");
        assert_eq!(profile.source, "oratory1990");
    }
}
