//! Bounded whole-track visualizer analysis, independent of the media provider.
//! Time buckets coarsen as recordings grow; frequency resolution is unchanged.
use super::{spectrogram, waveform};
use super::{SpectrogramData, WaveformData};
use std::sync::Arc;

const MAX_BUCKETS: usize = 8192;
const FFT_SIZE: usize = 2048;
const HOP: usize = FFT_SIZE / 2;

/// Power sums in equal-duration buckets. Pairwise merging preserves RMS energy.
#[derive(Default)]
struct Envelope {
    buckets: Vec<f64>,
    stride: usize,
    pending: f64,
    count: usize,
    samples: u64,
}
impl Envelope {
    fn push(&mut self, samples: &[f32]) {
        if self.stride == 0 {
            self.stride = HOP;
        }
        for &sample in samples {
            self.pending += f64::from(sample).powi(2);
            self.count += 1;
            self.samples += 1;
            if self.count == self.stride {
                self.buckets.push(self.pending);
                self.pending = 0.0;
                self.count = 0;
                if self.buckets.len() == MAX_BUCKETS {
                    for i in 0..MAX_BUCKETS / 2 {
                        self.buckets[i] = self.buckets[i * 2] + self.buckets[i * 2 + 1];
                    }
                    self.buckets.truncate(MAX_BUCKETS / 2);
                    self.stride *= 2;
                }
            }
        }
    }
    fn finish(mut self, key: String, duration: u64) -> WaveformData {
        // Integrate bucket overlaps into equally spaced output bins. The final
        // partial bucket uses its actual sample count, not a padded duration.
        if self.count > 0 {
            self.buckets.push(self.pending);
        }
        let n = waveform::DEFAULT_BIN_COUNT;
        let mut bins = vec![0.0; n];
        for (i, bin) in bins.iter_mut().enumerate() {
            let start = i as f64 * self.samples as f64 / n as f64;
            let end = (i + 1) as f64 * self.samples as f64 / n as f64;
            let first = start as usize / self.stride;
            let last = (end.ceil() as usize)
                .div_ceil(self.stride)
                .min(self.buckets.len());
            let mut power = 0.0;
            for j in first..last {
                let a = (j * self.stride) as f64;
                let b = ((j + 1) * self.stride).min(self.samples as usize) as f64;
                let overlap = end.min(b) - start.max(a);
                if overlap > 0.0 {
                    power += self.buckets[j] * overlap / (b - a);
                }
            }
            *bin = (power / (end - start)).sqrt() as f32;
        }
        // The existing constructor provides cache metadata and normalization.
        waveform::generate_waveform_from_pcm(key, duration, &bins)
    }
}

/// Quantized FFT frames with bounded temporal resolution. Max pooling keeps
/// short transients visible when two adjacent time buckets are merged.
struct Spectra {
    frames: Vec<u8>,
    stride: usize,
    count: usize,
    pending: [u8; 96],
    pcm: Vec<f32>,
    rate: u32,
}
impl Default for Spectra {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            stride: 1,
            count: 0,
            pending: [0; 96],
            pcm: Vec::new(),
            rate: 0,
        }
    }
}
impl Spectra {
    fn frame(&mut self, frame: &[u8]) {
        for (peak, value) in self.pending.iter_mut().zip(frame) {
            *peak = (*peak).max(*value);
        }
        self.count += 1;
        if self.count == self.stride {
            self.frames.extend_from_slice(&self.pending);
            self.pending.fill(0);
            self.count = 0;
            if self.frames.len() == MAX_BUCKETS * 96 {
                for i in 0..MAX_BUCKETS / 2 {
                    for b in 0..96 {
                        self.frames[i * 96 + b] =
                            self.frames[i * 192 + b].max(self.frames[i * 192 + 96 + b]);
                    }
                }
                self.frames.truncate(MAX_BUCKETS / 2 * 96);
                self.stride *= 2;
            }
        }
    }
    fn push(&mut self, samples: &[f32], rate: u32) {
        self.rate = rate;
        // Bound scratch PCM even if a decoder supplies an unusually big packet.
        for chunk in samples.chunks(65536) {
            self.pcm.extend_from_slice(chunk);
            if self.pcm.len() >= 65536 {
                self.flush();
            }
        }
    }
    fn flush(&mut self) {
        if self.pcm.len() < FFT_SIZE {
            return;
        }
        let data =
            spectrogram::generate_spectrogram_from_pcm(String::new(), 0, &self.pcm, self.rate);
        for frame in data.frames.chunks_exact(96) {
            self.frame(frame);
        }
        self.pcm.drain(..data.frame_count * HOP);
    }
    fn finish(mut self, key: String, duration: u64) -> SpectrogramData {
        self.flush();
        if self.count > 0 {
            self.frames.extend_from_slice(&self.pending);
        }
        SpectrogramData {
            track_key: key,
            duration_ms: duration,
            bins_per_frame: 96,
            frame_count: self.frames.len() / 96,
            frames_per_second: self.rate as f32 / (HOP * self.stride) as f32,
            sample_rate: self.rate,
            frames: self.frames,
            version: spectrogram::SPECTROGRAM_VERSION,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
}

pub fn analyze_audio(
    key: String,
    duration: u64,
    audio: Arc<[u8]>,
    spectrum: bool,
) -> Result<(WaveformData, Option<SpectrogramData>), waveform::WaveformError> {
    let mut envelope = Envelope::default();
    let mut spectra = spectrum.then(Spectra::default);
    waveform::decode_audio(audio, |samples, rate| {
        envelope.push(samples);
        if let Some(spectra) = &mut spectra {
            spectra.push(samples, rate);
        }
    })?;
    Ok((
        envelope.finish(key.clone(), duration),
        spectra.map(|s| s.finish(key, duration)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_recording_retains_late_audio_with_bounded_storage() {
        let mut envelope = Envelope::default();
        // More than the old 64M mono-sample / 256 MiB decoded ceiling.
        for _ in 0..1100 {
            envelope.push(&[0.0; 65536]);
        }
        envelope.push(&[1.0; 65536]);
        assert!(envelope.buckets.len() < MAX_BUCKETS);
        let wave = envelope.finish("long".into(), 1);
        assert_eq!(wave.bins.len(), 1000);
        assert_eq!(wave.bins[0], 0.0);
        assert!(wave.bins[999] > 0.99);
    }
    #[test]
    fn streaming_spectrum_matches_whole_pcm_across_packet_boundaries() {
        let samples: Vec<f32> = (0..22050).map(|i| (i as f32 * 0.13).sin()).collect();
        let expected = spectrogram::generate_spectrogram_from_pcm("x".into(), 500, &samples, 44100);
        let mut streamed = Spectra::default();
        for chunk in samples.chunks(713) {
            streamed.push(chunk, 44100);
        }
        assert_eq!(streamed.finish("x".into(), 500).frames, expected.frames);
    }
    #[test]
    fn spectral_compaction_keeps_late_transients_and_frequency_bins() {
        let mut s = Spectra::default();
        for _ in 0..MAX_BUCKETS * 9 {
            s.frame(&[3; 96]);
        }
        let mut peak = [0; 96];
        peak[80] = 255;
        s.frame(&peak);
        assert!(s.frames.len() < MAX_BUCKETS * 96);
        let data = s.finish("x".into(), 1);
        assert_eq!(data.frames.last().copied(), Some(0));
        assert_eq!(data.spectrum_at(data.frame_count - 1)[80], 255);
    }
}
