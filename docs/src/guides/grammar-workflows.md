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

Before importing anything, screen the document. A PDF built from page scans holds
no text at all, and an import from one produces nothing; `check-pdf` reports the
characters per page and the passages the extractor keeps, and fails the run when a
document cannot supply material:

```sh
wisecrow check-pdf --file ./grammar/es | tail -20
```

`import-pdf` reads the file with `pdf-extract`, falling back to poppler's
`pdftotext` when that returns too little to be prose, splits the text into
passages, and asks the model for the points the requested level is short of,
naming the points it already holds. What comes back is gated -- two sentences of
explanation, a correct and an incorrect example, a page the prompt carried -- and
placed the way `seed-grammar` places points, so an import can fill a level but
never moves a point already placed at another. Documents live one directory per
language, so the language comes from the path and a whole language imports in
one command:

```sh
wisecrow import-pdf --file ./grammar/es/yo-puedo-1-2021.pdf --level A1 --max-rules 5 --dry-run
wisecrow import-pdf --file ./grammar/es              # the whole shelf, each book at its own levels
wisecrow import-pdf --file ./grammar/es --level B1   # only the books that cover B1
```

Three things to know:

1. Run with `--dry-run` first. It costs the model call but writes nothing, and
   prints every point as it would be stored, so a document that yields poor
   points is found before it touches the syllabus.
2. `grammar/SOURCES.md` decides what may be sent and where it is read. A
   document is read to the model only when its row's `Synthesis` cell says
   `yes` -- the two standards documents say `no`, because their terms do not
   allow reproduction to a third party, and `--force` is for a locally hosted
   model only -- and only at the levels its `Levels` cell names. Without
   `--level` the whole shelf is walked lowest level first; with it, only the
   rows naming that level.
3. Every stored point cites its source: `source_ref` holds the document and
   page, and `source = 'pdf'`. A level holds up to thirty document-backed
   points beside its fifteen seeded ones, asked for fifteen at a time; a run
   over a level already at that target asks the model for nothing, and a
   proposal that matches a seeded point's slug at the same level is held, not
   written over it.

A point that reads badly is edited like any other: export the level with
`export-grammar`, correct it, and re-import via `import-grammar` -- `manual`
overrides `pdf` on the next upsert.

## Voice the examples

Grammar practice on the web shows a point's correct examples after an answer,
each with a play button. The clip is generated on first play if it has to be,
but a level's worth of sentences is cheap to prepare ahead of time:

```sh
wisecrow prefetch-grammar-audio --lang es --dry-run   # what is missing
wisecrow prefetch-grammar-audio --lang es             # voice it
```

Only correct examples are spoken. Clips are keyed by the sentence rather
than the example row, so re-seeding or re-importing a point costs nothing
for sentences that did not change; `--prune` reclaims clips whose sentences
no longer appear anywhere.

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
