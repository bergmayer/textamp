//! Queue shuffling. The live queue and history belong to AppState.

use crate::library::models::Track;
use rand::seq::SliceRandom;

/// Shuffle every queue entry while retaining the playing occurrence's identity.
pub fn shuffle_queue(queue: Vec<Track>, queue_index: Option<usize>) -> (Vec<Track>, Option<usize>) {
    shuffle(queue, queue_index, &mut rand::rng())
}
fn shuffle(
    queue: Vec<Track>,
    queue_index: Option<usize>,
    rng: &mut impl rand::Rng,
) -> (Vec<Track>, Option<usize>) {
    let mut entries: Vec<_> = queue.into_iter().enumerate().collect();
    entries.shuffle(rng);
    let current =
        queue_index.and_then(|index| entries.iter().position(|(original, _)| *original == index));
    (
        entries.into_iter().map(|(_, track)| track).collect(),
        current,
    )
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
                assert_eq!(shuffled[current.unwrap()].rating_key, index.to_string());
            } else {
                assert!(current.is_none());
            }
            let mut keys: Vec<_> = shuffled.iter().map(|t| t.rating_key.as_str()).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["0", "1", "2", "3", "4", "5"]);
        }
    }

    #[test]
    fn playing_track_participates_in_shuffle_without_being_pinned() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let positions: std::collections::BTreeSet<_> = (0..12)
            .map(|_| {
                let (queue, index) = shuffle(tracks(), Some(3), &mut rng);
                assert_eq!(queue[index.unwrap()].rating_key, "3");
                index.unwrap()
            })
            .collect();
        assert!(positions.len() > 1);
    }

    #[test]
    fn empty_shuffle_has_no_current_track() {
        let (queue, index) = shuffle_queue(Vec::new(), Some(0));
        assert!(queue.is_empty());
        assert_eq!(index, None);
    }
}
