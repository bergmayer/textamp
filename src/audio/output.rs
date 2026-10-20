//! Output selection. An explicitly selected network output never falls back locally.
use super::rodio_backend::{BufferedInput, PipelineEvent, StreamingInput};
use super::{AudioBackend, AudioError, RodioBackend, SampleTap};
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AudioOutput {
    #[default]
    Local,
    Device(String),
    AirPlay(super::airplay::AirplayDestination),
}

impl AudioOutput {
    pub fn label(&self) -> String {
        match self {
            Self::Local => "System default".into(),
            Self::Device(name) => format!("Local: {name}"),
            Self::AirPlay(destination) => format!("AirPlay: {}", destination.name),
        }
    }
}

pub fn local_devices() -> Result<Vec<String>, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let mut names = rodio::cpal::default_host()
        .output_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

pub(crate) enum Backend {
    Local(Box<RodioBackend>),
    #[cfg(unix)]
    AirPlay(Box<super::airplay::AirplayBackend>),
}

macro_rules! delegate {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::Local(backend) => backend.$method($($arg),*),
            #[cfg(unix)]
            Self::AirPlay(backend) => backend.$method($($arg),*),
        }
    };
}

impl Backend {
    pub(crate) fn new(output: AudioOutput) -> Result<Self, AudioError> {
        match output {
            AudioOutput::Local => RodioBackend::new().map(|b| Self::Local(Box::new(b))),
            AudioOutput::Device(name) => {
                RodioBackend::new_for_device(&name).map(|b| Self::Local(Box::new(b)))
            }
            AudioOutput::AirPlay(destination) => {
                #[cfg(unix)]
                {
                    super::airplay::AirplayBackend::new(destination)
                        .map(|b| Self::AirPlay(Box::new(b)))
                }
                #[cfg(not(unix))]
                {
                    let _ = destination;
                    Err(AudioError::PlaybackError(
                        "The AirPlay sender requires macOS or Linux; a native Windows sender is not available yet.".into(),
                    ))
                }
            }
        }
    }
    pub(crate) fn start_buffered(
        &mut self,
        id: u64,
        data: BufferedInput,
        position: Duration,
    ) -> Result<(), AudioError> {
        #[cfg(not(unix))]
        let _ = id;
        match self {
            Self::Local(b) => b.start_buffered(data, position),
            #[cfg(unix)]
            Self::AirPlay(b) => b.start_buffered(id, data, position),
        }
    }
    pub(crate) fn start_stream(
        &mut self,
        id: u64,
        input: StreamingInput,
    ) -> Result<(), AudioError> {
        delegate!(self, start_stream, id, input)
    }
    pub(crate) fn poll_pipeline(&mut self, paused: bool) -> Vec<PipelineEvent> {
        delegate!(self, poll_pipeline, paused)
    }
    pub(crate) fn sample_tap(&self) -> SampleTap {
        delegate!(self, sample_tap)
    }
    pub(crate) fn underrun_count(&self) -> u64 {
        delegate!(self, underrun_count)
    }
    pub(crate) fn volume(&self) -> f32 {
        delegate!(self, volume)
    }
    pub(crate) fn set_volume(&mut self, volume: f32) {
        delegate!(self, set_volume, volume)
    }
    pub(crate) fn pause(&mut self) {
        delegate!(self, pause)
    }
    pub(crate) fn resume(&mut self) {
        delegate!(self, resume)
    }
    pub(crate) fn stop(&mut self) {
        delegate!(self, stop)
    }
    pub(crate) fn seek(&mut self, position: Duration) -> bool {
        delegate!(self, seek, position)
    }
    pub(crate) fn position(&self) -> Option<Duration> {
        delegate!(self, position)
    }
    pub(crate) fn is_finished(&self) -> bool {
        delegate!(self, is_finished)
    }
    pub(crate) fn is_paused(&self) -> bool {
        delegate!(self, is_paused)
    }
    pub(crate) fn is_playing(&self) -> bool {
        delegate!(self, is_playing)
    }
}
