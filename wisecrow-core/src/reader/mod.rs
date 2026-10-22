//! Graded readers: the shelf and its ledger, extraction, segmentation,
//! translation, word notes, audio and the reading library.
//!
//! The books are the PDFs on the shelf. The pair file chooses the languages.
//! A `Translate` cell of `no` is the only hold.

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
