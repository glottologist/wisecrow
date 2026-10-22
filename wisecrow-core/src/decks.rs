//! A learner's named lists of words taken from a book.

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::srs::scheduler::CardManager;

/// One of a learner's decks for a language pair.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Deck {
    pub id: i32,
    pub name: String,
}

/// Which deck's cards a review may draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckFilter {
    /// Every due card for the pair.
    All,
    /// Only cards whose translation is in this deck.
    Only(i32),
}

impl DeckFilter {
    pub(crate) fn id(self) -> Option<i32> {
        match self {
            Self::All => None,
            Self::Only(id) => Some(id),
        }
    }
}

/// A deck and how many cards it holds.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DeckSummary {
    pub id: i32,
    pub name: String,
    pub cards: i64,
}

/// The card row an added word points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddedWord {
    pub translation_id: i32,
    pub owns_translation: bool,
}

/// Reads and writes `decks` and `deck_cards`.
pub struct DeckRepository;

impl DeckRepository {
    /// The user's decks for one pair, by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn list(
        pool: &PgPool,
        user_id: i32,
        native: &str,
        foreign: &str,
    ) -> Result<Vec<Deck>, WisecrowError> {
        let rows = sqlx::query_as(
            "SELECT id, name FROM decks
             WHERE user_id = $1 AND native_lang = $2 AND foreign_lang = $3
             ORDER BY name, id",
        )
        .bind(user_id)
        .bind(native)
        .bind(foreign)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Creates a deck, or returns the id when this user already has that name.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::InvalidInput`] when the trimmed name is empty,
    /// or a database error.
    pub async fn create(
        pool: &PgPool,
        user_id: i32,
        native: &str,
        foreign: &str,
        name: &str,
    ) -> Result<i32, WisecrowError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(WisecrowError::InvalidInput(
                "a deck name cannot be blank".into(),
            ));
        }
        let id = sqlx::query_scalar(
            "INSERT INTO decks (user_id, name, native_lang, foreign_lang)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, native_lang, foreign_lang, name)
             DO UPDATE SET name = EXCLUDED.name
             RETURNING id",
        )
        .bind(user_id)
        .bind(name)
        .bind(native)
        .bind(foreign)
        .fetch_one(pool)
        .await?;
        Ok(id)
    }

    /// Adds the normalised form to the deck, creating a card for it.
    ///
    /// A new translation is owned by the reader and hides itself from corpus
    /// evidence. A translation that already existed is linked and left as
    /// evidence. No picture is stored.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::InvalidInput`] `"that deck is not yours"` when
    /// the deck is missing or belongs to someone else, or when the form has
    /// no ok note for the deck's pair.
    pub async fn add_word(
        pool: &PgPool,
        user_id: i32,
        deck_id: i32,
        normalised: &str,
    ) -> Result<AddedWord, WisecrowError> {
        let mut transaction = pool.begin().await?;
        let deck: Option<(i32, String, String)> =
            sqlx::query_as("SELECT user_id, native_lang, foreign_lang FROM decks WHERE id = $1")
                .bind(deck_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some((owner, native, foreign)) = deck else {
            return Err(WisecrowError::InvalidInput("that deck is not yours".into()));
        };
        if owner != user_id {
            return Err(WisecrowError::InvalidInput("that deck is not yours".into()));
        }
        let note: Option<(i32, i32, String)> = sqlx::query_as(
            "SELECT g.language_id, g.native_language_id, g.meaning
             FROM book_glosses g
             JOIN languages fl ON fl.id = g.language_id
             JOIN languages nl ON nl.id = g.native_language_id
             WHERE fl.code = $1 AND nl.code = $2 AND g.normalised = $3 AND g.status = 'ok'",
        )
        .bind(&foreign)
        .bind(&native)
        .bind(normalised)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((foreign_id, native_id, meaning)) = note else {
            return Err(WisecrowError::InvalidInput("that word has no note".into()));
        };
        let inserted: Option<i32> = sqlx::query_scalar(
            "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
             DO NOTHING
             RETURNING id",
        )
        .bind(native_id)
        .bind(foreign_id)
        .bind(&meaning)
        .bind(normalised)
        .fetch_optional(&mut *transaction)
        .await?;
        let (translation_id, owns_translation) = match inserted {
            Some(id) => {
                sqlx::query(
                    "INSERT INTO owned_presentations
                         (translation_id, owner, native_face, teachable)
                     VALUES ($1, 'reader', $2, TRUE)",
                )
                .bind(id)
                .bind(&meaning)
                .execute(&mut *transaction)
                .await?;
                (id, true)
            }
            None => {
                let id: i32 = sqlx::query_scalar(
                    "SELECT id FROM translations
                     WHERE from_language_id = $1 AND to_language_id = $2
                       AND from_phrase = $3 AND to_phrase = $4",
                )
                .bind(native_id)
                .bind(foreign_id)
                .bind(&meaning)
                .bind(normalised)
                .fetch_one(&mut *transaction)
                .await?;
                (id, false)
            }
        };
        sqlx::query(
            "INSERT INTO reader_translations
                 (language_id, native_language_id, normalised, translation_id, owns_translation)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (language_id, native_language_id, normalised) DO NOTHING",
        )
        .bind(foreign_id)
        .bind(native_id)
        .bind(normalised)
        .bind(translation_id)
        .bind(owns_translation)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO deck_cards (deck_id, translation_id) VALUES ($1, $2)
             ON CONFLICT (deck_id, translation_id) DO NOTHING",
        )
        .bind(deck_id)
        .bind(translation_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        CardManager::ensure_cards(pool, &[translation_id], user_id).await?;
        Ok(AddedWord {
            translation_id,
            owns_translation,
        })
    }

    /// The user's decks for one pair, with a card count, by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn summaries(
        pool: &PgPool,
        user_id: i32,
        native: &str,
        foreign: &str,
    ) -> Result<Vec<DeckSummary>, WisecrowError> {
        Ok(sqlx::query_as(
            "SELECT d.id, d.name, COUNT(c.translation_id) AS cards
         FROM decks d
         LEFT JOIN deck_cards c ON c.deck_id = d.id
         WHERE d.user_id = $1 AND d.native_lang = $2 AND d.foreign_lang = $3
         GROUP BY d.id, d.name
         ORDER BY d.name, d.id",
        )
        .bind(user_id)
        .bind(native)
        .bind(foreign)
        .fetch_all(pool)
        .await?)
    }

    /// One deck the user owns, with its card count.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::InvalidInput`] `"that deck is not yours"` when
    /// the deck is missing or belongs to someone else, or a database error.
    pub async fn summary(
        pool: &PgPool,
        user_id: i32,
        deck_id: i32,
    ) -> Result<DeckSummary, WisecrowError> {
        let row: Option<DeckSummary> = sqlx::query_as(
            "SELECT d.id, d.name, COUNT(c.translation_id) AS cards
         FROM decks d
         LEFT JOIN deck_cards c ON c.deck_id = d.id
         WHERE d.id = $1 AND d.user_id = $2
         GROUP BY d.id, d.name",
        )
        .bind(deck_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
        row.ok_or_else(|| WisecrowError::InvalidInput("that deck is not yours".into()))
    }

    /// Translation ids in the deck, oldest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn cards(pool: &PgPool, deck_id: i32) -> Result<Vec<i32>, WisecrowError> {
        Ok(sqlx::query_scalar(
            "SELECT translation_id FROM deck_cards
             WHERE deck_id = $1
             ORDER BY added_at, translation_id",
        )
        .bind(deck_id)
        .fetch_all(pool)
        .await?)
    }
}
