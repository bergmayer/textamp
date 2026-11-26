//! Queue shuffling. The live queue and history belong to AppState.

use crate::library::models::Track;
use rand::seq::SliceRandom;

/// Keep the playing occurrence (or the first pending entry) at the head.
pub fn shuffle_queue(
    mut queue: Vec<Track>,
    queue_index: Option<usize>,
) -> (Vec<Track>, Option<usize>) {
    let current = queue_index.filter(|&index| index < queue.len());
    if let Some(index) = current {
        queue.swap(0, index);
    }
    if let Some((_, upcoming)) = queue.split_first_mut() {
        upcoming.shuffle(&mut rand::rng());
    }
    (queue, current.map(|_| 0))
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
            if let Some(index) = index.filter(|&i| i < 6) {
                assert_eq!(current, Some(0));
                assert_eq!(shuffled[0].rating_key, index.to_string());
            } else {
                assert!(current.is_none());
                assert_eq!(shuffled[0].rating_key, "0");
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
