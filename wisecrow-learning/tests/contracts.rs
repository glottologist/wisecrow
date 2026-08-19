use chrono::Utc;
use std::sync::Arc;
use wisecrow_learning::nback::{DnbConfig, DnbEngine, DnbMode, DnbVocab};
use wisecrow_learning::srs::{CardState, CardStatus, FsrsScheduler, Scheduler};

#[test]
fn public_contracts_construct_from_borrowed_or_convertible_inputs() {
    let scheduler: &dyn Scheduler = &FsrsScheduler;
    let card = CardState::new(7, Utc::now(), CardStatus::New);
    assert_eq!(scheduler.replay(&card, &[]).expect("empty replay"), card);

    let vocab: Arc<[DnbVocab]> = vec![
        DnbVocab::new(1, "one", "eins"),
        DnbVocab::new(2, "two", "zwei"),
        DnbVocab::new(3, "three", "drei"),
        DnbVocab::new(4, "four", "vier"),
        DnbVocab::new(5, "five", "fuenf"),
        DnbVocab::new(6, "six", "sechs"),
        DnbVocab::new(7, "seven", "sieben"),
        DnbVocab::new(8, "eight", "acht"),
    ]
    .into();
    let config = DnbConfig::new(DnbMode::AudioWritten, 2, 4_000);
    let engine = DnbEngine::new(vocab, &config, 42).expect("valid engine");
    assert_eq!(engine.trial_count(), 0);
}
