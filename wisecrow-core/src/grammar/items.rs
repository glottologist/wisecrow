//! The quiz item bank.
//!
//! Generated items are persisted rather than returned and forgotten, so that
//! generation accumulates instead of being paid for on every request. Every
//! item passes structural gates on the way in, and a human promotion step on
//! the way out: a subtly broken item does not merely teach the wrong thing, it
//! writes a false signal into the learner's mastery and causes the selection
//! algorithm to drill a point they already know.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use wisecrow_learning::grading::GradableItem;

use crate::errors::WisecrowError;

/// Marker a cloze sentence uses for the space the learner fills.
pub const BLANK: &str = "___";

/// Fewest options a multiple-choice item may offer.
const MIN_OPTIONS: usize = 3;

/// An item as generated, before it has been stored or gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemDraft {
    Cloze {
        sentence: String,
        answer: String,
        accepted: Vec<String>,
        hint: Option<String>,
    },
    MultipleChoice {
        question: String,
        options: Vec<(String, String)>,
        correct_option: String,
        hint: Option<String>,
    },
}

impl ItemDraft {
    #[must_use]
    pub fn cloze(sentence: &str, answer: &str) -> Self {
        Self::Cloze {
            sentence: sentence.to_owned(),
            answer: answer.to_owned(),
            accepted: Vec::new(),
            hint: None,
        }
    }

    #[must_use]
    pub fn multiple_choice(question: &str, options: &[(&str, &str)], correct_option: &str) -> Self {
        Self::MultipleChoice {
            question: question.to_owned(),
            options: options
                .iter()
                .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
                .collect(),
            correct_option: correct_option.to_owned(),
            hint: None,
        }
    }

    #[must_use]
    pub fn with_hint(mut self, text: &str) -> Self {
        match &mut self {
            Self::Cloze { hint, .. } | Self::MultipleChoice { hint, .. } => {
                *hint = Some(text.to_owned());
            }
        }
        self
    }

    /// Adds answers that are also correct, such as a contracted form.
    #[must_use]
    pub fn accepting(mut self, alternatives: &[&str]) -> Self {
        if let Self::Cloze { accepted, .. } = &mut self {
            accepted.extend(alternatives.iter().map(|form| (*form).to_owned()));
        }
        self
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Cloze { .. } => "cloze",
            Self::MultipleChoice { .. } => "multiple_choice",
        }
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        match self {
            Self::Cloze { hint, .. } | Self::MultipleChoice { hint, .. } => hint.as_deref(),
        }
    }

    /// Identifies the item by its content, so that regenerating the same
    /// exercise costs storage once rather than once per run.
    #[must_use]
    pub fn content_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.kind().as_bytes());
        match self {
            Self::Cloze {
                sentence,
                answer,
                accepted,
                ..
            } => {
                hasher.update(sentence.as_bytes());
                hasher.update(answer.as_bytes());
                for form in accepted {
                    hasher.update(form.as_bytes());
                }
            }
            Self::MultipleChoice {
                question,
                options,
                correct_option,
                ..
            } => {
                hasher.update(question.as_bytes());
                for (id, text) in options {
                    hasher.update(id.as_bytes());
                    hasher.update(text.as_bytes());
                }
                hasher.update(correct_option.as_bytes());
            }
        }
        format!("{:x}", hasher.finalize())
    }
}

/// Why a draft may not be shown to a learner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateFailure {
    NoBlank,
    AnswerLeaksIntoStem,
    EmptyAnswer,
    TooFewOptions,
    DuplicateOptions,
    EmptyOption,
    CorrectOptionMissing,
}

impl GateFailure {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NoBlank => "cloze sentence has no blank",
            Self::AnswerLeaksIntoStem => "the answer appears in the sentence around the blank",
            Self::EmptyAnswer => "cloze answer is empty",
            Self::TooFewOptions => "multiple choice offers too few options",
            Self::DuplicateOptions => "multiple choice repeats an option",
            Self::EmptyOption => "multiple choice has an empty option",
            Self::CorrectOptionMissing => "the correct option is not among the options",
        }
    }
}

/// Checks a cloze draft for the faults a learner would notice immediately.
#[must_use]
pub fn gate_cloze(sentence: &str, answer: &str) -> Option<GateFailure> {
    if answer.trim().is_empty() {
        return Some(GateFailure::EmptyAnswer);
    }
    if !sentence.contains(BLANK) {
        return Some(GateFailure::NoBlank);
    }

    let stem = sentence.replace(BLANK, " ");
    if contains_word(&stem, answer) {
        return Some(GateFailure::AnswerLeaksIntoStem);
    }
    None
}

/// Checks a multiple-choice draft's options.
#[must_use]
pub fn gate_multiple_choice(
    options: &[(String, String)],
    correct_option: &str,
) -> Option<GateFailure> {
    if options.len() < MIN_OPTIONS {
        return Some(GateFailure::TooFewOptions);
    }
    if options.iter().any(|(_, text)| text.trim().is_empty()) {
        return Some(GateFailure::EmptyOption);
    }

    let mut seen: Vec<String> = Vec::with_capacity(options.len());
    for (_, text) in options {
        let folded = fold(text);
        if seen.contains(&folded) {
            return Some(GateFailure::DuplicateOptions);
        }
        seen.push(folded);
    }

    if !options.iter().any(|(id, _)| id == correct_option) {
        return Some(GateFailure::CorrectOptionMissing);
    }
    None
}

/// Applies whichever structural gate suits the draft.
#[must_use]
pub fn gate(draft: &ItemDraft) -> Option<GateFailure> {
    match draft {
        ItemDraft::Cloze {
            sentence, answer, ..
        } => gate_cloze(sentence, answer),
        ItemDraft::MultipleChoice {
            options,
            correct_option,
            ..
        } => gate_multiple_choice(options, correct_option),
    }
}

/// Serialises a draft's list fields for storage.
fn encode<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, WisecrowError> {
    serde_json::to_value(value)
        .map_err(|error| WisecrowError::InvalidInput(format!("Cannot encode item field: {error}")))
}

fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Whether `needle` appears in `haystack` as a whole word, ignoring case.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let needle = fold(needle);
    if needle.is_empty() {
        return false;
    }
    fold(haystack)
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word == needle)
}

/// Counts the words of a draft that fall outside a level's vocabulary.
///
/// This is advice, never a verdict. Rejecting an item because its vocabulary
/// sits above the target CEFR level sounds like a lookup and is not: CEFRLex
/// resources such as FLELex are lemma and part-of-speech frequency
/// distributions, whereas the only tokeniser here splits on whitespace and
/// lowercases surface forms. Until lemmatisation exists, the figure informs a
/// reviewer during promotion and decides nothing by itself.
///
/// Returns `None` when no resource covers the language, which is the common
/// case and must be distinguished from "every word is in level".
#[must_use]
pub fn annotate_level(
    draft: &ItemDraft,
    in_level: Option<&std::collections::HashSet<String>>,
) -> Option<u32> {
    let vocabulary = in_level?;
    let text = match draft {
        ItemDraft::Cloze {
            sentence, answer, ..
        } => format!("{sentence} {answer}"),
        ItemDraft::MultipleChoice {
            question, options, ..
        } => {
            let choices: Vec<&str> = options.iter().map(|(_, text)| text.as_str()).collect();
            format!("{question} {}", choices.join(" "))
        }
    };

    let tokeniser = crate::preview::tokenize::WhitespaceTokenizer;
    let outside = crate::preview::tokenize::Tokenizer::tokenize(&tokeniser, &text)
        .into_iter()
        .filter(|word| word != BLANK && !vocabulary.contains(word))
        .count();
    Some(u32::try_from(outside).unwrap_or(u32::MAX))
}

/// What one [`ItemRepository::insert_candidates`] call did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InsertSummary {
    /// Stored as candidates, awaiting a reviewer.
    pub inserted: usize,
    /// Already present for this rule, by content hash.
    pub duplicates: usize,
    /// Stored as rejected because a structural gate refused them.
    pub gated: usize,
}

/// An item as stored, with the identity a session and an attempt refer to.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredItem {
    pub id: i32,
    pub rule_id: i32,
    pub revision: i32,
    pub kind: String,
    pub prompt: String,
    pub answer: Option<String>,
    pub accepted: serde_json::Value,
    pub options: Option<serde_json::Value>,
    pub correct_option: Option<String>,
    pub hint: Option<String>,
    pub status: String,
    pub out_of_level_words: Option<i32>,
}

impl StoredItem {
    /// The item reduced to what the shared grader needs.
    ///
    /// Grading is never done from the presentation: the server rebuilds this
    /// from the stored row so that a submission is judged against the revision
    /// the learner actually saw, whatever the client claims.
    ///
    /// # Errors
    ///
    /// Returns an error when a stored row does not carry the fields its kind
    /// requires, which the table's own constraints should already prevent.
    pub fn gradable(&self) -> Result<GradableItem, WisecrowError> {
        match self.kind.as_str() {
            "cloze" => {
                let answer = self
                    .answer
                    .as_deref()
                    .ok_or_else(|| malformed(self.id, "a cloze without an answer"))?;
                let accepted: Vec<String> = serde_json::from_value(self.accepted.clone()) // clone: deserialising consumes the value
                    .map_err(|_| malformed(self.id, "an unreadable accepted-answer list"))?;
                let accepted: Vec<&str> = accepted.iter().map(String::as_str).collect();
                Ok(GradableItem::cloze(answer, &accepted))
            }
            "multiple_choice" => {
                let options = self
                    .options
                    .clone() // clone: deserialising consumes the value
                    .ok_or_else(|| malformed(self.id, "a multiple choice without options"))?;
                let options: Vec<(String, String)> = serde_json::from_value(options)
                    .map_err(|_| malformed(self.id, "an unreadable option list"))?;
                let correct_option = self
                    .correct_option
                    .clone() // clone: the gradable item owns its identifier
                    .ok_or_else(|| malformed(self.id, "a multiple choice without an answer"))?;
                Ok(GradableItem::MultipleChoice {
                    options,
                    correct_option,
                })
            }
            kind => Err(malformed(self.id, &format!("an unknown kind {kind}"))),
        }
    }
}

fn malformed(item_id: i32, fault: &str) -> WisecrowError {
    WisecrowError::InvalidInput(format!("Quiz item {item_id} has {fault}"))
}

pub(crate) const ITEM_COLUMNS: &str = "id, rule_id, revision, kind, prompt, answer, accepted,
     options, correct_option, hint, status, out_of_level_words";

/// The same columns qualified for queries that join the rule and language.
const ITEM_COLUMNS_QUALIFIED: &str = "qi.id, qi.rule_id, qi.revision, qi.kind, qi.prompt,
     qi.answer, qi.accepted, qi.options, qi.correct_option, qi.hint, qi.status,
     qi.out_of_level_words";

pub struct ItemRepository;

impl ItemRepository {
    /// Stores generated drafts against a rule.
    ///
    /// A draft that fails a gate is still written, as `rejected` with its
    /// reason, so that a reviewer can see what the model produced rather than
    /// wonder where it went.
    ///
    /// # Errors
    ///
    /// Returns an error when a database write fails.
    pub async fn insert_candidates(
        pool: &PgPool,
        rule_id: i32,
        drafts: &[ItemDraft],
    ) -> Result<InsertSummary, WisecrowError> {
        Self::insert_candidates_with_vocabulary(pool, rule_id, drafts, None).await
    }

    /// As [`Self::insert_candidates`], additionally recording how many words
    /// of each draft fall outside the level's vocabulary.
    ///
    /// # Errors
    ///
    /// Returns an error when a database write fails.
    pub async fn insert_candidates_with_vocabulary(
        pool: &PgPool,
        rule_id: i32,
        drafts: &[ItemDraft],
        in_level: Option<&std::collections::HashSet<String>>,
    ) -> Result<InsertSummary, WisecrowError> {
        let mut summary = InsertSummary::default();

        for draft in drafts {
            let failure = gate(draft);
            let status = if failure.is_some() {
                "rejected"
            } else {
                "candidate"
            };

            let (answer, accepted, options, correct_option) = match draft {
                ItemDraft::Cloze {
                    answer, accepted, ..
                } => (
                    Some(answer.clone()), // clone: building owned bindings from a borrowed draft
                    encode(accepted)?,
                    None,
                    None,
                ),
                ItemDraft::MultipleChoice {
                    options,
                    correct_option,
                    ..
                } => (
                    None,
                    serde_json::Value::Array(vec![]),
                    Some(encode(options)?),
                    Some(correct_option.clone()), // clone: building owned bindings from a borrowed draft
                ),
            };

            let prompt = match draft {
                ItemDraft::Cloze { sentence, .. } => sentence,
                ItemDraft::MultipleChoice { question, .. } => question,
            };

            let inserted = sqlx::query(
                "INSERT INTO quiz_items
                     (rule_id, kind, prompt, answer, accepted, options, correct_option, hint,
                      status, gate_reason, content_sha256, out_of_level_words)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                 ON CONFLICT (rule_id, content_sha256) DO NOTHING",
            )
            .bind(rule_id)
            .bind(draft.kind())
            .bind(prompt)
            .bind(answer)
            .bind(accepted)
            .bind(options)
            .bind(correct_option)
            .bind(draft.hint())
            .bind(status)
            .bind(failure.map(GateFailure::reason))
            .bind(draft.content_hash())
            .bind(
                annotate_level(draft, in_level)
                    .map(|count| i32::try_from(count).unwrap_or(i32::MAX)),
            )
            .execute(pool)
            .await?;

            if inserted.rows_affected() == 0 {
                summary.duplicates = summary.duplicates.saturating_add(1);
            } else if failure.is_some() {
                summary.gated = summary.gated.saturating_add(1);
            } else {
                summary.inserted = summary.inserted.saturating_add(1);
            }
        }

        Ok(summary)
    }
}

impl ItemRepository {
    /// Items a learner may actually be shown for one grammar point.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn active_items_for_rule(
        pool: &PgPool,
        rule_id: i32,
    ) -> Result<Vec<StoredItem>, WisecrowError> {
        let items = sqlx::query_as::<_, StoredItem>(&format!(
            "SELECT {ITEM_COLUMNS} FROM quiz_items
             WHERE rule_id = $1 AND status = 'active'
             ORDER BY id"
        ))
        .bind(rule_id)
        .fetch_all(pool)
        .await?;
        Ok(items)
    }

    /// Candidates awaiting a reviewer, newest rules first within a language.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn candidates_for_language(
        pool: &PgPool,
        lang_code: &str,
        level_code: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredItem>, WisecrowError> {
        let items = sqlx::query_as::<_, StoredItem>(&format!(
            "SELECT {ITEM_COLUMNS_QUALIFIED} FROM quiz_items qi
             JOIN grammar_rules gr ON gr.id = qi.rule_id
             JOIN languages l ON l.id = gr.language_id
             JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
             WHERE qi.status = 'candidate'
               AND l.code = $1
               AND ($2::TEXT IS NULL OR cl.code = $2)
             ORDER BY cl.sort_order, gr.slug, qi.id
             LIMIT $3"
        ))
        .bind(lang_code)
        .bind(level_code)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        Ok(items)
    }

    /// Lets an item be served to learners.
    ///
    /// # Errors
    ///
    /// Returns an error when the update fails.
    pub async fn promote(pool: &PgPool, item_id: i32) -> Result<(), WisecrowError> {
        Self::set_status(pool, item_id, "active", None).await
    }

    /// Refuses an item, recording why for the next reviewer.
    ///
    /// # Errors
    ///
    /// Returns an error when the update fails.
    pub async fn reject(pool: &PgPool, item_id: i32, reason: &str) -> Result<(), WisecrowError> {
        Self::set_status(pool, item_id, "rejected", Some(reason)).await
    }

    /// Withdraws a previously active item without deleting it, so that an
    /// attempt uploaded later can still be graded against what was shown.
    ///
    /// # Errors
    ///
    /// Returns an error when the update fails.
    pub async fn retire(pool: &PgPool, item_id: i32) -> Result<(), WisecrowError> {
        Self::set_status(pool, item_id, "retired", None).await
    }

    async fn set_status(
        pool: &PgPool,
        item_id: i32,
        status: &str,
        reason: Option<&str>,
    ) -> Result<(), WisecrowError> {
        let updated = sqlx::query(
            "UPDATE quiz_items
             SET status = $2,
                 gate_reason = COALESCE($3, gate_reason),
                 promoted_at = CASE WHEN $2 = 'active' THEN CURRENT_TIMESTAMP ELSE promoted_at END
             WHERE id = $1",
        )
        .bind(item_id)
        .bind(status)
        .bind(reason)
        .execute(pool)
        .await?;

        if updated.rows_affected() == 0 {
            return Err(WisecrowError::InvalidInput(format!(
                "No quiz item with id {item_id}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("Yo ___ cansado.", "estoy", None)]
    #[case("Yo estoy cansado.", "estoy", Some(GateFailure::NoBlank))]
    #[case(
        "Estoy ___ porque estoy cansado.",
        "estoy",
        Some(GateFailure::AnswerLeaksIntoStem)
    )]
    #[case("Yo ___ cansado.", "  ", Some(GateFailure::EmptyAnswer))]
    fn cloze_gates(
        #[case] sentence: &str,
        #[case] answer: &str,
        #[case] expected: Option<GateFailure>,
    ) {
        assert_eq!(gate_cloze(sentence, answer), expected);
    }

    #[test]
    fn a_longer_word_containing_the_answer_does_not_count_as_a_leak() {
        assert_eq!(gate_cloze("Estoy___ estoyando.", "es"), None);
    }

    fn options(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[rstest]
    #[case(&[("o1", "ser"), ("o2", "estar"), ("o3", "haber")], "o2", None)]
    #[case(&[("o1", "ser"), ("o2", "SER"), ("o3", "haber")], "o1", Some(GateFailure::DuplicateOptions))]
    #[case(&[("o1", "ser"), ("o2", " "), ("o3", "haber")], "o1", Some(GateFailure::EmptyOption))]
    #[case(&[("o1", "ser"), ("o2", "estar")], "o1", Some(GateFailure::TooFewOptions))]
    #[case(&[("o1", "ser"), ("o2", "estar"), ("o3", "haber")], "o9", Some(GateFailure::CorrectOptionMissing))]
    fn multiple_choice_gates(
        #[case] values: &[(&str, &str)],
        #[case] correct: &str,
        #[case] expected: Option<GateFailure>,
    ) {
        assert_eq!(gate_multiple_choice(&options(values), correct), expected);
    }

    fn vocabulary(words: &[&str]) -> std::collections::HashSet<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn level_annotation_counts_words_outside_the_vocabulary() {
        let draft = ItemDraft::cloze("Yo ___ completamente agotado.", "estoy");
        let in_level = vocabulary(&["yo", "estoy", "agotado"]);
        assert_eq!(annotate_level(&draft, Some(&in_level)), Some(1));
    }

    #[test]
    fn level_annotation_is_absent_where_no_resource_covers_the_language() {
        let draft = ItemDraft::cloze("Yo ___ cansado.", "estoy");
        assert_eq!(
            annotate_level(&draft, None),
            None,
            "no resource is not the same as everything being in level"
        );
    }

    #[test]
    fn level_annotation_never_decides_a_draft_fate() {
        let draft = ItemDraft::cloze("Yo ___ absolutamente exhausto.", "estoy");
        let in_level = vocabulary(&["yo"]);
        assert!(annotate_level(&draft, Some(&in_level)).is_some_and(|count| count > 0));
        assert_eq!(
            gate(&draft),
            None,
            "an out-of-level draft still passes the gates"
        );
    }

    #[test]
    fn content_hash_ignores_the_hint_and_tracks_the_answer() {
        let plain = ItemDraft::cloze("Yo ___ cansado.", "estoy");
        let hinted = ItemDraft::cloze("Yo ___ cansado.", "estoy").with_hint("temporary");
        let different = ItemDraft::cloze("Yo ___ cansado.", "soy");

        assert_eq!(plain.content_hash(), hinted.content_hash());
        assert_ne!(plain.content_hash(), different.content_hash());
        assert_eq!(plain.content_hash().len(), 64);
    }
}
