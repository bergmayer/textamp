//! Playback controller.
//!
//! Manages audio playback state and commands.

use super::state::PlayStatus;
use tokio::sync::mpsc;

/// Commands sent to the audio thread.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play(String),      // URL to play
    Pause,
    Resume,
    Stop,
    SetVolume(f32),
    Seek(u64),
}

/// Playback controller that communicates with the audio thread.
pub struct PlaybackController {
    command_tx: mpsc::Sender<AudioCommand>,
    pub status: PlayStatus,
    pub volume: f32,
    pub muted: bool,
    volume_before_mute: f32,
}

impl PlaybackController {
    pub fn new(command_tx: mpsc::Sender<AudioCommand>) -> Self {
        Self {
            command_tx,
            status: PlayStatus::Stopped,
            volume: 0.8,
            muted: false,
            volume_before_mute: 0.8,
        }
    }

    /// Start playing a URL.
    pub async fn play_url(&mut self, url: String) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.status = PlayStatus::Buffering;
        self.command_tx.send(AudioCommand::Play(url)).await
    }

    /// Pause playback.
    pub async fn pause(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.status = PlayStatus::Paused;
        self.command_tx.send(AudioCommand::Pause).await
    }

    /// Resume playback.
    pub async fn resume(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.status = PlayStatus::Playing;
        self.command_tx.send(AudioCommand::Resume).await
    }

    /// Stop playback.
    pub async fn stop(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.status = PlayStatus::Stopped;
        self.command_tx.send(AudioCommand::Stop).await
    }

    /// Toggle play/pause.
    pub async fn toggle(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        match self.status {
            PlayStatus::Playing => self.pause().await,
            PlayStatus::Paused => self.resume().await,
            _ => Ok(()),
        }
    }

    /// Set volume (0.0 to 1.0).
    pub async fn set_volume(&mut self, volume: f32) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.volume = volume.clamp(0.0, 1.0);
        if !self.muted {
            self.command_tx.send(AudioCommand::SetVolume(self.volume)).await
        } else {
            Ok(())
        }
    }

    /// Increase volume by 5%.
    pub async fn volume_up(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.set_volume(self.volume + 0.05).await
    }

    /// Decrease volume by 5%.
    pub async fn volume_down(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.set_volume(self.volume - 0.05).await
    }

    /// Toggle mute.
    pub async fn toggle_mute(&mut self) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        if self.muted {
            self.muted = false;
            self.command_tx.send(AudioCommand::SetVolume(self.volume_before_mute)).await
        } else {
            self.volume_before_mute = self.volume;
            self.muted = true;
            self.command_tx.send(AudioCommand::SetVolume(0.0)).await
        }
    }

    /// Seek to position in milliseconds.
    pub async fn seek(&mut self, position_ms: u64) -> Result<(), mpsc::error::SendError<AudioCommand>> {
        self.command_tx.send(AudioCommand::Seek(position_ms)).await
    }

    /// Check if currently playing.
    pub fn is_playing(&self) -> bool {
        self.status == PlayStatus::Playing
    }

    /// Check if paused.
    pub fn is_paused(&self) -> bool {
        self.status == PlayStatus::Paused
    }

    /// Check if stopped.
    pub fn is_stopped(&self) -> bool {
        self.status == PlayStatus::Stopped
    }
}
