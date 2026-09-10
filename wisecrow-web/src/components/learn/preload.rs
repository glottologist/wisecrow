use std::collections::HashSet;

/// One independently loaded media channel.
#[derive(Clone, PartialEq, Debug)]
pub enum Channel<T> {
    Pending,
    Ready(T),
    Failed,
    /// Expected absence (no image query, phrase card, or `image_allowed = false`).
    Absent,
}

/// A fetched card image and the credit its licence requires on display.
#[derive(Clone, PartialEq, Debug)]
pub struct ImageReady {
    pub url: String,
    pub credit: Option<String>,
}

/// Per-card audio and image state. Completions publish independently.
#[derive(Clone, PartialEq, Debug)]
pub struct CardChannels {
    pub audio: Channel<String>,
    pub image: Channel<ImageReady>,
}

impl CardChannels {
    #[must_use]
    pub fn begin(fetch_image: bool) -> Self {
        Self {
            audio: Channel::Pending,
            image: if fetch_image {
                Channel::Pending
            } else {
                Channel::Absent
            },
        }
    }

    pub fn apply_audio(&mut self, result: Result<String, ()>) {
        self.audio = match result {
            Ok(url) => Channel::Ready(url),
            Err(()) => Channel::Failed,
        };
    }

    pub fn apply_image(&mut self, result: Result<Option<ImageReady>, ()>) {
        if matches!(self.image, Channel::Absent) {
            return;
        }
        self.image = match result {
            Ok(Some(image)) => Channel::Ready(image),
            Ok(None) => Channel::Absent,
            Err(()) => Channel::Failed,
        };
    }

    /// Arms a failed audio channel for another fetch. Returns whether to spawn.
    pub fn retry_audio(&mut self) -> bool {
        if matches!(self.audio, Channel::Failed) {
            self.audio = Channel::Pending;
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn audio_url(&self) -> Option<&str> {
        match &self.audio {
            Channel::Ready(url) => Some(url.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn audio_failed(&self) -> bool {
        matches!(self.audio, Channel::Failed)
    }

    #[must_use]
    pub fn image_url(&self) -> Option<&str> {
        match &self.image {
            Channel::Ready(image) => Some(image.url.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn image_credit(&self) -> Option<&str> {
        match &self.image {
            Channel::Ready(image) => image.credit.as_deref(),
            _ => None,
        }
    }
}

pub fn publish_audio(
    media: &mut std::collections::HashMap<usize, CardChannels>,
    index: usize,
    result: Result<String, ()>,
) {
    if let Some(channels) = media.get_mut(&index) {
        channels.apply_audio(result);
    }
}

pub fn publish_image(
    media: &mut std::collections::HashMap<usize, CardChannels>,
    index: usize,
    result: Result<Option<ImageReady>, ()>,
) {
    if let Some(channels) = media.get_mut(&index) {
        channels.apply_image(result);
    }
}

/// Whether a card should request an image. Phrases and abstract words do not.
#[must_use]
pub fn should_fetch_image(is_phrase: bool, image_allowed: bool) -> bool {
    image_allowed && !is_phrase
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
    fn audio_can_be_ready_while_image_is_pending() {
        let mut channels = CardChannels::begin(true);
        channels.apply_audio(Ok("data:audio".to_owned()));
        assert_eq!(channels.audio_url(), Some("data:audio"));
        assert!(matches!(channels.image, Channel::Pending));
        assert!(!channels.audio_failed());
    }

    #[test]
    fn audio_can_be_ready_while_image_failed() {
        let mut channels = CardChannels::begin(true);
        channels.apply_audio(Ok("data:audio".to_owned()));
        channels.apply_image(Err(()));
        assert_eq!(channels.audio_url(), Some("data:audio"));
        assert!(matches!(channels.image, Channel::Failed));
    }

    #[test]
    fn expected_no_image_is_not_an_error() {
        let mut channels = CardChannels::begin(false);
        assert!(matches!(channels.image, Channel::Absent));
        channels.apply_image(Err(()));
        assert!(matches!(channels.image, Channel::Absent));
        channels.apply_image(Ok(None));
        assert!(matches!(channels.image, Channel::Absent));
        assert!(!channels.audio_failed());
    }

    #[test]
    fn server_none_image_becomes_absent_not_failed() {
        let mut channels = CardChannels::begin(true);
        channels.apply_image(Ok(None));
        assert!(matches!(channels.image, Channel::Absent));
        assert!(channels.image_url().is_none());
    }

    #[test]
    fn audio_failure_is_visible_and_retryable() {
        let mut channels = CardChannels::begin(false);
        channels.apply_audio(Err(()));
        assert!(channels.audio_failed());
        assert!(channels.retry_audio());
        assert!(matches!(channels.audio, Channel::Pending));
        channels.apply_audio(Ok("data:audio".to_owned()));
        assert_eq!(channels.audio_url(), Some("data:audio"));
        assert!(!channels.retry_audio());
    }

    #[test]
    fn tracked_indices_are_never_refetched() {
        let tracked: HashSet<usize> = [0, 1, 3].into_iter().collect();
        assert_eq!(indices_to_fetch(0, 50, 5, &tracked), vec![2, 4, 5]);
    }

    #[test]
    fn eviction_keeps_previous_current_and_window() {
        let tracked: HashSet<usize> = (0..12).collect();
        let mut evicted = indices_to_evict(&tracked, 6, 3);
        evicted.sort_unstable();
        assert_eq!(evicted, vec![0, 1, 2, 3, 4, 10, 11]);
    }

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
    fn empty_deck_fetches_nothing() {
        assert!(indices_to_fetch(0, 0, 5, &HashSet::new()).is_empty());
    }

    #[test]
    fn eviction_at_deck_start_keeps_zero() {
        assert!(indices_to_evict(&(0..4).collect(), 0, 5).is_empty());
    }

    #[test]
    fn phrase_and_disallowed_cards_skip_image() {
        assert!(!should_fetch_image(true, true));
        assert!(!should_fetch_image(false, false));
        assert!(should_fetch_image(false, true));
    }
}
