pub mod feedback;
pub mod scoring;
pub mod session;
pub mod upload;

pub use wisecrow_learning::nback::{
    CompletedTrial, DnbConfig, DnbEngine, DnbMode, DnbVocab, Trial, TrialResponse,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dnb_reexports_shared_contracts() {
        let vocab: Vec<_> = (0..8)
            .map(|id| DnbVocab::new(id, format!("from-{id}"), format!("to-{id}")))
            .collect();
        let engine = DnbEngine::new(vocab, &DnbConfig::default(), 42).expect("valid engine");

        assert_eq!(engine.trial_count(), 0);
    }
}
