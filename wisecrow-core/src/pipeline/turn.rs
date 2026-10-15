//! Which corpus step runs next. This module does not touch the database.

/// What one language pair still needs before the gloss and audio cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairNeed {
    pub archive: bool,
    pub rank: bool,
    pub mine: bool,
}

/// The rotating step inside the cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Gloss,
    PhraseAudio,
    WordAudio,
}

/// The single step one invocation should take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Archive,
    Rank,
    Mine,
    Cycle(Turn),
}

/// Archives beat ranking, ranking beats a mine, and a mine beats the cycle.
#[must_use]
pub fn choose(need: PairNeed, turn: Turn) -> Step {
    if need.archive {
        Step::Archive
    } else if need.rank {
        Step::Rank
    } else if need.mine {
        Step::Mine
    } else {
        Step::Cycle(turn)
    }
}

/// Gloss, then phrase audio, then word audio, then gloss again.
#[must_use]
pub fn advance(turn: Turn) -> Turn {
    match turn {
        Turn::Gloss => Turn::PhraseAudio,
        Turn::PhraseAudio => Turn::WordAudio,
        Turn::WordAudio => Turn::Gloss,
    }
}

/// How a cycle step finished. The host sleeps on [`Attempt::Retry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// The step wrote something.
    Worked,
    /// This kind had nothing to do.
    Empty,
    /// Try again later. The kind still moves on.
    Retry,
}

/// The kind stored for the next wake.
///
/// A retry and an empty step both move forward. The caller returns on a retry
/// and tries the next kind on an empty step.
#[must_use]
pub fn settle(turn: Turn, attempt: Attempt) -> (Turn, Attempt) {
    (advance(turn), attempt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_beats_a_mine_and_a_retry_moves_off_word_audio() {
        assert_eq!(
            choose(
                PairNeed {
                    archive: true,
                    rank: true,
                    mine: true
                },
                Turn::Gloss
            ),
            Step::Archive
        );
        assert_eq!(
            choose(
                PairNeed {
                    archive: false,
                    rank: true,
                    mine: true
                },
                Turn::PhraseAudio
            ),
            Step::Rank
        );
        assert_eq!(
            choose(
                PairNeed {
                    archive: false,
                    rank: false,
                    mine: true
                },
                Turn::WordAudio
            ),
            Step::Mine
        );
        assert_eq!(advance(Turn::WordAudio), Turn::Gloss);
        assert_eq!(advance(Turn::Gloss), Turn::PhraseAudio);
        assert_eq!(advance(Turn::PhraseAudio), Turn::WordAudio);
    }

    #[test]
    fn retry_advances_before_the_host_sleeps() {
        let (next, step) = settle(Turn::WordAudio, Attempt::Retry);
        assert_eq!(next, Turn::Gloss);
        assert_eq!(step, Attempt::Retry);
        let (next, step) = settle(Turn::Gloss, Attempt::Empty);
        assert_eq!(next, Turn::PhraseAudio);
        assert_eq!(step, Attempt::Empty);
    }
}
