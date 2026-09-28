//! Turning a grammar document's passages into CEFR-levelled points.
//!
//! Extraction ([`crate::grammar::pdf`]) says what a document contains. This
//! module says what a learner should be taught from it, which is a different
//! question and not one a text layer can answer: reading the Irish standard
//! line by line produced 7055 fragments, where the level wants fifteen points.
//! A model is asked to state the points; nothing it returns is stored unless it
//! carries the prose and examples a seeded point carries, and cites a page the
//! prompt actually held. What passes is placed the way a seeded point is, so an
//! import fills a level's gaps and never moves a point already placed elsewhere.

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::pdf::GrammarPassage;
use super::rules::{
    slugify, NewGrammarRule, NewRuleExample, RulePlacement, RuleRepository, RuleSource,
};
use crate::errors::WisecrowError;
use crate::llm::prompts::{pdf_rules_prompt, PassageExcerpt};
use crate::llm::LlmProvider;

/// Output ceiling for one synthesis call.
///
/// Matched to the seeder's ceiling: the answer has the same shape, a round of
/// points with prose and examples, and the seeder measured a full fifteen at
/// just over 4000 tokens. [`SYNTHESIS_BATCH`] keeps every call under it; the
/// Anthropic client refuses an answer cut off at the ceiling outright.
const MAX_LLM_TOKENS: u32 = 8192;
/// Passage text sent in one call.
///
/// A 256-page standard extracts to about 350,000 characters, far past any
/// sensible single request, and a level wants fifteen points out of it. The
/// budget is spent across the whole document rather than on its opening pages,
/// because the opening pages of a grammar are its front matter.
pub(super) const PASSAGE_BUDGET_CHARS: usize = 60_000;
const MAX_TITLE_CHARS: usize = 100;
/// Shortest explanation accepted.
///
/// Two sentences of grammar prose do not fit in less. The floor is what stops a
/// model answering with a heading where a rule was asked for.
const MIN_EXPLANATION_CHARS: usize = 120;
const MIN_EXPLANATION_SENTENCES: usize = 2;
const MIN_EXAMPLES: usize = 2;
/// Document-backed points a level may hold beside the seeded ones.
///
/// Measured only against points whose source is `pdf`, so a level the seeder
/// has already filled to [`super::seeder::RULES_PER_LEVEL`] still takes what
/// the books have to add. The two targets are deliberately separate: raising
/// the seeder's would have it invent points for levels no document covers.
pub const DOCUMENT_RULES_PER_LEVEL: u32 = 30;
/// Points asked for in one model call.
///
/// Thirty in one answer would meet [`MAX_LLM_TOKENS`], so the target is
/// reached in rounds, each naming the titles the earlier ones produced.
pub(super) const SYNTHESIS_BATCH: u32 = 15;
/// Appended to the prompt when the first answer was not JSON.
///
/// Asked for one point and told that fewer is correct, a model sometimes
/// answers in prose. One more call, told plainly what shape is wanted, is
/// cheaper than losing the document at that level, and cheaper still than
/// losing the run.
const JSON_NUDGE: &str = "\n\nYour previous answer was not a JSON array. Return only the JSON \
                          array, with no text before or after it; return [] if the passages \
                          hold nothing a learner at this level needs.";

/// Document-backed points a level is still owed, or `None` at the target.
fn document_shortfall(held: usize) -> Option<u32> {
    let held = u32::try_from(held).unwrap_or(u32::MAX);
    DOCUMENT_RULES_PER_LEVEL
        .checked_sub(held)
        .filter(|wanted| *wanted > 0)
}

/// A point the gate refused, and why.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub title: String,
    pub reason: &'static str,
}

/// What one synthesis call produced.
///
/// Every point carries its citation in `source_ref`, e.g.
/// `yo-puedo-1-2021.pdf p.148`.
#[derive(Debug, Clone, Default)]
pub struct Synthesis {
    pub points: Vec<NewGrammarRule>,
    pub rejected: Vec<Rejection>,
}

/// The language and level an import writes to.
#[derive(Debug, Clone, Copy)]
pub struct ImportTarget<'a> {
    pub language_id: i32,
    pub language_name: &'a str,
    pub cefr_level: &'a str,
}

/// How an import run is bounded.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// Ask the model and report its answer, but write nothing.
    pub dry_run: bool,
    /// Ceiling on the points asked for, beneath what the level is short.
    pub max_rules: Option<u32>,
}

/// What importing one document at one level came to.
#[derive(Debug, Clone, Default)]
pub struct ImportOutcome {
    /// Points the level was asked for; zero when it was already full.
    pub wanted: u32,
    pub synthesis: Synthesis,
    /// Points written, or that refreshed a point already at this level.
    pub placed: usize,
    /// Points the model proposed that already sit at another level, left there.
    pub held: usize,
}

/// Synthesises a document's points for one level and places them.
///
/// The document is read, the level's shortfall against
/// [`DOCUMENT_RULES_PER_LEVEL`], counted over its document-backed points, is
/// measured, and the model is asked for at most that many points, capped again
/// by `max_rules`. A level already holding that many document-backed points
/// costs nothing; one that is short is asked in rounds of [`SYNTHESIS_BATCH`].
/// Accepted points are written through
/// [`RuleRepository::place_rule`], so an import can add to a level but can
/// never move a point that already sits at another one.
///
/// # Errors
///
/// Returns an error when the document cannot be read, when the model call
/// fails or its answer will not parse, or when the database refuses a write.
pub async fn import_document(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    target: ImportTarget<'_>,
    document: &Path,
    options: ImportOptions,
) -> Result<ImportOutcome, WisecrowError> {
    let content = super::pdf::extract(document)?;
    let name = document
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("document");
    import_passages(pool, provider, target, name, &content.passages, options).await
}

/// The part of [`import_document`] that needs no file: passages in, points
/// placed.
///
/// # Errors
///
/// As [`import_document`], less the reading of the document.
pub async fn import_passages(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    target: ImportTarget<'_>,
    document: &str,
    passages: &[GrammarPassage],
    options: ImportOptions,
) -> Result<ImportOutcome, WisecrowError> {
    let cefr_level_id = RuleRepository::ensure_cefr_level(pool, target.cefr_level).await?;
    let existing =
        RuleRepository::rules_for_level(pool, target.language_id, target.cefr_level).await?;
    let held_from_documents = existing
        .iter()
        .filter(|rule| rule.source == RuleSource::Pdf)
        .count();
    // A seeded or curated point at this level keeps its prose: `place_rule`
    // would otherwise rewrite it as document-backed under the same slug.
    let protected: HashSet<&str> = existing
        .iter()
        .filter(|rule| rule.source != RuleSource::Pdf)
        .map(|rule| rule.slug.as_str())
        .collect();
    let mut covered: Vec<String> = existing.iter().map(|rule| rule.title.clone()).collect(); // clone: the covered list grows as rounds accept titles

    let wanted = document_shortfall(held_from_documents)
        .map(|short| options.max_rules.map_or(short, |cap| short.min(cap)))
        .filter(|wanted| *wanted > 0);
    let Some(wanted) = wanted else {
        info!(
            "{} {} already holds {DOCUMENT_RULES_PER_LEVEL} document-backed points; \
             {document} was not read to the model",
            target.language_name, target.cefr_level
        );
        return Ok(ImportOutcome::default());
    };

    let mut synthesis = Synthesis::default();
    let mut placed = 0usize;
    let mut held = 0usize;
    let mut remaining = wanted;
    while remaining > 0 {
        let batch = remaining.min(SYNTHESIS_BATCH);
        info!(
            "Asking {} for {batch} {} {} points from {document}",
            provider.name(),
            target.language_name,
            target.cefr_level
        );
        let round = synthesise(
            provider,
            document,
            target.language_name,
            target.cefr_level,
            batch,
            &covered,
            passages,
        )
        .await?;
        let accepted = u32::try_from(round.points.len()).unwrap_or(u32::MAX);
        synthesis.rejected.extend(round.rejected);

        for point in round.points {
            covered.push(point.title.clone()); // clone: the title is both covered and stored
            if !options.dry_run {
                if protected.contains(point.slug.as_str()) {
                    debug!(
                        "Held \"{}\" at {} {}: the level already teaches it from another source",
                        point.title, target.language_name, target.cefr_level
                    );
                    held = held.saturating_add(1);
                } else {
                    match RuleRepository::place_rule(
                        pool,
                        target.language_id,
                        cefr_level_id,
                        &point,
                    )
                    .await?
                    {
                        RulePlacement::Placed(_) => placed = placed.saturating_add(1),
                        RulePlacement::HeldAtAnotherLevel(_) => {
                            held = held.saturating_add(1);
                        }
                    }
                }
            }
            synthesis.points.push(point);
        }

        remaining = remaining.saturating_sub(accepted);
        if accepted < batch {
            // Fewer than asked is the document running dry at this level;
            // asking again would buy rewordings of what it already gave.
            break;
        }
    }

    Ok(ImportOutcome {
        wanted,
        synthesis,
        placed,
        held,
    })
}

/// LLM response shape: [`crate::llm::prompts::grammar_seed_prompt`]'s contract
/// with the page the point was read from added.
#[derive(Debug, Deserialize)]
struct LlmPdfRule {
    title: String,
    explanation: String,
    page: usize,
    examples: Vec<LlmPdfExample>,
}

#[derive(Debug, Deserialize)]
struct LlmPdfExample {
    sentence: String,
    translation: Option<String>,
    #[serde(default = "default_true")]
    is_correct: bool,
}

const fn default_true() -> bool {
    true
}

/// Asks a model for the grammar points one level should take from a document.
///
/// `document` names the file for the citation, `wanted` is how many points the
/// level is short, and `covered` names the points it already holds so the model
/// is not asked for them again. Nothing is written to the database here: the
/// caller decides where an accepted point goes.
///
/// # Errors
///
/// Returns an error when the document holds no passages, when the model call
/// fails, or when its answer will not parse as JSON.
pub async fn synthesise(
    provider: &dyn LlmProvider,
    document: &str,
    language_name: &str,
    cefr_level: &str,
    wanted: u32,
    covered: &[String],
    passages: &[GrammarPassage],
) -> Result<Synthesis, WisecrowError> {
    let selected = select_passages(passages);
    if selected.is_empty() {
        return Err(WisecrowError::PdfExtractionError(
            "No passages to synthesise grammar points from".to_owned(),
        ));
    }

    let excerpts: Vec<PassageExcerpt<'_>> = selected
        .iter()
        .map(|passage| PassageExcerpt {
            page: passage.page,
            heading: passage.heading.as_deref(),
            text: passage.text.as_str(),
        })
        .collect();

    let covered: Vec<&str> = covered.iter().map(String::as_str).collect();
    synthesise_excerpts(
        provider,
        document,
        language_name,
        cefr_level,
        wanted,
        &covered,
        &excerpts,
    )
    .await
}

pub(super) async fn synthesise_excerpts(
    provider: &dyn LlmProvider,
    document: &str,
    language_name: &str,
    cefr_level: &str,
    wanted: u32,
    covered: &[&str],
    excerpts: &[PassageExcerpt<'_>],
) -> Result<Synthesis, WisecrowError> {
    let prompt = pdf_rules_prompt(language_name, cefr_level, wanted, covered, excerpts);
    let response = provider.generate(&prompt, MAX_LLM_TOKENS).await?;
    let proposed: Vec<LlmPdfRule> =
        match crate::llm::parse_fenced_json(&response, "PDF grammar points as JSON") {
            Ok(proposed) => proposed,
            Err(WisecrowError::LlmError(reason)) => {
                warn!(
                    "{reason}; asking {} once more for the array alone",
                    provider.name()
                );
                let nudged = [prompt.as_str(), JSON_NUDGE].concat();
                let response = provider.generate(&nudged, MAX_LLM_TOKENS).await?;
                crate::llm::parse_fenced_json(&response, "PDF grammar points as JSON")?
            }
            Err(error) => return Err(error),
        };

    let pages: Vec<usize> = excerpts.iter().map(|passage| passage.page).collect();
    Ok(gate(proposed, document, &pages, wanted))
}

/// Keeps the points that carry what a seeded point carries, and drops the rest.
fn gate(proposed: Vec<LlmPdfRule>, document: &str, pages: &[usize], wanted: u32) -> Synthesis {
    let limit = usize::try_from(wanted).unwrap_or(usize::MAX);
    let mut points = Vec::new();
    let mut rejected = Vec::new();

    for rule in proposed {
        if let Some(reason) = refusal(&rule, pages) {
            debug!("Refused \"{}\" from {document}: {reason}", rule.title);
            rejected.push(Rejection {
                title: rule.title,
                reason,
            });
            continue;
        }

        if points.len() >= limit {
            rejected.push(Rejection {
                title: rule.title,
                reason: "the level was already owed no more points",
            });
            continue;
        }

        points.push(NewGrammarRule {
            slug: slugify(&rule.title),
            title: rule.title,
            explanation: rule.explanation,
            source: RuleSource::Pdf,
            source_ref: Some(format!("{document} p.{}", rule.page)),
            examples: rule
                .examples
                .into_iter()
                .map(|example| NewRuleExample {
                    sentence: example.sentence,
                    translation: example.translation,
                    is_correct: example.is_correct,
                })
                .collect(),
        });
    }

    Synthesis { points, rejected }
}

/// Why a proposed point cannot be stored, or `None` when it can.
fn refusal(rule: &LlmPdfRule, pages: &[usize]) -> Option<&'static str> {
    if rule.title.trim().is_empty() {
        return Some("the title is empty");
    }
    if rule.title.chars().count() > MAX_TITLE_CHARS {
        return Some("the title is longer than a title");
    }
    if rule.explanation.chars().count() < MIN_EXPLANATION_CHARS
        || sentence_count(&rule.explanation) < MIN_EXPLANATION_SENTENCES
    {
        return Some("the explanation is shorter than two sentences of prose");
    }
    if rule.examples.len() < MIN_EXAMPLES {
        return Some("fewer than two examples");
    }
    if !rule.examples.iter().any(|example| example.is_correct) {
        return Some("no correct example");
    }
    if rule.examples.iter().all(|example| example.is_correct) {
        return Some("no incorrect example");
    }
    if !pages.contains(&rule.page) {
        return Some("the cited page was not among the passages");
    }
    None
}

/// Sentences in a piece of prose, counted by their closing punctuation.
fn sentence_count(text: &str) -> usize {
    text.split(['.', '!', '?'])
        .filter(|part| part.trim().chars().count() > 1)
        .count()
}

/// Spreads the passage budget across the whole document.
///
/// Every passage is sent when the document fits. When it does not, passages are
/// taken at an even stride, so a standard's later chapters are represented
/// rather than its title page and table of contents.
fn select_passages(passages: &[GrammarPassage]) -> Vec<&GrammarPassage> {
    let total: usize = passages
        .iter()
        .map(|passage| passage.text.chars().count())
        .sum();

    let stride = if total > PASSAGE_BUDGET_CHARS {
        total.div_ceil(PASSAGE_BUDGET_CHARS).max(1)
    } else {
        1
    };

    let mut spent = 0usize;
    let mut selected = Vec::new();
    for passage in passages.iter().step_by(stride) {
        let length = passage.text.chars().count();
        if spent.saturating_add(length) > PASSAGE_BUDGET_CHARS && !selected.is_empty() {
            break;
        }
        spent = spent.saturating_add(length);
        selected.push(passage);
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::pdf::ExampleSentence;

    struct StubProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for StubProvider {
        async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
            Ok(self.response.clone()) // clone: the stub answers every call the same way
        }

        fn name(&self) -> &str {
            "stub"
        }
    }

    /// Answers each call with the next scripted response, and the last one
    /// thereafter.
    struct TurnProvider {
        responses: Vec<String>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl TurnProvider {
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for TurnProvider {
        async fn generate(&self, prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call > 0 {
                assert!(
                    prompt.ends_with(JSON_NUDGE),
                    "the second call carries the nudge"
                );
            }
            let index = call.min(self.responses.len() - 1);
            Ok(self.responses[index].clone()) // clone: the stub hands out an owned answer per call
        }

        fn name(&self) -> &str {
            "stub"
        }
    }

    #[tokio::test]
    async fn a_prose_answer_is_asked_again_for_the_array_alone() {
        let provider = TurnProvider {
            responses: vec![
                "The passages hold nothing further at this level.".to_owned(),
                "[]".to_owned(),
            ],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };

        let synthesis = synthesise(
            &provider,
            "doc.pdf",
            "French",
            "C1",
            1,
            &[],
            &[passage(1, "Le passé", 300)],
        )
        .await
        .expect("the second answer parses");

        assert_eq!(provider.calls(), 2);
        assert!(synthesis.points.is_empty());
        assert!(synthesis.rejected.is_empty());
    }

    #[tokio::test]
    async fn a_second_prose_answer_is_the_error() {
        let provider = TurnProvider {
            responses: vec!["Nothing to add.".to_owned()],
            calls: std::sync::atomic::AtomicUsize::new(0),
        };

        let error = synthesise(
            &provider,
            "doc.pdf",
            "French",
            "C1",
            1,
            &[],
            &[passage(1, "Le passé", 300)],
        )
        .await
        .expect_err("two prose answers are a failure");

        assert_eq!(provider.calls(), 2, "one retry, never more");
        assert!(matches!(error, WisecrowError::LlmError(_)));
    }

    fn passage(page: usize, heading: &str, length: usize) -> GrammarPassage {
        GrammarPassage {
            heading: Some(heading.to_owned()),
            text: "grammar prose. ".repeat(length.div_ceil(15)),
            page,
            examples: vec![ExampleSentence {
                text: "Tá sé ann.".to_owned(),
                translation: None,
            }],
        }
    }

    fn good_rule(page: usize) -> String {
        format!(
            r#"{{"title":"The Genitive after Verbal Nouns",
                "explanation":"A noun governed by a verbal noun stands in the genitive case. The verbal noun behaves as a noun here rather than as a verb, so its object is possessed rather than acted upon.",
                "page":{page},
                "examples":[
                  {{"sentence":"ag ceannach an tí","translation":"buying the house","is_correct":true}},
                  {{"sentence":"ag ceannach an teach","translation":"buying the house","is_correct":false}}
                ]}}"#
        )
    }

    #[tokio::test]
    async fn a_complete_point_citing_a_supplied_page_is_kept() {
        let provider = StubProvider {
            response: format!("[{}]", good_rule(12)),
        };
        let passages = vec![passage(12, "An Ginideach", 400)];

        let synthesis = synthesise(
            &provider,
            "irish-caighdean-oifigiuil-2017",
            "Irish",
            "B2",
            15,
            &[],
            &passages,
        )
        .await
        .unwrap();

        assert_eq!(synthesis.points.len(), 1);
        assert!(synthesis.rejected.is_empty());
        assert_eq!(
            synthesis.points[0].source_ref.as_deref(),
            Some("irish-caighdean-oifigiuil-2017 p.12")
        );
        assert_eq!(synthesis.points[0].source, RuleSource::Pdf);
        assert_eq!(synthesis.points[0].examples.len(), 2);
    }

    #[tokio::test]
    async fn a_point_citing_a_page_the_prompt_never_held_is_refused() {
        let provider = StubProvider {
            response: format!("[{}]", good_rule(999)),
        };
        let passages = vec![passage(12, "An Ginideach", 400)];

        let synthesis = synthesise(&provider, "doc", "Irish", "B2", 15, &[], &passages)
            .await
            .unwrap();

        assert!(synthesis.points.is_empty());
        assert_eq!(
            synthesis.rejected[0].reason,
            "the cited page was not among the passages"
        );
    }

    #[tokio::test]
    async fn no_more_points_are_kept_than_the_level_is_owed() {
        let response = format!(
            "[{},{},{}]",
            good_rule(12),
            good_rule(12).replace(
                "Genitive after Verbal Nouns",
                "Genitive after Compound Prepositions"
            ),
            good_rule(12).replace("Genitive after Verbal Nouns", "The Autonomous Verb"),
        );
        let provider = StubProvider { response };
        let passages = vec![passage(12, "An Ginideach", 400)];

        let synthesis = synthesise(&provider, "doc", "Irish", "B2", 2, &[], &passages)
            .await
            .unwrap();

        assert_eq!(synthesis.points.len(), 2);
        assert_eq!(
            synthesis.rejected[0].reason,
            "the level was already owed no more points"
        );
    }

    #[test]
    fn a_heading_offered_as_a_rule_is_refused() {
        let rule = LlmPdfRule {
            title: "Na Forainmnigh".to_owned(),
            explanation: "Personal pronouns.".to_owned(),
            page: 3,
            examples: vec![],
        };

        assert_eq!(
            refusal(&rule, &[3]),
            Some("the explanation is shorter than two sentences of prose")
        );
    }

    #[test]
    fn a_point_without_an_incorrect_example_is_refused() {
        let rule = LlmPdfRule {
            title: "The Genitive after Verbal Nouns".to_owned(),
            explanation: "A noun governed by a verbal noun stands in the genitive case. The verbal noun behaves as a noun here rather than as a verb, so its object is possessed rather than acted upon.".to_owned(),
            page: 3,
            examples: vec![
                LlmPdfExample { sentence: "ag ceannach an tí".to_owned(), translation: None, is_correct: true },
                LlmPdfExample { sentence: "ag díol an tí".to_owned(), translation: None, is_correct: true },
            ],
        };

        assert_eq!(refusal(&rule, &[3]), Some("no incorrect example"));
    }

    #[test]
    fn a_long_document_is_sampled_across_its_length_rather_than_from_its_front() {
        let passages: Vec<GrammarPassage> = (1..=300)
            .map(|page| passage(page, "Caibidil", 1000))
            .collect();

        let selected = select_passages(&passages);
        let pages: Vec<usize> = selected.iter().map(|passage| passage.page).collect();
        let sent: usize = selected
            .iter()
            .map(|passage| passage.text.chars().count())
            .sum();

        assert!(sent <= PASSAGE_BUDGET_CHARS, "budget respected: {sent}");
        assert!(
            pages.last().is_some_and(|last| *last > 200),
            "the document's later pages are represented: {pages:?}"
        );
    }

    #[test]
    fn a_document_inside_the_budget_is_sent_whole() {
        let passages: Vec<GrammarPassage> = (1..=10)
            .map(|page| passage(page, "Caibidil", 300))
            .collect();

        assert_eq!(select_passages(&passages).len(), 10);
    }

    #[test]
    fn sentence_count_counts_closing_punctuation() {
        assert_eq!(sentence_count("One sentence only."), 1);
        assert_eq!(sentence_count("First one. Second one."), 2);
        assert_eq!(sentence_count("Is it? It is! Yes."), 3);
    }

    #[test]
    fn document_shortfall_counts_only_document_backed_points() {
        assert_eq!(document_shortfall(0), Some(DOCUMENT_RULES_PER_LEVEL));
        assert_eq!(
            document_shortfall(12),
            Some(18),
            "twelve held, eighteen owed"
        );
        assert_eq!(document_shortfall(30), None, "the document target is met");
        assert_eq!(document_shortfall(45), None, "past the target is still met");
    }
}
