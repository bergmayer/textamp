//! Provider-independent artwork caching and audio visualization.
pub mod analysis;
mod artwork_cache;
pub mod mosaic;
mod spectrogram;
pub mod time_axis;
mod waveform;
pub use artwork_cache::{ARTWORK_LIMIT, ArtworkCache};
pub use spectrogram::{
    SpectrogramCache, SpectrogramData, generate_spectrogram, generate_spectrogram_from_pcm,
};
pub use waveform::{
    WaveformCache, WaveformData, WaveformError, generate_waveform, generate_waveform_from_pcm,
};
