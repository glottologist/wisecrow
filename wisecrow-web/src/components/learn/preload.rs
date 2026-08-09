use std::collections::HashSet;

/// Media for one card, ready before the card is shown.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct CardMedia {
    pub audio_url: Option<String>,
    pub image_url: Option<String>,
    pub image_credit: Option<String>,
}

/// Fetch lifecycle for one index. `Pending` blocks duplicate spawns;
/// `Failed` blocks retry storms (a failed card simply shows no media).
#[derive(Clone, PartialEq, Debug)]
pub enum MediaEntry {
    Pending,
    Ready(CardMedia),
    Failed,
}

/// Indices to fetch when `current` is showing: `current..=current+window`,
/// clipped to the deck, skipping any index already tracked (whatever its
/// state).
#[must_use]
pub fn indices_to_fetch(
    current: usize,
    total: usize,
    window: usize,
    tracked: &HashSet<usize>,
) -> Vec<usize> {
    (current..total.min(current.saturating_add(window).saturating_add(1)))
        .filter(|index| !tracked.contains(index))
        .collect()
}

/// Indices to drop so only `current-1 ..= current+window` stays resident.
#[must_use]
pub fn indices_to_evict(tracked: &HashSet<usize>, current: usize, window: usize) -> Vec<usize> {
    let low = current.saturating_sub(1);
    let high = current.saturating_add(window);
    tracked
        .iter()
        .copied()
        .filter(|index| *index < low || *index > high)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_covers_current_and_next() {
        assert_eq!(
            indices_to_fetch(0, 50, 5, &HashSet::new()),
            vec![0, 1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn window_clips_at_deck_end() {
        assert_eq!(indices_to_fetch(48, 50, 5, &HashSet::new()), vec![48, 49]);
    }

    #[test]
    fn tracked_indices_are_never_refetched() {
        let tracked: HashSet<usize> = [0, 1, 3].into_iter().collect();
        assert_eq!(indices_to_fetch(0, 50, 5, &tracked), vec![2, 4, 5]);
    }

    #[test]
    fn empty_deck_fetches_nothing() {
        assert!(indices_to_fetch(0, 0, 5, &HashSet::new()).is_empty());
    }

    #[test]
    fn eviction_keeps_previous_current_and_window() {
        let tracked: HashSet<usize> = (0..12).collect();
        let mut evicted = indices_to_evict(&tracked, 6, 3);
        evicted.sort_unstable();
        assert_eq!(evicted, vec![0, 1, 2, 3, 4, 10, 11]); // keeps 5..=9
    }

    #[test]
    fn eviction_at_deck_start_keeps_zero() {
        assert!(indices_to_evict(&(0..4).collect(), 0, 5).is_empty());
    }
}
