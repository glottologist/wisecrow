# CLI reference

The `wisecrow` binary ships with sixteen subcommands. Every command has a
short alias (in parentheses below) for shell ergonomics.

## Synopsis

```text
wisecrow <COMMAND> [ARGS...]
```

Run `wisecrow --help` or `wisecrow <COMMAND> --help` for the latest argument
list.

## Subcommands at a glance

| Command | Alias | Touches DB | Purpose |
|---------|-------|:---------:|---------|
| [`download`](#download) | `d` | no | Fetch corpus files only. |
| [`download-all`](#download-all) | `da` | no | Fetch every language pair against one native lang. |
| [`ingest`](#ingest) | `i` | yes | Fetch + parse + persist translations. |
| [`learn`](#learn) | `r` | yes | Open the SRS flashcard TUI. |
| [`nback`](#nback) | `nb` | yes | Run an adaptive dual n-back session. |
| [`list-languages`](#list-languages) | `l` | no | Print the supported-language table. |
| [`seed-grammar`](#seed-grammar) | `sg` | yes + LLM | Generate grammar rules via an LLM. |
| [`import-grammar`](#import-grammar) | `ig` | yes | Import grammar rules from a JSON file. |
| [`import-pdf`](#import-pdf) | `ip` | yes | Import grammar rules extracted from a PDF. |
| [`generate-exercises`](#generate-exercises) | `ge` | yes + LLM | Generate cloze and MC quizzes from stored rules. |
| [`quiz`](#quiz) | `q` | no | Run a quiz directly from a PDF. |
| [`prefetch-media`](#prefetch-media) | `pm` | yes | Pre-warm the audio/image cache. |
| [`sync`](#sync) | `s` | yes | Pull data from a remote Wisecrow. |
| [`gloss`](#gloss) | `gl` | yes + LLM | Leipzig interlinear gloss for a sentence (cached). |
| [`graded-reader`](#graded-reader) | `gr` | yes + LLM | Generate a CEFR-graded passage from learned vocab. |
| [`preview`](#preview) | `pv` | yes (+ LLM if `--gloss-unknowns`) | Annotate subtitle file tokens with corpus + SRS state. |

## Common options

`download`, `download-all`, and `ingest` share the four corpus-shaping
arguments:

| Flag | Default | Description |
|------|--------:|-------------|
| `-n`, `--native-lang` | _required_ | Your native language ISO 639 code. |
| `-f`, `--foreign-lang` | _required_ | Target language code (must differ from native). |
| `--corpus` | all | Space-delimited filter: `open_subtitles`, `cc_matrix`, `nllb`. |
| `--max-file-size-mb` | `102400` | Per-file ceiling for downloaded content length. |
| `--unpack` | `true` | Decompress `.gz`/`.zip` after download. |

> **Note:** Pass `--corpus "cc_matrix nllb"` (one shell argument). The clap
> definition uses a space as the value delimiter.

---

## `download`

```sh
wisecrow download -n <NATIVE> -f <FOREIGN> [--corpus ...] [--max-file-size-mb N] [--unpack BOOL]
```

Downloads TMX and OPUS XML alignment files for every selected corpus and
optionally decompresses them. **No database is required.** The files land in
the current working directory (use `download-all` if you want a structured
output tree).

Each file runs in its own Tokio task and respects SIGTERM/SIGINT for graceful
abort.

---

## `download-all`

```sh
wisecrow download-all -n <NATIVE> -o <DIR> [--corpus ...] [--max-file-size-mb N] [--unpack BOOL]
```

Downloads corpora for **every supported foreign language** against the given
native language, into `<DIR>/<native>-<foreign>/`. Useful for offline mirrors
and CI fixture preparation.

| Flag | Required | Description |
|------|:--------:|-------------|
| `-n`, `--native-lang` | yes | Native language code. |
| `-o`, `--output-dir`  | yes | Output directory; created if missing. The path is canonicalised before any sub-directories are joined to defend against traversal. |

---

## `ingest`

```sh
wisecrow ingest -n <NATIVE> -f <FOREIGN> [--corpus ...] [--max-file-size-mb N] [--unpack BOOL]
```

Same shape as `download`, but each file is also parsed and its translation
pairs persisted to PostgreSQL. Tasks run in parallel; the process aborts
in-flight tasks on SIGINT/SIGTERM.

Per-batch behaviour:

- Batches of 1000 pairs are deduplicated by `(source, target)` before insert.
- `INSERT … ON CONFLICT DO UPDATE SET frequency = frequency + 1` makes
  re-runs cumulative, not destructive.
- Languages are upserted lazily through `DatabasePersister::ensure_language`.

---

## `learn`

```sh
wisecrow learn -n <NATIVE> -f <FOREIGN> [--deck-size N] [--speed-ms MS] [--user-id N]
```

Opens the flashcard TUI. Defaults: deck of 50 cards, 3000 ms auto-advance,
user ID 1.

| Flag | Default | Description |
|------|--------:|-------------|
| `--deck-size` | `50` | Number of cards in the session. Filled with due cards first, then unlearned vocabulary by frequency. |
| `--speed-ms` | `3000` | Auto-advance interval in milliseconds; clamped to `[500, 10000]`. |
| `--user-id` | `1` | Users are scoped per FK; create more with the schema `users` table. |

If a paused session exists for `(user_id, native, foreign)` it is resumed.
Press `q` to pause; the next invocation picks up at the same card index.

---

## `nback`

```sh
wisecrow nback -n <NATIVE> -f <FOREIGN> [--mode MODE] [--n-level N] [--user-id N]
```

Runs an adaptive dual n-back session using your stored vocabulary as
stimuli. Requires at least 8 ingested pairs for the language combination.

| Flag | Default | Values |
|------|---------|--------|
| `--mode` | `audio_written` | `audio_written`, `word_translation`, `audio_image` |
| `--n-level` | `2` | `1`–`9` (clamped to range) |
| `--user-id` | `1` | FK into `users` |

Controls during a trial: `[A]` audio match, `[L]` visual match,
`[Enter]` submit, `[Q]` quit. The engine adapts every 5 trials and
terminates early when accuracy is consistently below 40 %.

---

## `list-languages`

```sh
wisecrow list-languages
```

Prints the 102 supported language codes with their human-readable names.
Useful before invoking `ingest` to confirm a code.

---

## `seed-grammar`

```sh
wisecrow seed-grammar --lang <CODE> --levels A1,A2,B1,...
```

Generates grammar rules for the supplied CEFR levels via the configured LLM
provider. 15 rules are requested per level; the prompt asks for at least one
correct and one incorrect example per rule.

Requires `WISECROW__LLM_PROVIDER` and `WISECROW__LLM_API_KEY`.

---

## `import-grammar`

```sh
wisecrow import-grammar --lang <CODE> --file rules.json
```

Imports grammar rules from a JSON file matching the
`wisecrow_dto::GrammarRuleImport` shape. The import flow is upsert by
`(language_id, cefr_level_id, title)` so re-running with corrected data is
safe.

Example file shape:

```json
[
  {
    "title": "Present tense of regular -ar verbs",
    "explanation": "Spanish verbs ending in -ar follow a fixed pattern: drop -ar and add -o, -as, -a, -amos, -áis, -an.",
    "cefr_level": "A1",
    "examples": [
      { "sentence": "Hablo español.", "translation": "I speak Spanish.", "is_correct": true },
      { "sentence": "Hablamos español.", "translation": "We speak Spanish.", "is_correct": true }
    ]
  }
]
```

---

## `import-pdf`

```sh
wisecrow import-pdf --lang <CODE> --level <CEFR> --file path/to/grammar.pdf
```

Extracts text from a PDF and stores each parsed rule with `source = 'pdf'`.
The extraction is best-effort — see [Grammar workflows](../guides/grammar-workflows.md)
for tips on cleaning the imported rules.

---

## `generate-exercises`

```sh
wisecrow generate-exercises --lang <CODE> --level <CEFR> [--count N]
```

Generates a mix of cloze and multiple-choice quizzes from the stored grammar
rules at the chosen CEFR level. Requires the LLM configuration. The output
is printed to stdout (suitable for piping into `jq`).

| Flag | Default |
|------|--------:|
| `--count` | `20` |

---

## `quiz`

```sh
wisecrow quiz --pdf-path path/to/quiz.pdf [--num-questions N]
```

Reads a PDF, generates `N` quiz items inline, and runs them in the terminal.
This command does **not** require a database — it is the lightest path to
trying the quiz UI.

| Flag | Default |
|------|--------:|
| `--num-questions` | `20` |

---

## `prefetch-media`

```sh
wisecrow prefetch-media -n <NATIVE> -f <FOREIGN> [--audio BOOL] [--images BOOL]
```

Walks the deck and warms the on-disk cache for audio and/or image media.
Audio uses Microsoft Edge TTS (no API key); images need
`WISECROW__UNSPLASH_API_KEY`.

This command is a no-op for media types whose feature is not compiled in.

---

## `sync`

```sh
wisecrow sync --remote https://wisecrow.example.com [--api-key KEY]
```

Pulls `languages`, `translations`, and `grammar_rules` from a remote
Wisecrow instance, paginated by primary key.
`--api-key` is sent as the `x-api-key` HTTP header.
`sync_metadata.last_synced_at` is updated for each table on success.

---

## `gloss`

```sh
wisecrow gloss --sentence "<TEXT>" --lang <CODE> [--refresh]
```

Produces a Leipzig interlinear gloss of `<TEXT>` in the given language.
Result is cached in the `glosses` table (SHA-256 of sentence × lang_code).
`--refresh` discards the cached value and re-prompts the LLM.

Requires `WISECROW__LLM_PROVIDER` and `WISECROW__LLM_API_KEY`.

See [Leipzig glossing](../guides/glossing.md) for the full guide.

---

## `graded-reader`

```sh
wisecrow graded-reader -n <NATIVE> -f <FOREIGN> --cefr <LEVEL>
                       [--seed-states 2[,3]] [--seed-min-stability F]
                       [--seed-limit 30] [--length-words 200]
                       [--format md|html] [--output PATH]
```

Generates a personalised passage at the given CEFR level seeded from cards
you've learned (FSRS state filter via `--seed-states`).
Produces Markdown by default; pass `--format html` for a self-contained
viewable page.

Requires `WISECROW__LLM_PROVIDER` and `WISECROW__LLM_API_KEY`.

See [Graded reader](../guides/graded-reader.md) for the full guide.

---

## `preview`

```sh
wisecrow preview --file <PATH> -n <NATIVE> -f <FOREIGN>
                 [--unknown-only] [--no-srs] [--top-n N] [--gloss-unknowns]
```

Parses a `.srt`, `.vtt`, `.ass`, or `.ssa` subtitle file, tokenises with the
language-appropriate segmenter (jieba/lindera/kham/whitespace), and prints a
frequency-sorted table tagged with each token's SRS status.
`--gloss-unknowns` LLM-translates corpus-misses inline.

See [Subtitle preview](../guides/preview-subtitles.md) for the full guide.
