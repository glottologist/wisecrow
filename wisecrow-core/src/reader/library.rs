//! The books a learner can open, and the chapter, note and place they ask for.

use sqlx::PgPool;
use wisecrow_dto::{BookDto, ChapterDto, GlossDto, ParagraphDto, SentenceDto, TokenDto};

use crate::errors::WisecrowError;

/// Ready books that have at least one translation into `foreign`.
///
/// `readable` is true only when every sentence has a translation and no run
/// into that language has failed. The place is the learner's row for the
/// book's file.
///
/// # Errors
///
/// Returns an error when the query fails or a count does not fit the DTO.
pub async fn library_books(
    pool: &PgPool,
    user_id: i32,
    foreign: &str,
) -> Result<Vec<BookDto>, WisecrowError> {
    let rows: Vec<LibraryRow> = sqlx::query_as(
        "SELECT b.id, b.title, b.series, c.code AS cefr,
                (SELECT COUNT(*) FROM book_sentences s WHERE s.book_id = b.id) AS sentences,
                (SELECT COUNT(*) FROM book_translations t
                   JOIN book_sentences s ON s.id = t.sentence_id
                   JOIN languages fl ON fl.id = t.language_id
                  WHERE s.book_id = b.id AND fl.code = $1) AS translated,
                COALESCE((
                    SELECT cardinality(p.failed_runs)
                    FROM book_import_progress p
                    JOIN languages fl ON fl.id = p.language_id
                    WHERE p.book_id = b.id AND fl.code = $1
                ), 0) AS failed,
                rp.chapter, rp.paragraph, rp.sentence
         FROM books b
         JOIN cefr_levels c ON c.id = b.cefr_level_id
         LEFT JOIN reading_positions rp
           ON rp.user_id = $2 AND rp.file = b.file
          AND rp.language_id = b.source_language_id
         WHERE b.status = 'ready'
           AND EXISTS (
               SELECT 1 FROM book_translations t
               JOIN book_sentences s ON s.id = t.sentence_id
               JOIN languages fl ON fl.id = t.language_id
               WHERE s.book_id = b.id AND fl.code = $1
           )
         ORDER BY b.title, b.id",
    )
    .bind(foreign)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(LibraryRow::into_dto).collect()
}

/// One chapter of a readable book. The sentence text is the translation.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] `"this chapter is not readable yet"`
/// when the book is missing or not readable, or a database error.
pub async fn chapter_text(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    book_id: i32,
    chapter: i32,
) -> Result<ChapterDto, WisecrowError> {
    if !book_is_readable(pool, book_id, foreign).await? {
        return Err(WisecrowError::InvalidInput(
            "this chapter is not readable yet".into(),
        ));
    }
    let sentences: Vec<SentenceRow> = sqlx::query_as(
        "SELECT t.id AS translation_id, s.paragraph, t.text, s.text AS source
         FROM book_sentences s
         JOIN book_translations t ON t.sentence_id = s.id
         JOIN languages fl ON fl.id = t.language_id
         WHERE s.book_id = $1 AND s.chapter = $2 AND fl.code = $3
         ORDER BY s.position",
    )
    .bind(book_id)
    .bind(chapter)
    .bind(foreign)
    .fetch_all(pool)
    .await?;
    let translation_ids: Vec<i32> = sentences.iter().map(|row| row.translation_id).collect();
    let tokens = token_rows(pool, &translation_ids).await?;
    let keys: Vec<&str> = tokens.iter().map(|row| row.normalised.as_str()).collect();
    let ready = ready_glosses(pool, native, foreign, &keys).await?;
    let mut paragraphs: Vec<ParagraphDto> = Vec::new();
    for sentence in sentences {
        let sentence_tokens: Vec<TokenDto> = tokens
            .iter()
            .filter(|token| token.translation_id == sentence.translation_id)
            .filter_map(|token| token_dto(&sentence.text, token, &ready))
            .collect();
        let dto = SentenceDto {
            id: sentence.translation_id,
            text: sentence.text,
            source: sentence.source,
            tokens: sentence_tokens,
        };
        match paragraphs.last_mut() {
            Some(paragraph) if paragraph.paragraph == sentence.paragraph => {
                paragraph.sentences.push(dto);
            }
            _ => paragraphs.push(ParagraphDto {
                paragraph: sentence.paragraph,
                sentences: vec![dto],
            }),
        }
    }
    Ok(ChapterDto {
        book_id,
        chapter,
        paragraphs,
    })
}

/// The stored note for a word. A missing or failed row is pending.
///
/// This never calls the model.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn gloss_for(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    normalised: &str,
) -> Result<GlossDto, WisecrowError> {
    let row: Option<GlossRow> = sqlx::query_as(
        "SELECT g.status, g.meaning, g.lemma, g.class, g.note
         FROM book_glosses g
         JOIN languages fl ON fl.id = g.language_id
         JOIN languages nl ON nl.id = g.native_language_id
         WHERE fl.code = $1 AND nl.code = $2 AND g.normalised = $3",
    )
    .bind(foreign)
    .bind(native)
    .bind(normalised)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) if row.status == "ok" => GlossDto {
            status: "ok".into(),
            meaning: row.meaning,
            lemma: row.lemma,
            class: row.class,
            note: row.note,
        },
        _ => GlossDto {
            status: "pending".into(),
            meaning: None,
            lemma: None,
            class: None,
            note: None,
        },
    })
}

/// Records the learner's place in the book's file.
///
/// The language is the book's source language, so a new revision of the same
/// file keeps the place.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] when the book is missing, or a
/// database error.
pub async fn save_reading_position(
    pool: &PgPool,
    user_id: i32,
    book_id: i32,
    chapter: i32,
    paragraph: i32,
    sentence: i32,
) -> Result<(), WisecrowError> {
    let updated = sqlx::query(
        "INSERT INTO reading_positions
             (user_id, file, language_id, chapter, paragraph, sentence)
         SELECT $1, b.file, b.source_language_id, $3, $4, $5
         FROM books b
         WHERE b.id = $2
         ON CONFLICT (user_id, file, language_id)
         DO UPDATE SET chapter = EXCLUDED.chapter,
                       paragraph = EXCLUDED.paragraph,
                       sentence = EXCLUDED.sentence,
                       updated_at = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(book_id)
    .bind(chapter)
    .bind(paragraph)
    .bind(sentence)
    .execute(pool)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(WisecrowError::InvalidInput(
            "that book is not on the shelf".into(),
        ));
    }
    Ok(())
}

async fn book_is_readable(
    pool: &PgPool,
    book_id: i32,
    foreign: &str,
) -> Result<bool, WisecrowError> {
    let row: Option<(i64, i64, i32)> = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM book_sentences s WHERE s.book_id = b.id),
                (SELECT COUNT(*) FROM book_translations t
                   JOIN book_sentences s ON s.id = t.sentence_id
                   JOIN languages fl ON fl.id = t.language_id
                  WHERE s.book_id = b.id AND fl.code = $2),
                COALESCE((
                    SELECT cardinality(p.failed_runs)
                    FROM book_import_progress p
                    JOIN languages fl ON fl.id = p.language_id
                    WHERE p.book_id = b.id AND fl.code = $2
                ), 0)
         FROM books b
         WHERE b.id = $1 AND b.status = 'ready'",
    )
    .bind(book_id)
    .bind(foreign)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some((sentences, translated, failed)) => {
            sentences > 0 && translated == sentences && failed == 0
        }
        None => false,
    })
}

async fn token_rows(
    pool: &PgPool,
    translation_ids: &[i32],
) -> Result<Vec<TokenRow>, WisecrowError> {
    if translation_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT translation_id, start_byte, end_byte, normalised
         FROM book_tokens
         WHERE translation_id = ANY($1)
         ORDER BY translation_id, ordinal",
    )
    .bind(translation_ids)
    .fetch_all(pool)
    .await?)
}

async fn ready_glosses(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    keys: &[&str],
) -> Result<Vec<String>, WisecrowError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_scalar(
        "SELECT g.normalised
         FROM book_glosses g
         JOIN languages fl ON fl.id = g.language_id
         JOIN languages nl ON nl.id = g.native_language_id
         WHERE fl.code = $1 AND nl.code = $2 AND g.status = 'ok'
           AND g.normalised = ANY($3)",
    )
    .bind(foreign)
    .bind(native)
    .bind(keys)
    .fetch_all(pool)
    .await?)
}

fn token_dto(text: &str, token: &TokenRow, ready: &[String]) -> Option<TokenDto> {
    let start = usize::try_from(token.start_byte).ok()?;
    let end = usize::try_from(token.end_byte).ok()?;
    if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    Some(TokenDto {
        surface: text[start..end].to_owned(),
        normalised: token.normalised.clone(), // clone: the token list is scanned once per sentence
        pending: !ready.iter().any(|key| key == &token.normalised),
    })
}

#[derive(sqlx::FromRow)]
struct LibraryRow {
    id: i32,
    title: String,
    series: String,
    cefr: String,
    sentences: i64,
    translated: i64,
    failed: i32,
    chapter: Option<i32>,
    paragraph: Option<i32>,
    sentence: Option<i32>,
}

impl LibraryRow {
    fn into_dto(self) -> Result<BookDto, WisecrowError> {
        let sentences = i32::try_from(self.sentences)
            .map_err(|_| WisecrowError::InvalidInput("a book has too many sentences".into()))?;
        Ok(BookDto {
            id: self.id,
            title: self.title,
            series: self.series,
            cefr: self.cefr,
            sentences,
            readable: self.translated == self.sentences && self.failed == 0,
            chapter: self.chapter,
            paragraph: self.paragraph,
            sentence: self.sentence,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SentenceRow {
    translation_id: i32,
    paragraph: i32,
    text: String,
    source: String,
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    translation_id: i32,
    start_byte: i32,
    end_byte: i32,
    normalised: String,
}

#[derive(sqlx::FromRow)]
struct GlossRow {
    status: String,
    meaning: Option<String>,
    lemma: Option<String>,
    class: Option<String>,
    note: Option<String>,
}
