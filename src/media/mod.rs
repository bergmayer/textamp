//! Provider-independent artwork caching and audio visualization.
pub mod analysis;
mod artwork_cache;
mod spectrogram;
mod waveform;
pub use artwork_cache::ArtworkCache;
pub use spectrogram::{
    generate_spectrogram, generate_spectrogram_from_pcm, SpectrogramCache, SpectrogramData,
};
pub use waveform::{
    generate_waveform, generate_waveform_from_pcm, WaveformCache, WaveformData, WaveformError,
};
