//! Playback service for queue and track management.
//!
//! This service handles the business logic for:
//! - Queue management (add, remove, clear, reorder)
//! - Track navigation (next, previous)
//! - Play history tracking
//! - Playback mode transitions (queue vs radio)
//!
//! # Cross-Platform Design
//!
//! This service is UI-agnostic and can be used with any frontend:
//! - TUI (ratatui)
//! - iOS (SwiftUI)
//! - Web (Svelte/React)

use std::collections::VecDeque;

use crate::api::models::Track;

/// Maximum number of tracks in play history.
pub const MAX_HISTORY_SIZE: usize = 50;

/// Result of a navigation action (next/previous).
#[derive(Debug, Clone)]
pub enum NavigationResult {
    /// Successfully moved to a new track.
    Track(Track),
    /// Reached the end of the queue (no more tracks).
    EndOfQueue,
    /// No tracks available.
    Empty,
}

/// Queue manager that owns queue state and enforces invariants.
///
/// This encapsulates the queue tracks, current index, and history,
/// ensuring that the index is always valid relative to the queue.
#[derive(Debug, Clone, Default)]
pub struct QueueManager {
    tracks: Vec<Track>,
    current_index: Option<usize>,
    history: VecDeque<Track>,
}

impl QueueManager {
    /// Create a new empty queue manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current queue of tracks.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Get mutable access to tracks (use sparingly).
    pub fn tracks_mut(&mut self) -> &mut Vec<Track> {
        &mut self.tracks
    }

    /// Get the current index.
    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Set the current index (clamped to valid range).
    pub fn set_index(&mut self, index: Option<usize>) {
        self.current_index = index.map(|i| i.min(self.tracks.len().saturating_sub(1)));
        if self.tracks.is_empty() {
            self.current_index = None;
        }
    }

    /// Get the play history.
    pub fn history(&self) -> &VecDeque<Track> {
        &self.history
    }

    /// Get the current track.
    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|idx| self.tracks.get(idx))
    }

    /// Get queue length.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Get remaining tracks count (from current position to end).
    pub fn remaining(&self) -> usize {
        match self.current_index {
            Some(idx) => self.tracks.len().saturating_sub(idx + 1),
            None => self.tracks.len(),
        }
    }

    /// Add a single track to the end of the queue.
    pub fn enqueue(&mut self, track: Track) -> bool {
        self.tracks.push(track);
        true
    }

    /// Add multiple tracks to the end of the queue.
    ///
    /// Returns the number of tracks actually added.
    pub fn enqueue_many(&mut self, tracks: Vec<Track>) -> usize {
        let added = tracks.len();
        self.tracks.extend(tracks);
        added
    }

    /// Remove a track from the queue by index.
    ///
    /// Returns the removed track if the index was valid.
    pub fn remove(&mut self, index: usize) -> Option<Track> {
        if index >= self.tracks.len() {
            return None;
        }

        let removed = self.tracks.remove(index);

        // Adjust current index if needed
        if let Some(current) = self.current_index {
            if index < current {
                self.current_index = Some(current - 1);
            } else if index == current && current >= self.tracks.len() {
                self.current_index = if self.tracks.is_empty() {
                    None
                } else {
                    Some(self.tracks.len() - 1)
                };
            }
        }

        Some(removed)
    }

    /// Clear the entire queue.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
    }

    /// Move to the next track.
    pub fn next(&mut self) -> NavigationResult {
        if self.tracks.is_empty() {
            return NavigationResult::Empty;
        }

        let current_idx = self.current_index.unwrap_or(0);

        // Add current track to history before moving
        if let Some(track) = self.tracks.get(current_idx) {
            self.add_to_history(track.clone());
        }

        let next_idx = current_idx + 1;
        if next_idx >= self.tracks.len() {
            return NavigationResult::EndOfQueue;
        }

        self.current_index = Some(next_idx);

        match self.tracks.get(next_idx) {
            Some(track) => NavigationResult::Track(track.clone()),
            None => NavigationResult::EndOfQueue,
        }
    }

    /// Move to the previous track.
    pub fn previous(&mut self) -> NavigationResult {
        if self.tracks.is_empty() {
            return NavigationResult::Empty;
        }

        let current_idx = self.current_index.unwrap_or(0);

        if current_idx > 0 {
            let prev_idx = current_idx - 1;
            self.current_index = Some(prev_idx);

            match self.tracks.get(prev_idx) {
                Some(track) => NavigationResult::Track(track.clone()),
                None => NavigationResult::Empty,
            }
        } else {
            // At beginning, stay at start
            match self.tracks.first() {
                Some(track) => NavigationResult::Track(track.clone()),
                None => NavigationResult::Empty,
            }
        }
    }

    /// Replace the queue with new tracks and start from the beginning.
    ///
    /// Returns the first track to play, if any.
    pub fn replace(&mut self, tracks: Vec<Track>) -> Option<Track> {
        self.tracks.clear();
        let added = self.enqueue_many(tracks);

        if added > 0 {
            self.current_index = Some(0);
            self.tracks.first().cloned()
        } else {
            self.current_index = None;
            None
        }
    }

    /// Insert a track to play next (after current position).
    pub fn play_next(&mut self, track: Track) -> bool {
        let insert_pos = self.current_index.map(|idx| idx + 1).unwrap_or(0);
        self.tracks.insert(insert_pos.min(self.tracks.len()), track);
        true
    }

    /// Add a track to play history.
    fn add_to_history(&mut self, track: Track) {
        // Don't add duplicates of the most recent track
        if self.history.back().map(|t| &t.rating_key) == Some(&track.rating_key) {
            return;
        }

        self.history.push_back(track);

        // Trim to max size
        while self.history.len() > MAX_HISTORY_SIZE {
            self.history.pop_front();
        }
    }

    /// Clear history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Extend queue with additional tracks.
    pub fn extend(&mut self, tracks: impl IntoIterator<Item = Track>) {
        self.tracks.extend(tracks);
    }

    /// Get track keys for creating a playlist.
    pub fn track_keys(&self) -> Vec<String> {
        self.tracks.iter().map(|t| t.rating_key.clone()).collect()
    }
}

/// Playback service with static helper methods.
///
/// For backward compatibility with existing code that uses static methods.
pub struct PlaybackService;

impl PlaybackService {
    /// Add a single track to the end of the queue.
    pub fn enqueue_track(queue: &mut Vec<Track>, track: Track) -> bool {
        queue.push(track);
        true
    }

    /// Add multiple tracks to the end of the queue.
    pub fn enqueue_tracks(queue: &mut Vec<Track>, tracks: Vec<Track>) -> usize {
        let added = tracks.len();
        queue.extend(tracks);
        added
    }

    /// Remove a track from the queue by index.
    pub fn remove_from_queue(
        queue: &mut Vec<Track>,
        queue_index: &mut Option<usize>,
        remove_idx: usize,
    ) -> Option<Track> {
        if remove_idx >= queue.len() {
            return None;
        }

        let removed = queue.remove(remove_idx);

        if let Some(current) = *queue_index {
            if remove_idx < current {
                *queue_index = Some(current - 1);
            } else if remove_idx == current && current >= queue.len() {
                *queue_index = if queue.is_empty() {
                    None
                } else {
                    Some(queue.len() - 1)
                };
            }
        }

        Some(removed)
    }

    /// Clear the entire queue.
    pub fn clear_queue(queue: &mut Vec<Track>, queue_index: &mut Option<usize>) {
        queue.clear();
        *queue_index = None;
    }

    /// Move to the next track in the queue.
    pub fn next_track(
        queue: &[Track],
        queue_index: &mut Option<usize>,
        history: &mut VecDeque<Track>,
    ) -> NavigationResult {
        if queue.is_empty() {
            return NavigationResult::Empty;
        }

        let current_idx = queue_index.unwrap_or(0);

        if let Some(track) = queue.get(current_idx) {
            Self::add_to_history(history, track.clone());
        }

        let next_idx = current_idx + 1;
        if next_idx >= queue.len() {
            return NavigationResult::EndOfQueue;
        }

        *queue_index = Some(next_idx);

        match queue.get(next_idx) {
            Some(track) => NavigationResult::Track(track.clone()),
            None => NavigationResult::EndOfQueue,
        }
    }

    /// Move to the previous track.
    pub fn previous_track(
        queue: &[Track],
        queue_index: &mut Option<usize>,
        _history: &mut VecDeque<Track>,
    ) -> NavigationResult {
        if queue.is_empty() {
            return NavigationResult::Empty;
        }

        let current_idx = queue_index.unwrap_or(0);

        if current_idx > 0 {
            let prev_idx = current_idx - 1;
            *queue_index = Some(prev_idx);

            match queue.get(prev_idx) {
                Some(track) => NavigationResult::Track(track.clone()),
                None => NavigationResult::Empty,
            }
        } else {
            match queue.first() {
                Some(track) => NavigationResult::Track(track.clone()),
                None => NavigationResult::Empty,
            }
        }
    }

    /// Get the current track without changing position.
    pub fn current_track(queue: &[Track], queue_index: Option<usize>) -> Option<&Track> {
        queue_index.and_then(|idx| queue.get(idx))
    }

    /// Add a track to play history.
    pub fn add_to_history(history: &mut VecDeque<Track>, track: Track) {
        if history.back().map(|t| &t.rating_key) == Some(&track.rating_key) {
            return;
        }

        history.push_back(track);

        while history.len() > MAX_HISTORY_SIZE {
            history.pop_front();
        }
    }

    /// Replace the queue with new tracks.
    pub fn replace_queue(
        queue: &mut Vec<Track>,
        queue_index: &mut Option<usize>,
        tracks: Vec<Track>,
    ) -> Option<Track> {
        queue.clear();
        let added = Self::enqueue_tracks(queue, tracks);

        if added > 0 {
            *queue_index = Some(0);
            queue.first().cloned()
        } else {
            *queue_index = None;
            None
        }
    }

    /// Insert a track to play next.
    pub fn play_next(queue: &mut Vec<Track>, queue_index: Option<usize>, track: Track) -> bool {
        let insert_pos = queue_index.map(|idx| idx + 1).unwrap_or(0);
        queue.insert(insert_pos.min(queue.len()), track);
        true
    }

    /// Get queue length.
    pub fn queue_len(queue: &[Track]) -> usize {
        queue.len()
    }

    /// Check if queue is empty.
    pub fn is_queue_empty(queue: &[Track]) -> bool {
        queue.is_empty()
    }

    /// Get remaining tracks count.
    pub fn remaining_tracks(queue: &[Track], queue_index: Option<usize>) -> usize {
        match queue_index {
            Some(idx) => queue.len().saturating_sub(idx + 1),
            None => queue.len(),
        }
    }

    /// Shuffle the queue, keeping the current track at position 0.
    ///
    /// Returns the new queue and the new index (always 0 if there was a current track).
    pub fn shuffle_queue(
        queue: Vec<Track>,
        queue_index: Option<usize>,
    ) -> (Vec<Track>, Option<usize>) {
        use rand::seq::SliceRandom;
        use rand::rng;

        if queue.is_empty() {
            return (queue, None);
        }

        let mut rng = rng();

        // If we have a current track, keep it at position 0 and shuffle the rest
        match queue_index {
            Some(current_idx) if current_idx < queue.len() => {
                let current_track = queue[current_idx].clone();
                let mut rest: Vec<Track> = queue
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| *i != current_idx)
                    .map(|(_, t)| t)
                    .collect();
                rest.shuffle(&mut rng);

                let mut result = vec![current_track];
                result.extend(rest);
                (result, Some(0))
            }
            _ => {
                // No current track, shuffle everything
                let mut shuffled = queue;
                shuffled.shuffle(&mut rng);
                (shuffled, Some(0))
            }
        }
    }

    /// Build a queue from a track list starting at a specific index.
    ///
    /// This is useful when the user selects a track from a list and wants
    /// to play from there, including all subsequent tracks.
    pub fn build_queue_from_list(
        tracks: &[Track],
        start_index: usize,
    ) -> (Vec<Track>, Option<usize>) {
        if tracks.is_empty() || start_index >= tracks.len() {
            return (Vec::new(), None);
        }

        // Include all tracks from start_index onwards
        let queue: Vec<Track> = tracks[start_index..].to_vec();
        (queue, Some(0))
    }

    /// Build a queue with all tracks, positioning at the specified index.
    ///
    /// This keeps all tracks but starts playback at the given position.
    pub fn build_queue_all_tracks(
        tracks: &[Track],
        start_index: usize,
    ) -> (Vec<Track>, Option<usize>) {
        if tracks.is_empty() {
            return (Vec::new(), None);
        }

        let queue = tracks.to_vec();
        let index = start_index.min(tracks.len().saturating_sub(1));
        (queue, Some(index))
    }

    /// Get the total duration of all tracks in the queue.
    pub fn total_duration_ms(queue: &[Track]) -> u64 {
        queue.iter().map(|t| t.duration_ms()).sum()
    }

    /// Get the remaining duration from current position to end of queue.
    pub fn remaining_duration_ms(queue: &[Track], queue_index: Option<usize>, position_ms: u64) -> u64 {
        let Some(idx) = queue_index else {
            return Self::total_duration_ms(queue);
        };

        let Some(current) = queue.get(idx) else {
            return 0;
        };

        // Remaining of current track
        let current_remaining = current.duration_ms().saturating_sub(position_ms);

        // Duration of subsequent tracks
        let subsequent: u64 = queue.iter().skip(idx + 1).map(|t| t.duration_ms()).sum();

        current_remaining + subsequent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(key: &str, title: &str) -> Track {
        Track {
            rating_key: key.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_enqueue_track() {
        let mut queue = Vec::new();
        let track = make_track("1", "Track 1");

        assert!(PlaybackService::enqueue_track(&mut queue, track));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].title, "Track 1");
    }

    #[test]
    fn test_next_track() {
        let queue = vec![
            make_track("1", "Track 1"),
            make_track("2", "Track 2"),
            make_track("3", "Track 3"),
        ];
        let mut queue_index = Some(0);
        let mut history = VecDeque::new();

        match PlaybackService::next_track(&queue, &mut queue_index, &mut history) {
            NavigationResult::Track(t) => assert_eq!(t.title, "Track 2"),
            _ => panic!("Expected Track"),
        }
        assert_eq!(queue_index, Some(1));
        assert_eq!(history.len(), 1);

        match PlaybackService::next_track(&queue, &mut queue_index, &mut history) {
            NavigationResult::Track(t) => assert_eq!(t.title, "Track 3"),
            _ => panic!("Expected Track"),
        }
        assert_eq!(queue_index, Some(2));

        match PlaybackService::next_track(&queue, &mut queue_index, &mut history) {
            NavigationResult::EndOfQueue => {}
            _ => panic!("Expected EndOfQueue"),
        }
    }

    #[test]
    fn test_remove_from_queue() {
        let mut queue = vec![
            make_track("1", "Track 1"),
            make_track("2", "Track 2"),
            make_track("3", "Track 3"),
        ];
        let mut queue_index = Some(1);

        let removed = PlaybackService::remove_from_queue(&mut queue, &mut queue_index, 0);
        assert!(removed.is_some());
        assert_eq!(queue_index, Some(0));

        let removed = PlaybackService::remove_from_queue(&mut queue, &mut queue_index, 0);
        assert!(removed.is_some());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn test_replace_queue() {
        let mut queue = vec![make_track("old", "Old Track")];
        let mut queue_index = Some(0);

        let new_tracks = vec![
            make_track("1", "New 1"),
            make_track("2", "New 2"),
        ];

        let first = PlaybackService::replace_queue(&mut queue, &mut queue_index, new_tracks);
        assert!(first.is_some());
        assert_eq!(first.unwrap().title, "New 1");
        assert_eq!(queue.len(), 2);
        assert_eq!(queue_index, Some(0));
    }

    #[test]
    fn test_queue_manager_enqueue() {
        let mut qm = QueueManager::new();
        let track = make_track("1", "Track 1");

        assert!(qm.enqueue(track));
        assert_eq!(qm.len(), 1);
        assert_eq!(qm.tracks()[0].title, "Track 1");
    }

    #[test]
    fn test_queue_manager_next() {
        let mut qm = QueueManager::new();
        qm.enqueue(make_track("1", "Track 1"));
        qm.enqueue(make_track("2", "Track 2"));
        qm.set_index(Some(0));

        match qm.next() {
            NavigationResult::Track(t) => assert_eq!(t.title, "Track 2"),
            _ => panic!("Expected Track"),
        }
        assert_eq!(qm.current_index(), Some(1));
        assert_eq!(qm.history().len(), 1);
    }

    #[test]
    fn test_queue_manager_remove() {
        let mut qm = QueueManager::new();
        qm.enqueue(make_track("1", "Track 1"));
        qm.enqueue(make_track("2", "Track 2"));
        qm.set_index(Some(1));

        let removed = qm.remove(0);
        assert!(removed.is_some());
        assert_eq!(qm.current_index(), Some(0));
        assert_eq!(qm.len(), 1);
    }
}
