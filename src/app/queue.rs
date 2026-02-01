//! Play queue management.

use crate::api::models::Track;
use rand::seq::SliceRandom;

/// Play queue with history support.
#[derive(Debug, Default)]
pub struct PlayQueue {
    pub tracks: Vec<Track>,
    pub current_index: Option<usize>,
    pub history: Vec<Track>,
}

impl PlayQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the queue.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
        self.history.clear();
    }

    /// Add a track to the end of the queue.
    pub fn add(&mut self, track: Track) {
        self.tracks.push(track);
        if self.current_index.is_none() {
            self.current_index = Some(0);
        }
    }

    /// Add multiple tracks to the queue.
    pub fn add_many(&mut self, tracks: Vec<Track>) {
        let was_empty = self.tracks.is_empty();
        self.tracks.extend(tracks);
        if was_empty && !self.tracks.is_empty() {
            self.current_index = Some(0);
        }
    }

    /// Play a track immediately, clearing the queue.
    pub fn play_now(&mut self, track: Track) {
        self.clear();
        self.tracks.push(track);
        self.current_index = Some(0);
    }

    /// Play multiple tracks immediately, clearing the queue.
    pub fn play_all(&mut self, tracks: Vec<Track>) {
        self.clear();
        if !tracks.is_empty() {
            self.tracks = tracks;
            self.current_index = Some(0);
        }
    }

    /// Get the current track.
    pub fn current(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.tracks.get(idx))
    }

    /// Move to the next track.
    pub fn next(&mut self) -> Option<&Track> {
        if let Some(idx) = self.current_index {
            // Save current to history
            if let Some(track) = self.tracks.get(idx) {
                self.history.push(track.clone());
            }

            if idx + 1 < self.tracks.len() {
                self.current_index = Some(idx + 1);
                return self.tracks.get(idx + 1);
            }
        }
        None
    }

    /// Move to the previous track.
    pub fn previous(&mut self) -> Option<&Track> {
        // First try history
        if let Some(track) = self.history.pop() {
            // Insert at current position and move back
            if let Some(idx) = self.current_index {
                if idx > 0 {
                    self.current_index = Some(idx - 1);
                    return self.tracks.get(idx - 1);
                }
            }
            // Put it back if we can't go back
            self.history.push(track);
        }

        // Otherwise try moving back in the queue
        if let Some(idx) = self.current_index {
            if idx > 0 {
                self.current_index = Some(idx - 1);
                return self.tracks.get(idx - 1);
            }
        }
        None
    }

    /// Shuffle the queue (keeping current track at position 0).
    pub fn shuffle(&mut self) {
        let mut rng = rand::rng();

        if let Some(idx) = self.current_index {
            if let Some(current) = self.tracks.get(idx).cloned() {
                // Remove current track
                self.tracks.remove(idx);
                // Shuffle remaining
                self.tracks.shuffle(&mut rng);
                // Insert current at front
                self.tracks.insert(0, current);
                self.current_index = Some(0);
            }
        } else {
            self.tracks.shuffle(&mut rng);
        }
    }

    /// Remove a track by index.
    pub fn remove(&mut self, index: usize) {
        if index < self.tracks.len() {
            self.tracks.remove(index);

            // Adjust current index
            if let Some(current) = self.current_index {
                if index < current {
                    self.current_index = Some(current - 1);
                } else if index == current {
                    // Current was removed, stay at same index (now next track)
                    if current >= self.tracks.len() && !self.tracks.is_empty() {
                        self.current_index = Some(self.tracks.len() - 1);
                    } else if self.tracks.is_empty() {
                        self.current_index = None;
                    }
                }
            }
        }
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Get queue length.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Check if there's a next track.
    pub fn has_next(&self) -> bool {
        self.current_index
            .map(|idx| idx + 1 < self.tracks.len())
            .unwrap_or(false)
    }

    /// Check if there's a previous track.
    pub fn has_previous(&self) -> bool {
        !self.history.is_empty()
            || self.current_index.map(|idx| idx > 0).unwrap_or(false)
    }
}
