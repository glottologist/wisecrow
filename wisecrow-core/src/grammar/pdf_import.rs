//! Turning a grammar document's passages into CEFR-levelled points.
//!
//! Extraction ([`crate::grammar::pdf`]) says what a document contains. This
//! module says what a learner should be taught from it, which is a different
//! question and not one a text layer can answer: reading the Irish standard
//! line by line produced 7055 fragments, where the level wants fifteen points.
//! A model is asked to state the points; nothing it returns is stored unless it
//! carries the prose and examples a seeded point carries, and cites a page the
//! prompt actually held.

use serde::Deserialize;
use tracing::debug;

use super::pdf::GrammarPassage;
use super::rules::{slugify, NewGrammarRule, NewRuleExample, RuleSource};
use crate::errors::WisecrowError;
use crate::llm::prompts::{pdf_rules_prompt, PassageExcerpt};
use crate::llm::LlmProvider;

/// Output ceiling for one synthesis call.
///
/// Matched to the seeder's ceiling: the answer has the same shape, a level's
/// worth of points with prose and examples, and the seeder measured that at just
/// over 4000 tokens for a full level.
const MAX_LLM_TOKENS: u32 = 8192;
/// Passage text sent in one call.
///
/// A 256-page standard extracts to about 350,000 characters, far past any
/// sensible single request, and a level wants fifteen points out of it. The
/// budget is spent across the whole document rather than on its opening pages,
/// because the opening pages of a grammar are its front matter.
const PASSAGE_BUDGET_CHARS: usize = 60_000;
const MAX_TITLE_CHARS: usize = 100;
/// Shortest explanation accepted.
///
/// Two sentences of grammar prose do not fit in less. The floor is what stops a
/// model answering with a heading where a rule was asked for.
const MIN_EXPLANATION_CHARS: usize = 120;
const MIN_EXPLANATION_SENTENCES: usize = 2;
const MIN_EXAMPLES: usize = 2;

/// A point the model proposed and the gate accepted.
#[derive(Debug, Clone)]
pub struct SynthesisedPoint {
    pub rule: NewGrammarRule,
    /// Document and page the point was read from, e.g. `yo-puedo p.148`.
    pub source_ref: String,
}

/// A point the gate refused, and why.
#[derive(Debug, Clone)]
pub struct Rejection {
    pub title: String,
    pub reason: &'static str,
}

/// What one synthesis call produced.
#[derive(Debug, Clone)]
pub struct Synthesis {
    pub points: Vec<SynthesisedPoint>,
    pub rejected: Vec<Rejection>,
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

    let prompt = pdf_rules_prompt(language_name, cefr_level, wanted, covered, &excerpts);
    let response = provider.generate(&prompt, MAX_LLM_TOKENS).await?;
    let proposed: Vec<LlmPdfRule> =
        crate::llm::parse_fenced_json(&response, "PDF grammar points as JSON")?;

    let pages: Vec<usize> = selected.iter().map(|passage| passage.page).collect();
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

        points.push(SynthesisedPoint {
            source_ref: format!("{document} p.{}", rule.page),
            rule: NewGrammarRule {
                slug: slugify(&rule.title),
                title: rule.title,
                explanation: rule.explanation,
                source: RuleSource::Pdf,
                examples: rule
                    .examples
                    .into_iter()
                    .map(|example| NewRuleExample {
                        sentence: example.sentence,
                        translation: example.translation,
                        is_correct: example.is_correct,
                    })
                    .collect(),
            },
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
            synthesis.points[0].source_ref,
            "irish-caighdean-oifigiuil-2017 p.12"
        );
        assert_eq!(synthesis.points[0].rule.source, RuleSource::Pdf);
        assert_eq!(synthesis.points[0].rule.examples.len(), 2);
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
}
