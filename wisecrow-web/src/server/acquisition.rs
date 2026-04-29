use dioxus::prelude::*;

use wisecrow::cli::SUPPORTED_LANGUAGE_INFO;
use wisecrow_dto::{AnnotatedTokenDto, GradedReaderDto, SubtitleFormatDto};

use super::{pool, validate_lang};

const MAX_SENTENCE_BYTES: usize = 4 * 1024;
const MAX_SUBTITLE_BYTES: usize = 2 * 1024 * 1024;

fn resolve_language_name(code: &str) -> Result<&'static str, ServerFnError> {
    SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
        .ok_or_else(|| ServerFnError::new(format!("Unknown language: {code}")))
}

fn load_llm_provider() -> Result<Box<dyn wisecrow::llm::LlmProvider>, ServerFnError> {
    let settings = config::Config::builder()
        .add_source(config::Environment::with_prefix("WISECROW").separator("__"))
        .build()
        .map_err(|e| ServerFnError::new(format!("Config error: {e}")))?;
    let cfg: wisecrow::config::Config = settings
        .try_deserialize()
        .map_err(|e| ServerFnError::new(format!("Config error: {e}")))?;
    wisecrow::llm::create_provider(&cfg)
        .map_err(|e| ServerFnError::new(format!("LLM provider error: {e}")))
}

#[server]
pub async fn gloss_sentence(
    sentence: String,
    lang: String,
    refresh: bool,
) -> Result<String, ServerFnError> {
    if sentence.is_empty() {
        return Err(ServerFnError::new("Sentence cannot be empty"));
    }
    if sentence.len() > MAX_SENTENCE_BYTES {
        return Err(ServerFnError::new(format!(
            "Sentence exceeds maximum size of {MAX_SENTENCE_BYTES} bytes"
        )));
    }
    validate_lang(&lang)?;
    let lang_name = resolve_language_name(&lang)?;
    let db = pool()?;
    let provider = load_llm_provider()?;
    wisecrow::grammar::gloss::generate_or_lookup_with_refresh(
        db,
        provider.as_ref(),
        &sentence,
        &lang,
        lang_name,
        refresh,
    )
    .await
    .map_err(|e| ServerFnError::new(format!("Gloss failed: {e}")))
}

#[server]
pub async fn generate_graded_reader(
    user_id: i32,
    native: String,
    foreign: String,
    cefr: String,
    seed_states: Vec<i16>,
    seed_min_stability: Option<f32>,
    seed_limit: u32,
    length_words: u32,
) -> Result<GradedReaderDto, ServerFnError> {
    validate_lang(&native)?;
    validate_lang(&foreign)?;
    let foreign_name = resolve_language_name(&foreign)?;
    let db = pool()?;
    let provider = load_llm_provider()?;
    let request = wisecrow::grammar::graded_reader::GradedReaderRequest {
        native_lang: &native,
        foreign_lang: &foreign,
        foreign_lang_name: foreign_name,
        user_id,
        cefr: &cefr,
        seed_states: &seed_states,
        seed_min_stability,
        seed_limit,
        length_words,
    };
    let reader = wisecrow::grammar::graded_reader::generate(db, provider.as_ref(), &request)
        .await
        .map_err(|e| ServerFnError::new(format!("Graded reader generation failed: {e}")))?;
    Ok(GradedReaderDto::from(&reader))
}

#[server]
pub async fn preview_subtitles(
    user_id: i32,
    native: String,
    foreign: String,
    format: SubtitleFormatDto,
    content: String,
    unknown_only: bool,
    no_srs: bool,
    top_n: Option<u32>,
    gloss_unknowns: bool,
) -> Result<Vec<AnnotatedTokenDto>, ServerFnError> {
    use wisecrow::preview::annotate::{AnnotatedToken, Status};

    validate_lang(&native)?;
    validate_lang(&foreign)?;
    if content.len() > MAX_SUBTITLE_BYTES {
        return Err(ServerFnError::new(format!(
            "Subtitle file exceeds maximum size of {} MB",
            MAX_SUBTITLE_BYTES / (1024 * 1024)
        )));
    }
    let db = pool()?;

    let cues = match format {
        SubtitleFormatDto::Srt => wisecrow::preview::subtitle::parse_srt(&content),
        SubtitleFormatDto::Vtt => wisecrow::preview::subtitle::parse_vtt(&content),
        SubtitleFormatDto::Ass => wisecrow::preview::subtitle::parse_ass(&content),
    }
    .map_err(|e| ServerFnError::new(format!("Subtitle parse failed: {e}")))?;

    let tokenizer = wisecrow::preview::tokenize::for_language(&foreign)
        .map_err(|e| ServerFnError::new(format!("Tokenizer error: {e}")))?;
    let mut tokens: Vec<String> = cues.iter().flat_map(|c| tokenizer.tokenize(c)).collect();
    tokens.sort();
    tokens.dedup();

    let mut annotated: Vec<AnnotatedToken> = if no_srs {
        tokens
            .into_iter()
            .map(|t| AnnotatedToken {
                token: t,
                frequency: None,
                status: Status::Unknown,
                llm_translation: None,
            })
            .collect()
    } else {
        wisecrow::preview::annotate::annotate_tokens(db, &foreign, user_id, &tokens)
            .await
            .map_err(|e| ServerFnError::new(format!("Annotate failed: {e}")))?
    };

    if gloss_unknowns {
        let provider = load_llm_provider()?;
        let foreign_name = resolve_language_name(&foreign)?;
        let native_name = resolve_language_name(&native)?;
        wisecrow::preview::annotate::enrich_unknowns_with_llm(
            &mut annotated,
            provider.as_ref(),
            foreign_name,
            native_name,
        )
        .await
        .map_err(|e| ServerFnError::new(format!("LLM enrichment failed: {e}")))?;
    }

    let mut filtered: Vec<AnnotatedToken> = annotated
        .into_iter()
        .filter(|a| !unknown_only || matches!(a.status, Status::New | Status::Unknown))
        .collect();
    filtered.sort_by(|a, b| b.frequency.unwrap_or(0).cmp(&a.frequency.unwrap_or(0)));
    if let Some(n) = top_n {
        filtered.truncate(usize::try_from(n).unwrap_or(usize::MAX));
    }

    Ok(filtered.iter().map(AnnotatedTokenDto::from).collect())
}
