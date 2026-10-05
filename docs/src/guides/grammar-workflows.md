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

Every path on this page shares that provider, so the choice of provider is
also a choice of who pays for the run. Setting `WISECROW__LLM_PROVIDER` to
`claude-cli` and `WISECROW__LLM_API_KEY` to a `claude setup-token` OAuth token
runs the same prompts through the locally installed Claude Code binary under a
Claude subscription, which matters most for `import-pdf`: a shelf of textbooks
is thousands of calls. The deployment side of that switch is described in
[Billing model calls to a subscription](https://github.com/glottologist/wisecrow/blob/main/DEPLOYMENT.md#billing-model-calls-to-a-subscription).

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

For the six-language calypso shelf, use the
[sync and batch-run instructions](../../../DEPLOYMENT.md#sync-grammar-files-and-run-imports).
`scripts/import-grammar.sh` imports the eligible documents and reports A1–C2
coverage; `scripts/prefetch-grammar-audio.sh` prepares and verifies their audio.

Before importing anything, screen the document. A PDF built from page scans holds
no text at all, and an import from one produces nothing; `check-pdf` reports the
characters per page and the passages the extractor keeps, and fails the run when a
document cannot supply material:

```sh
wisecrow check-pdf --file ./grammar/es | tail -20
```

`import-pdf` reads the file with `pdf-extract`, falling back to poppler's
`pdftotext` when that returns too little to be prose, splits the text into
passages, and asks the model for rules at the requested levels. With
`--incremental`, every prose chunk is visited without a total rule ceiling.
Proposals need at least 120 characters and two sentences of explanation, a
correct and an incorrect example, and a page the prompt carried. Exact matches
are skipped, and a second model pass screens for rewordings against the entire
language syllabus. Existing rules and example IDs are preserved.
Documents live one directory per
language, so the language comes from the path and a whole language imports in
one command:

```sh
wisecrow import-pdf --file ./grammar/es/yo-puedo-1-2021.pdf --level A1 --max-rules 5 --dry-run
wisecrow import-pdf --incremental --file ./grammar/es              # every eligible book and level
wisecrow import-pdf --incremental --file ./grammar/es --level B1   # only the books that cover B1
```

Three things to know:

1. Run with `--dry-run` first. It costs the model call but writes nothing, and
   prints every point as it would be stored, so a document that yields poor
   points is found before it touches the syllabus. This is a bounded sample;
   `--incremental` cannot be combined with `--dry-run` or `--max-rules`.
2. `grammar/SOURCES.md` decides what may be sent and where it is read. A
   document is read to the model only when its row's `Synthesis` cell says
   `yes` -- the two standards documents say `no`, because their terms do not
   allow reproduction to a third party, and `--force` is for a locally hosted
   model only -- and only at the levels its `Levels` cell names. Without
   `--level` the whole shelf is walked lowest level first; with it, only the
   rows naming that level.
3. Every stored point cites its document and page. Incremental imports record
   progress in PostgreSQL by file contents, language and level. Reruns skip
   completed files even if renamed, resume failures and process changed files
   or new levels. Old imports without progress records receive one catch-up
   scan. Extraction continues until no further source points are found, including
   when an entire batch already exists in the syllabus. Semantic deduplication is
   model judgment and should be reviewed. Without `--incremental`, the legacy thirty-PDF-rule
   target still applies.

Incremental imports save source extraction responses in PostgreSQL's
`grammar_llm_cache` table. Extraction depends on the source passages, language,
level and points previously extracted from those passages. Adding syllabus rules
does not change this request. A resumed chunk replays its saved batches and skips
rules already committed. Response reuse logs `PDF LLM cache hit; skipped ... request`.

The `grammar_rule_comparisons` table separately stores semantic duplicate decisions
between pairs of rule texts. A saved match skips a candidate while the matching
rule still exists with the same title and explanation. Saved nonmatches skip those
pairs; only unseen or changed pairs are sent for review. This works even when the
syllabus grows or review batch boundaries move. Logs report saved comparisons and
the number of previously unchecked pairs sent for review. Examples and citations
do not affect semantic comparison identity.

A review request numbers its own candidates from zero, because a later batch
carries only those still undecided. An answer that names a candidate nobody asked
about, decides one twice, or matches a rule it was not shown is rejected, and the
log names which of those happened along with the start of the answer. The rejection
then goes back to the model, which is asked once more before the import gives up on
that request, as an answer that is not a JSON array of points already is. A provider
that failed outright is not asked again.

Both caches distinguish provider/model settings; comparison decisions also
distinguish language and review version. Changed source text, level or extraction
prompt requires new extraction. First-time extraction and unseen comparisons still
use the model. Provider errors, invalid responses and extraction batches with
quality refusals remain retryable and are not cached. Saved responses and completed
comparisons survive a later import rollback.

Migration 038 adds the comparison table. Existing rules and completed import
progress remain intact. Older syllabus-dependent extraction responses cannot be
reused by the new source prompt, so unfinished chunks need an initial source pass.
Dry runs and legacy top-up imports do not use these caches.

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
