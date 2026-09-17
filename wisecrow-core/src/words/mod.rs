//! Sentence-derived word promotion.
//!
//! A frequent word that appears only inside corpus sentences becomes a
//! learning card in three bounded steps: extraction counts words across the
//! evidence view and stores candidates with a few source examples; promotion
//! asks the model for a canonical presentation with those examples as
//! context; storage links each candidate to a translation row under a stable
//! ID. Generated rows are never counted as corpus evidence again.

mod extract;
mod promote;
mod store;

pub use extract::extract_words;
pub use promote::promote_words;

use crate::errors::WisecrowError;

/// Finite target and evidence threshold for one extraction run.
#[derive(Debug)]
pub struct ExtractionOptions {
    pub(crate) limit: u32,
    pub(crate) min_occurrences: u32,
}

impl ExtractionOptions {
    /// Validates a finite candidate target and evidence threshold.
    ///
    /// # Errors
    ///
    /// Rejects limits outside 1–10,000 and thresholds below two.
    pub fn new(limit: u32, min_occurrences: u32) -> Result<Self, WisecrowError> {
        if !(1..=10_000).contains(&limit) || min_occurrences < 2 {
            return Err(WisecrowError::InvalidInput(
                "Invalid word extraction limits".into(),
            ));
        }
        Ok(Self {
            limit,
            min_occurrences,
        })
    }
}

/// Which candidates a promotion run attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Only candidates never attempted.
    Pending,
    /// Pending candidates and those whose last attempt failed.
    RetryFailed,
    /// Every candidate, refreshing accepted and rejected presentations.
    All,
}

/// Bound on the candidates one promotion run attempts.
#[derive(Debug)]
pub struct PromotionOptions {
    pub(crate) limit: u32,
    pub(crate) refresh: Refresh,
}

impl PromotionOptions {
    /// Validates the maximum number of attempted candidates.
    ///
    /// # Errors
    ///
    /// Rejects limits outside 1–1,000.
    pub fn new(limit: u32, refresh: Refresh) -> Result<Self, WisecrowError> {
        if !(1..=1000).contains(&limit) {
            return Err(WisecrowError::InvalidInput(
                "Invalid word promotion limit".into(),
            ));
        }
        Ok(Self { limit, refresh })
    }
}

/// What one extraction run scanned and published.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractionSummary {
    pub scanned_rows: u64,
    pub candidates: u64,
}

/// Outcome counts of one promotion run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PromotionSummary {
    pub attempted: u32,
    pub accepted: u32,
    pub rejected: u32,
    pub failed: u32,
    pub stale: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(0, 5, false)]
    #[case(10_001, 5, false)]
    #[case(500, 1, false)]
    #[case(500, 5, true)]
    #[case(1, 2, true)]
    #[case(10_000, 2, true)]
    fn extraction_options_are_bounded(
        #[case] limit: u32,
        #[case] min_occurrences: u32,
        #[case] accepted: bool,
    ) {
        assert_eq!(
            ExtractionOptions::new(limit, min_occurrences).is_ok(),
            accepted
        );
    }

    #[rstest]
    #[case(0, false)]
    #[case(1001, false)]
    #[case(1, true)]
    #[case(1000, true)]
    fn promotion_options_are_bounded(#[case] limit: u32, #[case] accepted: bool) {
        assert_eq!(
            PromotionOptions::new(limit, Refresh::Pending).is_ok(),
            accepted
        );
    }
}
