// SPDX-License-Identifier: GPL-3.0

//! 10-band equalizer definitions and presets.

/// Standard 10-band equalizer center frequencies in Hz.
pub const BAND_FREQUENCIES: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Labels for UI display.
pub const BAND_LABELS: [&str; 10] = [
    "31", "62", "125", "250", "500", "1K", "2K", "4K", "8K", "16K",
];

/// Represents a single EQ band.
#[derive(Debug, Clone, Copy)]
pub struct EqualizerBand {
    pub frequency: f32,
    pub label: &'static str,
    /// Gain in dB, range: -12.0 to +12.0
    pub gain_db: f32,
}

impl EqualizerBand {
    pub fn new(index: usize, gain_db: f32) -> Self {
        Self {
            frequency: BAND_FREQUENCIES[index],
            label: BAND_LABELS[index],
            gain_db: gain_db.clamp(-12.0, 12.0),
        }
    }
}

/// Named EQ presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqPreset {
    Flat,
    Rock,
    Pop,
    Jazz,
    Classical,
    Bass,
    Treble,
    Vocal,
}

impl EqPreset {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Rock => "Rock",
            Self::Pop => "Pop",
            Self::Jazz => "Jazz",
            Self::Classical => "Classical",
            Self::Bass => "Bass Boost",
            Self::Treble => "Treble Boost",
            Self::Vocal => "Vocal",
        }
    }

    pub fn gains(&self) -> [f32; 10] {
        match self {
            Self::Flat => [0.0; 10],
            Self::Rock => [5.0, 4.0, 2.0, 0.0, -1.0, 1.0, 3.0, 4.0, 5.0, 5.0],
            Self::Pop => [1.0, 2.0, 3.0, 3.0, 2.0, 0.0, -1.0, 0.0, 1.0, 2.0],
            Self::Jazz => [3.0, 2.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0],
            Self::Classical => [4.0, 3.0, 2.0, 1.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0],
            Self::Bass => [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            Self::Treble => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 5.0, 6.0],
            Self::Vocal => [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0],
        }
    }

    pub const ALL: [EqPreset; 8] = [
        Self::Flat,
        Self::Rock,
        Self::Pop,
        Self::Jazz,
        Self::Classical,
        Self::Bass,
        Self::Treble,
        Self::Vocal,
    ];
}
