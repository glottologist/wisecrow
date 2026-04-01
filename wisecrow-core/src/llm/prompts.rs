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
