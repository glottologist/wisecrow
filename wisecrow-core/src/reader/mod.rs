//! Graded readers: the shelf and its ledger, extraction, segmentation,
//! translation, word notes, audio and the reading library.
//!
//! The books are English graded readers held on an operator-managed shelf.
//! Nothing here reads a book that its ledger row has not cleared.

pub mod audio;
pub mod extract;
pub mod gloss;
pub mod import;
pub mod library;
pub mod notes;
pub mod segment;
pub mod shelf;
pub mod tokens;
pub mod translate;
