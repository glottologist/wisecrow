/// Builds a prompt for generating grammar rules for a given language and CEFR level.
#[must_use]
pub fn grammar_seed_prompt(language_name: &str, cefr_level: &str, count: u32) -> String {
    format!(
        r#"Generate exactly {count} grammar rules for {language_name} at CEFR level {cefr_level}.

Return a JSON array where each element has this structure:
{{
  "title": "Short rule title (e.g. 'Present Simple Conjugation')",
  "explanation": "Clear explanation of the rule (2-4 sentences)",
  "examples": [
    {{
      "sentence": "An example sentence demonstrating the rule",
      "translation": "English translation of the sentence",
      "is_correct": true
    }},
    {{
      "sentence": "An incorrect example showing a common mistake",
      "translation": "English translation",
      "is_correct": false
    }}
  ]
}}

Requirements:
- Each rule must have at least 2 examples (1 correct, 1 incorrect)
- Rules should be specific and actionable, not vague
- Examples should be realistic sentences a learner would encounter
- Explanations should reference the specific grammatical structure
- Return ONLY the JSON array, no surrounding text"#
    )
}

/// Builds a prompt for an LLM to produce a CEFR-graded passage in target language.
#[must_use]
pub fn graded_reader_prompt(
    seed_vocab: &[(&str, &str)],
    cefr_level: &str,
    target_lang_name: &str,
    length_words: u32,
) -> String {
    let seed_lines: Vec<String> = seed_vocab
        .iter()
        .map(|(foreign, native)| format!("- {foreign} ({native})"))
        .collect();
    format!(
        r#"Write a short passage in {target_lang_name} at CEFR level {cefr_level}, approximately {length_words} words long. The passage should reuse most of the seed vocabulary listed below; you may introduce a small amount of new vocabulary appropriate for the level.

Seed vocabulary the reader knows (foreign — native gloss):
{seeds}

Return a JSON object with this exact shape:
{{
  "passage": "the passage text in {target_lang_name}",
  "glossary": [
    {{"word": "<foreign>", "translation": "<native>"}}
  ]
}}

- Include in the glossary every word from the passage that is NOT in the seed list.
- Keep grammar at level {cefr_level} or below.
- Return ONLY the JSON object, no surrounding text."#,
        seeds = seed_lines.join("\n"),
    )
}

/// Builds a prompt for translating a list of foreign-language words into the
/// learner's native language. Used by `wisecrow preview --gloss-unknowns`.
#[must_use]
pub fn unknown_words_prompt(
    words: &[String],
    foreign_lang_name: &str,
    native_lang_name: &str,
) -> String {
    let word_list = words
        .iter()
        .enumerate()
        .map(|(i, w)| format!("{}. {w}", i.saturating_add(1)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"Provide concise {native_lang_name} translations for these {foreign_lang_name} words.

Words:
{word_list}

Return a JSON object with this exact shape:
{{
  "glosses": [
    {{"word": "<foreign>", "translation": "<{native_lang_name}>"}}
  ]
}}

- One entry per input word, in the same order.
- Translations should be the most common/canonical sense, 1-3 words.
- Return ONLY the JSON object, no surrounding text."#
    )
}

/// Builds a prompt for generating a Leipzig interlinear gloss of a sentence.
#[must_use]
pub fn gloss_prompt(sentence: &str, language_name: &str) -> String {
    format!(
        r#"Produce a Leipzig interlinear gloss of the following {language_name} sentence.

Sentence: {sentence}

Format your response as exactly four lines:
1. The original sentence (surface forms).
2. Morpheme-level breakdown with hyphens between morphemes.
3. Gloss tags (one per morpheme) — use standard Leipzig abbreviations (NOM, ACC, GEN, 1SG, 3PL, PST, etc.). Use `=` for clitics and `-` for affixes.
4. A free English translation.

Return only those four lines, no surrounding prose."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gloss_prompt_contains_language_and_sentence() {
        let p = gloss_prompt("Меня зовут Иван", "Russian");
        assert!(p.contains("Russian"));
        assert!(p.contains("Меня зовут Иван"));
        assert!(p.contains("Leipzig"));
        assert!(p.contains("morpheme"));
    }

    #[test]
    fn gloss_prompt_handles_empty_sentence() {
        let p = gloss_prompt("", "Spanish");
        assert!(p.contains("Spanish"));
    }

    #[test]
    fn graded_reader_prompt_contains_level_and_seeds() {
        let seed = vec![("casa", "house"), ("perro", "dog")];
        let p = graded_reader_prompt(&seed, "B1", "Spanish", 200);
        assert!(p.contains("B1"));
        assert!(p.contains("Spanish"));
        assert!(p.contains("200"));
        assert!(p.contains("casa"));
        assert!(p.contains("perro"));
        assert!(p.contains("passage"));
        assert!(p.contains("glossary"));
    }

    #[test]
    fn graded_reader_prompt_handles_empty_seed_list() {
        let p = graded_reader_prompt(&[], "A1", "French", 100);
        assert!(p.contains("A1"));
        assert!(p.contains("French"));
    }

    #[test]
    fn unknown_words_prompt_lists_words_and_languages() {
        let words = vec!["casa".to_owned(), "perro".to_owned()];
        let p = unknown_words_prompt(&words, "Spanish", "English");
        assert!(p.contains("Spanish"));
        assert!(p.contains("English"));
        assert!(p.contains("casa"));
        assert!(p.contains("perro"));
        assert!(p.contains("glosses"));
    }
}
