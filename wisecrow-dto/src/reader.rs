//! Types the story reader and its decks return.

use serde::{Deserialize, Serialize};

/// The books a learner can open for one language pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDto {
    pub books: Vec<BookDto>,
}

/// One ready book and, when the learner has one, their place in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookDto {
    pub id: i32,
    pub title: String,
    pub series: String,
    pub cefr: String,
    pub sentences: i32,
    pub readable: bool,
    pub chapter: Option<i32>,
    pub paragraph: Option<i32>,
    pub sentence: Option<i32>,
}

/// One chapter of a readable book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterDto {
    pub book_id: i32,
    pub chapter: i32,
    pub paragraphs: Vec<ParagraphDto>,
}

/// One paragraph, in sentence order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParagraphDto {
    pub paragraph: i32,
    pub sentences: Vec<SentenceDto>,
}

/// A translated sentence and the tokens laid over its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentenceDto {
    pub id: i32,
    pub text: String,
    pub source: String,
    pub tokens: Vec<TokenDto>,
}

/// A tappable span. `pending` is true when no ok gloss exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDto {
    pub surface: String,
    pub normalised: String,
    pub pending: bool,
}

/// A stored note. `status` is `ok` or `pending`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossDto {
    pub status: String,
    pub meaning: Option<String>,
    pub lemma: Option<String>,
    pub class: Option<String>,
    pub note: Option<String>,
}

/// A learner's deck and how many cards it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckDto {
    pub id: i32,
    pub name: String,
    pub cards: u32,
}

/// How many books a learner has started, and how many they have finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BookProgressDto {
    pub started: u32,
    pub finished: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_dtos_round_trip() {
        let chapter = ChapterDto {
            book_id: 4,
            chapter: 1,
            paragraphs: vec![ParagraphDto {
                paragraph: 1,
                sentences: vec![SentenceDto {
                    id: 9,
                    text: "Tom vivía.".into(),
                    source: "Tom lived.".into(),
                    tokens: vec![TokenDto {
                        surface: "vivía".into(),
                        normalised: "vivia".into(),
                        pending: true,
                    }],
                }],
            }],
        };
        let progress = BookProgressDto {
            started: 1,
            finished: 0,
        };
        let chapter_json = serde_json::to_string(&chapter).expect("chapter");
        let progress_json = serde_json::to_string(&progress).expect("progress");
        let restored: ChapterDto = serde_json::from_str(&chapter_json).expect("chapter back");
        let restored_progress: BookProgressDto =
            serde_json::from_str(&progress_json).expect("progress back");
        assert!(restored.paragraphs[0].sentences[0].tokens[0].pending);
        assert_eq!(restored, chapter);
        assert_eq!(restored_progress, progress);
    }
}
