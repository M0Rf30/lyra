// SPDX-License-Identifier: GPL-3.0

//! Streaming sample-rate conversion.
//!
//! Used only as a fallback when the output device cannot natively play the
//! decoded stream's sample rate (for example a hardware-locked 48 kHz device
//! handed a 44.1 kHz-family DSD-to-PCM stream). When the device supports the
//! source rate natively no resampler is created and samples pass through
//! untouched.
//!
//! Backed by `rubato`'s asynchronous resampler. The sinc modes apply a real
//! anti-aliasing filter — essential when downsampling DSD-derived PCM, which
//! carries large ultrasonic shaped noise that would otherwise alias into the
//! audible band — while the `Linear` mode uses cheap polynomial interpolation
//! with no anti-aliasing.
//!
//! Config-driven quality tiers are represented locally by
//! [`ResamplerQuality`] since lyra has no config-file story for this yet.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, PolynomialDegree, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction, calculate_cutoff,
};

/// Resampler quality tiers, trading CPU cost for anti-aliasing accuracy.
///
/// `SincMedium` is the
/// balanced default; `Linear` has no anti-aliasing at all and should only be
/// used when CPU budget is the overriding concern.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ResamplerQuality {
    /// Highest-quality sinc interpolation (largest filter, slowest).
    SincBest,
    /// Balanced sinc interpolation. Default.
    #[default]
    SincMedium,
    /// Fastest sinc interpolation (smallest filter).
    SincFast,
    /// Cheap polynomial interpolation with no anti-aliasing filter.
    Linear,
}

/// Number of input frames fed to the resampler per processing chunk. With a
/// fixed-input async resampler this is also `input_frames_next()`.
const CHUNK_FRAMES: usize = 1024;

/// Streaming, anti-aliased sample-rate converter for interleaved `f32` audio.
pub struct StreamResampler {
    resampler: Async<f32>,
    channels: usize,
    /// Input frames required per `process_into_buffer` call (constant for a
    /// fixed-input async resampler).
    chunk: usize,
    /// Interleaved accumulator of input samples not yet consumed.
    input: Vec<f32>,
    /// Interleaved scratch buffer holding one chunk of resampler output.
    scratch: Vec<f32>,
}

impl StreamResampler {
    /// Create a resampler converting interleaved `channels`-channel audio from
    /// `src_rate` to `dst_rate` at the requested `quality`.
    ///
    /// Returns `None` if the resampler could not be constructed; the caller
    /// should then fall back to passthrough.
    pub fn new(
        src_rate: u32,
        dst_rate: u32,
        channels: usize,
        quality: ResamplerQuality,
    ) -> Option<Self> {
        let channels = channels.max(1);
        let ratio = f64::from(dst_rate.max(1)) / f64::from(src_rate.max(1));

        let resampler = match quality {
            ResamplerQuality::Linear => Async::<f32>::new_poly(
                ratio,
                1.1,
                PolynomialDegree::Linear,
                CHUNK_FRAMES,
                channels,
                FixedAsync::Input,
            )
            .ok()?,
            _ => {
                let params = sinc_params(quality);
                Async::<f32>::new_sinc(
                    ratio,
                    1.1,
                    &params,
                    CHUNK_FRAMES,
                    channels,
                    FixedAsync::Input,
                )
                .ok()?
            }
        };

        let chunk = resampler.input_frames_next();
        let scratch = vec![0.0; resampler.output_frames_max() * channels];

        Some(Self {
            resampler,
            channels,
            chunk,
            input: Vec::new(),
            scratch,
        })
    }

    /// Resample one block of interleaved input, returning interleaved output at
    /// the destination rate. Leftover input (less than one chunk) is carried
    /// across calls so block boundaries stay continuous.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.input.extend_from_slice(input);

        let ch = self.channels;
        let chunk_samples = self.chunk * ch;
        let mut out = Vec::new();

        while self.input.len() >= chunk_samples {
            let indexing = Indexing {
                input_offset: 0,
                output_offset: 0,
                active_channels_mask: None,
                partial_len: None,
            };

            // Borrow three disjoint fields (`input`, `scratch`, `resampler`)
            // inside a block so the adapters release them before we read the
            // output and drain the input below.
            let (nbr_in, nbr_out) = {
                let in_adapter =
                    match InterleavedSlice::new(&self.input[..chunk_samples], ch, self.chunk) {
                        Ok(a) => a,
                        Err(_) => break,
                    };
                let out_cap = self.scratch.len() / ch;
                let mut out_adapter =
                    match InterleavedSlice::new_mut(&mut self.scratch, ch, out_cap) {
                        Ok(a) => a,
                        Err(_) => break,
                    };
                match self.resampler.process_into_buffer(
                    &in_adapter,
                    &mut out_adapter,
                    Some(&indexing),
                ) {
                    Ok(counts) => counts,
                    Err(_) => break,
                }
            };

            // Guard against a pathological zero-consumption result that would
            // otherwise spin forever.
            if nbr_in == 0 {
                break;
            }

            out.extend_from_slice(&self.scratch[..nbr_out * ch]);
            self.input.drain(..nbr_in * ch);
        }

        out
    }
}

/// Map a quality level to rubato sinc interpolation parameters.
fn sinc_params(quality: ResamplerQuality) -> SincInterpolationParameters {
    let (sinc_len, oversampling_factor, interpolation, window) = match quality {
        ResamplerQuality::SincBest => (
            256,
            256,
            SincInterpolationType::Cubic,
            WindowFunction::BlackmanHarris2,
        ),
        ResamplerQuality::SincFast => (
            64,
            128,
            SincInterpolationType::Linear,
            WindowFunction::Hann2,
        ),
        // SincMedium (the default) and the `Linear` fallthrough (which does not
        // call this) use balanced parameters.
        _ => (
            128,
            256,
            SincInterpolationType::Quadratic,
            WindowFunction::Blackman2,
        ),
    };
    SincInterpolationParameters {
        sinc_len,
        f_cutoff: calculate_cutoff(sinc_len, window),
        interpolation,
        oversampling_factor,
        window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_ratio_is_near_identity_length() {
        let mut rs = StreamResampler::new(44100, 44100, 2, ResamplerQuality::SincMedium).unwrap();
        let input = vec![0.0f32; 2048 * 2];
        let out = rs.process(&input);
        // Ratio 1:1, so output length should track input length closely.
        assert!((out.len() as i64 - input.len() as i64).unsigned_abs() < 2048 * 2);
    }

    #[test]
    fn downsampling_shrinks_output() {
        let mut rs = StreamResampler::new(88200, 44100, 2, ResamplerQuality::SincFast).unwrap();
        let input = vec![0.0f32; 4096 * 2];
        let out = rs.process(&input);
        assert!(out.len() < input.len());
    }

    #[test]
    fn linear_quality_constructs() {
        let rs = StreamResampler::new(48000, 44100, 2, ResamplerQuality::Linear);
        assert!(rs.is_some());
    }

    #[test]
    fn leftover_input_carries_across_calls() {
        let mut rs = StreamResampler::new(44100, 48000, 1, ResamplerQuality::SincMedium).unwrap();
        // Feed fewer samples than one chunk; should not panic and may return
        // an empty (or short) result until enough input accumulates.
        let _ = rs.process(&[0.0f32; 10]);
        let out = rs.process(&vec![0.0f32; 4096]);
        assert!(!out.is_empty());
    }
}
