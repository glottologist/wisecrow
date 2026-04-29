# Grammar workflows

Wisecrow stores graded grammar rules per language and CEFR level. Three
ingestion paths are available — pick whichever matches the source you have.

| Path | Best for | Source field |
|------|----------|--------------|
| `seed-grammar` | Bootstrap a brand-new language. | `ai` |
| `import-grammar` | Curated rules you already wrote in JSON. | `manual` |
| `import-pdf` | Course materials, textbook PDFs. | `pdf` |

## Bootstrap with an LLM

`seed-grammar` asks the configured LLM provider for 15 rules per CEFR level.

```sh
export WISECROW__LLM_PROVIDER=anthropic
export WISECROW__LLM_API_KEY=sk-ant-...
wisecrow seed-grammar --lang es --levels A1,A2,B1,B2,C1,C2
```

Behind the scenes:

1. The driver builds one `grammar_seed_prompt(language_name, level, count)`
   per level (`wisecrow-core/src/llm/prompts.rs:3`).
2. It calls `LlmProvider::generate(prompt, 4096)`.
3. `seed_grammar::parse_llm_json` strips Markdown fences and deserialises
   the array.
4. Each rule is upserted via `RuleRepository::upsert_rule` keyed on
   `(language_id, cefr_level_id, title)`.

The upsert key makes re-running safe — refining a level's rules just
overwrites the previous run.

## Import from JSON

`import-grammar` consumes a `[GrammarRuleImport]` JSON array:

```json
[
  {
    "title": "Definite articles",
    "explanation": "Spanish has four definite articles: el, la, los, las.",
    "cefr_level": "A1",
    "examples": [
      { "sentence": "El libro está en la mesa.", "translation": "The book is on the table.", "is_correct": true },
      { "sentence": "La libro está en la mesa.", "translation": "(incorrect: noun is masculine)", "is_correct": false }
    ]
  }
]
```

Field reference: `wisecrow-dto/src/lib.rs:126`.

```sh
wisecrow import-grammar --lang es --file rules.json
```

The flow shares the upsert key with `seed-grammar`, so a manual edit can
override an AI-seeded rule simply by reusing its title.

## Import from PDF

`import-pdf` runs `pdf-extract` over the file, splits it into sections, and
upserts a rule per detected paragraph. It is best-effort:

```sh
wisecrow import-pdf --lang es --level B1 --file ./grammar/spanish-b1.pdf
```

Two things to know:

1. The extracted titles are clipped at 100 characters with a UTF-8 boundary
   check, so long headings get truncated rather than corrupting the row.
2. PDF examples are imported as `is_correct = true` because there is no
   reliable signal in unstructured text.

If the result is messy, export the rules with a SQL query, edit them, and
re-import via `import-grammar` — `manual` overrides `pdf` on the next
upsert.

## Generate quizzes from rules

Once rules exist for a `(language, level)` pair, you can ask the LLM to
turn them into exercises:

```sh
wisecrow generate-exercises --lang es --level A2 --count 30
```

Each exercise has a `rule_id` linking back to the source rule (so you can
display "this question tests rule X" in a UI). The flow:

1. Load all rules for the level via `RuleRepository::rules_for_level`.
2. Build the `exercise_generation_prompt` (a strict spec asking for a JSON
   array of cloze and multiple-choice items).
3. Parse the response, tolerating fenced code-blocks.
4. Truncate to the requested count, splitting evenly between cloze and MC.

The output is printed to stdout. Pipe it into `jq` if you want to feed it
into another tool.

## Inspect what's stored

```sql
SELECT cl.code AS level, gr.source, count(*) AS rules
FROM grammar_rules gr
JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
GROUP BY level, gr.source
ORDER BY level, source;
```

Useful for spotting gaps before you start drilling.

## Cleaning up

To drop everything and start over for a language:

```sql
DELETE FROM grammar_rules
WHERE language_id = (SELECT id FROM languages WHERE code = 'es');
-- rule_examples cascade-delete via FK
```
