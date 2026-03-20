// SPDX-License-Identifier: GPL-3.0

//! 10-band parametric equalizer DSP implemented as a rodio `Source` adapter.
//!
//! Uses cascaded second-order IIR (biquad) peaking-EQ filters, one per band.
//! Coefficients and bypass state are shared via atomics so the audio thread
//! never blocks on a mutex when the UI adjusts a slider.

use rodio::Source;
use rodio::source::SeekError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use super::equalizer::BAND_FREQUENCIES;

/// Number of EQ bands (fixed at 10).
pub const NUM_BANDS: usize = 10;

/// Bandwidth of each peaking EQ filter in octaves.
const BANDWIDTH_OCTAVES: f32 = 1.0;

// ---------------------------------------------------------------------------
// Biquad coefficients
// ---------------------------------------------------------------------------

/// Second-order IIR (biquad) filter coefficients in Direct Form I.
///
/// Transfer function:
///   H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (a0 + a1·z⁻¹ + a2·z⁻²)
///
/// Stored pre-normalized (divided by a0).
#[derive(Debug, Clone, Copy)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Identity (pass-through) coefficients.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Pack five f32 coefficients into a u64 pair for atomic storage.
    /// We use two u64 values: one holds (b0, b1) and the other (b2, a1, a2-ish).
    /// Actually, we pack all five as compressed f16-ish isn't worth the complexity.
    /// Instead, we use a simpler scheme: each coefficient is stored in its own
    /// atomic slot. But to minimize atomics, we pack b0+b1 into one u64 and
    /// b2+a1+a2 — wait, 5 floats don't fit in 2 u64s nicely.
    ///
    /// Simplest correct approach: store the full coefficient set behind an
    /// `Arc<[AtomicU64; 3]>` per band where we pack two f32s per u64.
    /// u64[0] = (b0, b1), u64[1] = (b2, a1), u64[2] = (a2, _pad).
    fn pack_pair(a: f32, b: f32) -> u64 {
        let a_bits = a.to_bits() as u64;
        let b_bits = b.to_bits() as u64;
        (a_bits << 32) | b_bits
    }

    fn unpack_pair(packed: u64) -> (f32, f32) {
        let a = f32::from_bits((packed >> 32) as u32);
        let b = f32::from_bits(packed as u32);
        (a, b)
    }

    /// Store coefficients into three atomic u64 slots.
    pub fn store(&self, slots: &[AtomicU64; 3]) {
        slots[0].store(Self::pack_pair(self.b0, self.b1), Ordering::Release);
        slots[1].store(Self::pack_pair(self.b2, self.a1), Ordering::Release);
        slots[2].store(Self::pack_pair(self.a2, 0.0), Ordering::Release);
    }

    /// Load coefficients from three atomic u64 slots.
    pub fn load(slots: &[AtomicU64; 3]) -> Self {
        let (b0, b1) = Self::unpack_pair(slots[0].load(Ordering::Acquire));
        let (b2, a1) = Self::unpack_pair(slots[1].load(Ordering::Acquire));
        let (a2, _) = Self::unpack_pair(slots[2].load(Ordering::Acquire));
        Self { b0, b1, b2, a1, a2 }
    }
}

/// Compute peaking EQ biquad coefficients.
///
/// Based on the Audio EQ Cookbook by Robert Bristow-Johnson.
///
/// # Arguments
/// - `freq_hz`: center frequency of the band
/// - `gain_db`: boost/cut in decibels (0 = unity)
/// - `sample_rate`: audio sample rate in Hz
/// - `bandwidth_octaves`: filter bandwidth in octaves
pub fn compute_peaking_eq(
    freq_hz: f32,
    gain_db: f32,
    sample_rate: f32,
    bandwidth_octaves: f32,
) -> BiquadCoeffs {
    if gain_db.abs() < 0.001 {
        return BiquadCoeffs::IDENTITY;
    }

    let a = 10.0_f32.powf(gain_db / 40.0); // sqrt(10^(dB/20))
    let w0 = 2.0 * std::f32::consts::PI * freq_hz / sample_rate;
    let sin_w0 = w0.sin();
    let cos_w0 = w0.cos();
    // alpha = sin(w0) * sinh(ln(2)/2 * BW * w0/sin(w0))
    let alpha = sin_w0 * (2.0_f32.ln() / 2.0 * bandwidth_octaves * w0 / sin_w0).sinh();

    let b0 = 1.0 + alpha * a;
    let b1 = -2.0 * cos_w0;
    let b2 = 1.0 - alpha * a;
    let a0 = 1.0 + alpha / a;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha / a;

    // Normalize by a0
    BiquadCoeffs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

// ---------------------------------------------------------------------------
// Per-band biquad filter state (per channel)
// ---------------------------------------------------------------------------

/// Direct Form II Transposed biquad state for one channel.
#[derive(Default, Clone)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    fn process(&mut self, input: f32, c: &BiquadCoeffs) -> f32 {
        let output = c.b0 * input + self.z1;
        self.z1 = c.b1 * input - c.a1 * output + self.z2;
        self.z2 = c.b2 * input - c.a2 * output;
        output
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Shared coefficients for all 10 bands
// ---------------------------------------------------------------------------

/// Shared atomic storage for EQ coefficients across all bands.
/// Each band uses 3 AtomicU64 slots (see `BiquadCoeffs::store/load`).
pub type SharedCoeffs = Arc<[[AtomicU64; 3]; NUM_BANDS]>;

/// Create a new shared coefficients array initialized to identity.
pub fn new_shared_coeffs() -> SharedCoeffs {
    Arc::new(std::array::from_fn(|_| {
        let slots: [AtomicU64; 3] = std::array::from_fn(|_| AtomicU64::new(0));
        BiquadCoeffs::IDENTITY.store(&slots);
        slots
    }))
}

// ---------------------------------------------------------------------------
// EqController — UI-side handle
// ---------------------------------------------------------------------------

/// Controller for adjusting EQ parameters from the UI thread.
///
/// All operations are lock-free (atomic stores) so they never block audio.
#[derive(Clone)]
pub struct EqController {
    coeffs: SharedCoeffs,
    bypass: Arc<AtomicBool>,
    /// Current sample rate — needed to recompute coefficients.
    sample_rate: f32,
}

impl EqController {
    /// Create a new controller with default sample rate.
    pub fn new(coeffs: SharedCoeffs, bypass: Arc<AtomicBool>, sample_rate: f32) -> Self {
        Self {
            coeffs,
            bypass,
            sample_rate,
        }
    }

    /// Set the gain for a single band and recompute its coefficients.
    pub fn set_band(&self, index: usize, gain_db: f32) {
        if index >= NUM_BANDS {
            return;
        }
        let c = compute_peaking_eq(
            BAND_FREQUENCIES[index],
            gain_db,
            self.sample_rate,
            BANDWIDTH_OCTAVES,
        );
        c.store(&self.coeffs[index]);
    }

    /// Set all 10 bands at once.
    pub fn set_all(&self, gains: &[f32; NUM_BANDS]) {
        for (i, &gain_db) in gains.iter().enumerate() {
            let c = compute_peaking_eq(
                BAND_FREQUENCIES[i],
                gain_db,
                self.sample_rate,
                BANDWIDTH_OCTAVES,
            );
            c.store(&self.coeffs[i]);
        }
    }

    /// Enable or disable (bypass) the EQ.
    pub fn set_enabled(&self, enabled: bool) {
        self.bypass.store(!enabled, Ordering::Release);
    }

    /// Check if EQ is currently enabled.
    pub fn is_enabled(&self) -> bool {
        !self.bypass.load(Ordering::Acquire)
    }

    /// Update sample rate (e.g. when a new track with different rate starts).
    /// This does NOT recompute coefficients — call `set_all` after if needed.
    pub fn set_sample_rate(&mut self, rate: f32) {
        self.sample_rate = rate;
    }
}

// ---------------------------------------------------------------------------
// EqSource — audio-thread Source adapter
// ---------------------------------------------------------------------------

/// A `rodio::Source` adapter that applies a 10-band parametric EQ.
///
/// Reads coefficients from shared atomics (lock-free) and applies cascaded
/// biquad filters to each sample. When bypassed, samples pass through unchanged.
pub struct EqSource<S: Source<Item = f32>> {
    inner: S,
    coeffs: SharedCoeffs,
    bypass: Arc<AtomicBool>,
    /// Per-band, per-channel filter state.
    /// Indexed as `states[band][channel]`.
    states: Vec<Vec<BiquadState>>,
    /// Number of audio channels.
    channels: u16,
    /// Current sample index within a frame (for interleaved channel tracking).
    channel_idx: u16,
}

impl<S: Source<Item = f32>> EqSource<S> {
    /// Wrap a source with the 10-band EQ.
    pub fn new(inner: S, coeffs: SharedCoeffs, bypass: Arc<AtomicBool>) -> Self {
        let channels = inner.channels();
        let states = (0..NUM_BANDS)
            .map(|_| vec![BiquadState::default(); channels as usize])
            .collect();

        Self {
            inner,
            coeffs,
            bypass,
            states,
            channels,
            channel_idx: 0,
        }
    }

    /// Reset all filter states (e.g. after a seek).
    #[allow(dead_code)]
    pub fn reset_states(&mut self) {
        for band in &mut self.states {
            for ch in band.iter_mut() {
                ch.reset();
            }
        }
    }
}

impl<S: Source<Item = f32>> Iterator for EqSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;

        // Bypass: pass through unchanged.
        if self.bypass.load(Ordering::Relaxed) {
            self.channel_idx = (self.channel_idx + 1) % self.channels;
            return Some(sample);
        }

        let ch = self.channel_idx as usize;
        self.channel_idx = (self.channel_idx + 1) % self.channels;

        // Cascade through all 10 bands.
        let mut out = sample;
        for band_idx in 0..NUM_BANDS {
            let c = BiquadCoeffs::load(&self.coeffs[band_idx]);
            out = self.states[band_idx][ch].process(out, &c);
        }

        Some(out)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source<Item = f32>> Source for EqSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_coefficients_pass_through() {
        let c = BiquadCoeffs::IDENTITY;
        let mut state = BiquadState::default();
        // Identity filter should pass samples through unchanged.
        for i in 0..100 {
            let input = (i as f32) * 0.01;
            let output = state.process(input, &c);
            assert!(
                (output - input).abs() < 1e-6,
                "sample {i}: {output} != {input}"
            );
        }
    }

    #[test]
    fn coefficients_pack_unpack_roundtrip() {
        let c = compute_peaking_eq(1000.0, 6.0, 44100.0, 1.0);
        let slots: [AtomicU64; 3] = std::array::from_fn(|_| AtomicU64::new(0));
        c.store(&slots);
        let loaded = BiquadCoeffs::load(&slots);
        assert!((c.b0 - loaded.b0).abs() < 1e-6);
        assert!((c.b1 - loaded.b1).abs() < 1e-6);
        assert!((c.b2 - loaded.b2).abs() < 1e-6);
        assert!((c.a1 - loaded.a1).abs() < 1e-6);
        assert!((c.a2 - loaded.a2).abs() < 1e-6);
    }

    #[test]
    fn zero_gain_produces_identity() {
        let c = compute_peaking_eq(1000.0, 0.0, 44100.0, 1.0);
        assert!((c.b0 - 1.0).abs() < 1e-6);
        assert!(c.b1.abs() < 1e-6);
        assert!(c.b2.abs() < 1e-6);
        assert!(c.a1.abs() < 1e-6);
        assert!(c.a2.abs() < 1e-6);
    }
}
