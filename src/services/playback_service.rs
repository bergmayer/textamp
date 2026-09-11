//! Queue shuffling. The live queue and history belong to AppState.

use crate::library::models::Track;
use rand::seq::SliceRandom;

pub const MAX_HISTORY_SIZE: usize = 50;

/// Shuffle in place, preserving the current track at the front.
/// A nonempty queue always starts at index zero, including when the supplied
/// index is missing or invalid.
pub fn shuffle_queue(
    mut queue: Vec<Track>,
    queue_index: Option<usize>,
) -> (Vec<Track>, Option<usize>) {
    if queue.is_empty() {
        return (queue, None);
    }

    let shuffle_start = match queue_index.filter(|&index| index < queue.len()) {
        Some(index) => {
            queue.swap(0, index);
            1
        }
        None => 0,
    };
    queue[shuffle_start..].shuffle(&mut rand::rng());
    (queue, Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks() -> Vec<Track> {
        (0..6)
            .map(|id| Track {
                rating_key: id.to_string(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn shuffle_preserves_current_and_every_queue_entry() {
        for index in [None, Some(0), Some(3), Some(5), Some(99)] {
            let (shuffled, current) = shuffle_queue(tracks(), index);
            assert_eq!(current, Some(0));
            if let Some(index) = index.filter(|&i| i < 6) {
                assert_eq!(shuffled[0].rating_key, index.to_string());
            }
            let mut keys: Vec<_> = shuffled.iter().map(|t| t.rating_key.as_str()).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["0", "1", "2", "3", "4", "5"]);
        }
    }

    #[test]
    fn empty_shuffle_has_no_current_track() {
        let (queue, index) = shuffle_queue(Vec::new(), Some(0));
        assert!(queue.is_empty());
        assert_eq!(index, None);
    }
}
