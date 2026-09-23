# CLI reference

The `wisecrow` binary ships with thirty subcommands. Most have a short alias
(in parentheses below) for shell ergonomics.

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
| [`frequency`](#frequency) | `fr` | yes | Rank stored translations from a word-frequency list. |
| [`learn`](#learn) | `r` | yes | Open the SRS flashcard TUI. |
| [`nback`](#nback) | `nb` | yes | Run an adaptive dual n-back session. |
| [`list-languages`](#list-languages) | `l` | no | Print the supported-language table. |
| [`seed-grammar`](#seed-grammar) | `sg` | yes + LLM | Generate grammar rules via an LLM. |
| [`import-grammar`](#import-grammar) | `ig` | yes | Import grammar rules from a JSON file. |
| [`import-pdf`](#import-pdf) | `ip` | yes | Import grammar rules extracted from a PDF. |
| [`generate-exercises`](#generate-exercises) | `ge` | yes + LLM | Generate cloze and MC quizzes from stored rules. |
| [`quiz`](#quiz) | `q` | no | Run a quiz directly from a PDF. |
| [`extract-words`](#extract-words) | — | yes | Count sentence words into word candidates. |
| [`promote-words`](#promote-words) | — | yes + LLM | Give candidates meanings and link them to learning rows. |
| [`prefetch-media`](#prefetch-media) | `pm` | yes | Pre-warm the audio/image cache. |
| [`sync`](#sync) | `s` | yes | Pull data from a remote Wisecrow. |
| [`gloss`](#gloss) | `gl` | yes + LLM | Leipzig interlinear gloss for a sentence (cached). |
| [`graded-reader`](#graded-reader) | `gr` | yes + LLM | Generate a CEFR-graded passage from learned vocab. |
| [`preview`](#preview) | `pv` | yes (+ LLM if `--gloss-unknowns`) | Annotate subtitle file tokens with corpus + SRS state. |
| [`user`](#user) | `u` | yes | Manage accounts and web login. |
| [`sync-client`](#sync-client) | `sc` | yes | Manage per-client corpus-sync API keys. |
| [`extract-phrases`](#extract-phrases) | — | yes | Mine frequent multi-word phrases into staging. |
| [`translate-phrases`](#translate-phrases) | — | yes + LLM | Translate staged phrases and promote them into decks. |
| [`score-sentences`](#score-sentences) | `ss` | yes | Rank stored sentences by the words they use. |
| [`sentence-card`](#sentence-card) | `sent` | yes + LLM | Build a sentence card around a word within reach. |
| [`gloss-deck`](#gloss-deck) | `gd` | yes + LLM | Give pending words their card presentations. |
| [`prune`](#prune) | `pr` | yes | Demote pairs whose prompt holds no recognised word. |
| [`ensure-syllabus`](#ensure-syllabus) | `es` | yes + LLM | Fill any CEFR level with no grammar points. |
| [`refresh-syllabus`](#refresh-syllabus) | `rs` | yes + LLM | Rewrite machine-generated grammar prose. |
| [`export-grammar`](#export-grammar) | `eg` | yes | Dump a language's syllabus as JSON. |
| [`generate-items`](#generate-items) | `gi` | yes + LLM | Generate quiz items for a level's grammar points. |
| [`promote-items`](#promote-items) | `pi` | yes | Review generated items before they are served. |

## Common options

`download`, `download-all`, and `ingest` share the four corpus-shaping
arguments:

| Flag | Default | Description |
|------|--------:|-------------|
| `-n`, `--native-lang` | _required_ | Your native language ISO 639 code. |
| `-f`, `--foreign-lang` | _required_ | Target language code (must differ from native). |
| `--corpus` | all | Space-delimited filter: `open_subtitles`, `cc_aligned`, `cc_matrix`, `paracrawl`, `nllb`. |
| `--max-file-size-mb` | `102400` | Per-file ceiling for downloaded content length. |
| `--max-decompressed-mb` | `8192` | Per-file ceiling for the expanded archive. |
| `--unpack` | `true` | Decompress `.gz`/`.zip` after download. |

> **Note:** Pass `--corpus "cc_matrix nllb"` (one shell argument). The clap
> definition uses a space as the value delimiter.

---

## `download`

```sh
wisecrow download -n <NATIVE> -f <FOREIGN> [--corpus ...] [--max-file-size-mb N] [--max-decompressed-mb N] [--unpack BOOL]
```

Downloads the TMX translation-memory release for every selected corpus and
optionally decompresses it. **No database is required.** The files land in
the current working directory (use `download-all` if you want a structured
output tree).

Each file runs in its own Tokio task and respects SIGTERM/SIGINT for graceful
abort.

---

## `download-all`

```sh
wisecrow download-all -n <NATIVE> -o <DIR> [--corpus ...] [--max-file-size-mb N] [--max-decompressed-mb N] [--unpack BOOL]
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
wisecrow ingest -n <NATIVE> -f <FOREIGN> [--corpus ...] [--max-file-size-mb N] [--max-decompressed-mb N] [--unpack BOOL]
wisecrow ingest --file <PATH> -n <NATIVE> -f <FOREIGN>
```

Same shape as `download`, but each file is also parsed and its translation
pairs persisted to PostgreSQL. Tasks run in parallel; the process aborts
in-flight tasks on SIGINT/SIGTERM.

| Flag | Default | Description |
|------|--------:|-------------|
| `--file` | — | Ingest this local TMX file instead of downloading. Must be decompressed; the corpus and download options are ignored. |
| `--tmx-source-lang` | native code | Exact `xml:lang` tag the file uses for the native side. Requires `--file`. |
| `--tmx-target-lang` | foreign code | Exact `xml:lang` tag the file uses for the foreign side, such as `zh_CN` for `zh`. Requires `--file`. |

OpenSubtitles publishes Chinese as `zh_CN`; a request for `zh` selects that
archive automatically and stores its pairs under `zh`. An import that accepts
no pairs, meets malformed XML or is refused by the database fails that job,
and the command exits nonzero once every job has finished.

Per-batch behaviour:

- Batches of 1000 pairs are deduplicated by `(source, target)` before insert.
- `INSERT … ON CONFLICT DO UPDATE SET frequency = frequency + 1` makes
  re-runs cumulative, not destructive.
- Languages are upserted lazily through `DatabasePersister::ensure_language`.

---

## `frequency`

```sh
wisecrow frequency --lang <CODE> [--file PATH | --from-corpus]
```

Replaces the `frequency` column with figures from a word-frequency list.
Ingestion leaves new rows at 1, and deck selection skips rows at that value, so
a corpus stays largely unusable until this has run. See
[Frequency ranking](../guides/frequency-ranking.md) for the full workflow.

| Flag | Default | Description |
|------|--------:|-------------|
| `-l`, `--lang` | _required_ | Language whose words the list holds. |
| `--file` | — | Read a local list instead of downloading Hermit Dave's. |
| `--from-corpus` | `false` | Derive counts from the stored phrases; conflicts with `--file`. |

With neither flag the command fetches
`hermitdave/FrequencyWords/…/<code>/<code>_50k.txt`, which exists for 62
languages and none of the Celtic ones. With `--file` it reads any of three
layouts, choosing per line rather than per file:

| Layout | Shape | Source |
|--------|-------|--------|
| `word count` | space-separated pair | Hermit Dave |
| `rank<TAB>word<TAB>count` | tab-separated triple | Leipzig Corpora Collection |
| `word,count` | comma-separated pair | published CSV lists |

`--from-corpus` needs no list at all: it tokenises the phrases already stored
for the language and counts the forms, which is the route for languages nobody
has published a list for. It requires a tokeniser for the language.

Matching folds case and strips edge punctuation (`.,!?;:"'¡¿`), and applies to
whichever side of a pair holds the listed language, so ingest direction is
irrelevant. Only whole phrases match: a word list ranks single-word rows, not
the sentences containing that word. A file that parses to no entries is an
error rather than a silent no-op.

---

## `extract-words`

```sh
wisecrow extract-words -n <NATIVE> -f <FOREIGN> [--limit N] [--min-occurrences N]
```

Counts the words of every corpus sentence for the pair and publishes the most
frequent as word candidates, each with up to three source sentences kept as
evidence. Counting runs inside PostgreSQL temporary tables on one dedicated
connection, so a corpus of millions of rows is never held in memory. A re-run
replaces a candidate's count rather than adding to it and removes candidates
the new selection no longer contains, except accepted ones, whose promotions
and cards remain.

| Flag | Default | Description |
|------|--------:|-------------|
| `--limit` | `500` | Candidates to publish, most frequent first (1–10,000). |
| `--min-occurrences` | `5` | Occurrences a word needs, across at least two sentences (at least 2). |

Rows that an earlier `promote-words` generated, and rows promoted as phrases,
are not corpus text and are never counted. Two further gates keep corpus noise
out of the counts: each distinct foreign sentence counts once, however many
rows repeat it, and a sentence is skipped when it holds three or more single
accented letters or, from four words upwards, a majority of one-letter tokens,
the signature of text decoded through the wrong encoding or of mis-aligned
rows from another language. The scanned figure in the log counts distinct
sentences.

---

## `promote-words`

```sh
wisecrow promote-words -n <NATIVE> -f <FOREIGN> [--limit N] [--mode pending|retry-failed|all]
```

Asks the model for a canonical presentation of each candidate, with its source
sentences as untrusted context, and links every accepted word to the
translation row that will teach it. An existing corpus row spelling the word is
reused; otherwise a generated row is created and owned by the promotion. IDs
are stable: refreshing a meaning under `--mode all` changes the text of the
same row and leaves every card and review on it untouched.

| Flag | Default | Description |
|------|--------:|-------------|
| `--limit` | `200` | Candidates to attempt in one run (1–1,000). |
| `--mode` | `pending` | `pending` attempts new candidates; `retry-failed` adds those whose last attempt failed; `all` refreshes every candidate. |

The run reports attempted, accepted, rejected, failed and stale counts and
exits nonzero when any candidate failed or went stale, so a scripted pilot
cannot pass on a partial result. Generation happens outside the database
transaction; each candidate then commits on its own, so a failure part-way
keeps every earlier success.

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
wisecrow prefetch-media -n <NATIVE> -f <FOREIGN> [--limit N] [--offset N] [--max-bytes N] [--dry-run] [--audio BOOL] [--images BOOL]
```

Prepares the on-disk cache for a finite slice of the pair's preparation
deck: the first 10,000 ranked words that carry a current presentation and
the first 2,000 ranked phrases, interleaved to at most 10,000 entries. The
deck is the same however much of it a run asks for, so `--offset` names a
stable position until the ranking or presentations change; after an
import, ranking or enrichment, preview again from offset zero.

```sh
wisecrow prefetch-media -n en -f fr --limit 100 --offset 0 --dry-run
wisecrow prefetch-media -n en -f fr --limit 100 --offset 0 --max-bytes 67108864
wisecrow prefetch-media -n en -f br --limit 100 --audio=false --images=true --dry-run
```

| Flag | Default | Description |
|------|--------:|-------------|
| `--limit` | `100` | Deck entries in this run (1–5,000). |
| `--offset` | `0` | Deck position to start from; offset plus limit may not pass 10,000. |
| `--max-bytes` | `67108864` | Generated payload bytes the run may admit (1 byte–5 GiB). |
| `--dry-run` | off | Read-only preview: reports cached and missing media, generates nothing, applies no migrations and creates no cache directory. |
| `--audio` | `true` | Prepare speech; `--audio=false` skips it. |
| `--images` | `true` | Prepare images; `--images=false` skips it. |

The budget counts new payload bytes admitted during this invocation and
nothing else: cache hits cost nothing, filesystem overhead and completed
provider transfers are not counted, and a charge is never refunded, so a
payload admitted and then lost to a publication error still counts. Once a
payload is refused, later misses are reported as budget-exhausted without a
provider call, while probes continue so hits stay identifiable. At most four
requests are outstanding at once, and presentations are loaded one hundred
at a time.

Audio uses Microsoft Edge TTS (no API key), or CereProc where configured.
Images need at least one stock-photo key (`WISECROW__UNSPLASH_API_KEY`,
`WISECROW__PEXELS_API_KEY` and/or `WISECROW__PIXABAY_API_KEY`; optional
`WISECROW__IMAGE_PROVIDER`). A medium this build or configuration cannot
produce is reported as unsupported rather than ready: a preview says so,
and an execution is refused before any call, so a language without a voice
(Breton, for instance) is prepared with `--audio=false --images=true`.

The run prints the range, per-medium counters, admitted and generated bytes
and the next offset, then exits nonzero when any medium failed, was refused
by the budget or is unsupported, naming the first affected IDs so the same
range can be retried before advancing. A preview's missing entries are
expected and do not fail the run.

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

---

## `user`

```sh
wisecrow user add --email <EMAIL> --display-name <NAME> [--admin]
wisecrow user list
wisecrow user passwd --email <EMAIL>
wisecrow user disable --email <EMAIL>
```

Manages accounts for the web application; there is no public signup, so the
first admin is created this way. `add` and `passwd` prompt for a password
unless `WISECROW__INIT_PASSWORD` is set, which allows non-interactive
provisioning. `disable` clears the password and revokes live sessions.

---

## `sync-client`

```sh
wisecrow sync-client add --name <NAME>
wisecrow sync-client list
wisecrow sync-client revoke --name <NAME>
```

Issues per-client keys for the corpus-sync endpoints. `add` prints the key
once. Pullers send it as the `x-api-key` header; keys are individually
revocable and compared in constant time. See
[Sync workflow](../guides/sync-workflow.md).

---

## `extract-phrases`

```sh
wisecrow extract-phrases --lang <CODE>
```

Mines the corpus for frequent multi-word phrases and stages them. Staging is
separate from the decks so that a phrase is translated and reviewed before a
learner ever meets it; [`translate-phrases`](#translate-phrases) is what moves
it on.

---

## `translate-phrases`

```sh
wisecrow translate-phrases --lang <CODE> --native-lang <CODE> [--limit N] [--refresh]
```

Translates staged phrases with the configured model and promotes them into the
decks. `--limit` caps how many are sent in one run (default 100). `--refresh`
re-glosses phrases already translated, updating the linked rows in place rather
than adding duplicates.

---

## `score-sentences`

```sh
wisecrow score-sentences --lang <CODE>
```

Scores every stored sentence of a language by the words it uses, which is what
[`sentence-card`](#sentence-card) draws on. The language's words must already
be ranked — run [`frequency`](#frequency) with `--from-corpus` first — because
an unranked word carries no weight to score with.

---

## `sentence-card`

```sh
wisecrow sentence-card --lang <CODE> --native-lang <CODE> --user-id <ID> [--word <WORD>]
```

Builds a sentence card around one word the learner is close to knowing. With
no `--word`, the next word the deck would serve is used, which is the loop the
word deck and the sentence deck are meant to form.

---

## `gloss-deck`

```sh
wisecrow gloss-deck --lang <CODE> --native-lang <CODE> [--limit N] [--offset N] [--dry-run]
```

Gives pending words the presentations their cards are built from. `--limit`
bounds one window (1–1000); rows that are punctuation, digits or the wrong
script are reported and skipped, so fewer than the limit may be enriched.
`--offset` steps past a rejected window during an inspection pass; restart from
zero once entries have been written, since accepted words leave the pending
list. `--dry-run` lists the window without calling the model.

---

## `prune`

```sh
wisecrow prune --lang <CODE> [--native-lang <CODE>] [--dry-run]
```

Demotes translation pairs whose prompt holds no recognised word — corpus
corruption such as `Bthey` or `andthatthe`, which no ordering rule inside the
deck query can catch. Run it with `--dry-run` first: a corpus that turns out to
be mostly the wrong language loses most of its rows, and that is better seen
than discovered.

---

## `ensure-syllabus`

```sh
wisecrow ensure-syllabus --lang <CODE>
wisecrow ensure-syllabus --all
```

Fills any CEFR level that holds no grammar points for a language, so that every
ingested language has a syllabus to practise against rather than only the ones
someone remembered to seed. `--all` does the same for every language already
present in the corpus. Points it generates are marked `llm`; a level that
already holds points is left alone.

---

## `refresh-syllabus`

```sh
wisecrow refresh-syllabus --lang <CODE>
```

Rewrites the title and explanation of machine-generated grammar points, for
when a better model or a better prompt would produce clearer prose. Points from
a curated inventory are never touched: their wording is the reason they were
curated.

---

## `export-grammar`

```sh
wisecrow export-grammar --lang <CODE> [--out FILE]
```

Dumps a language's syllabus as JSON, in the shape
[`import-grammar`](#import-grammar) accepts, for diffing or backup. Prints to
standard output when `--out` is omitted.

---

## `generate-items`

```sh
wisecrow generate-items --lang <CODE> --level <CEFR> [--per-rule N]
```

Generates quiz items for every grammar point at a level and stores them as
candidates. Nothing generated here is served: items enter the bank at
`candidate` status and reach learners only through
[`promote-items`](#promote-items). Generation is therefore an accumulating
investment rather than a cost paid on every request, and a subtly wrong item
gets a human reading before it can write a false signal into anyone's mastery.

`--per-rule` sets how many items to ask the model for per point (default 8).

---

## `promote-items`

```sh
wisecrow promote-items --lang <CODE> [--level <CEFR>] --list [--limit N]
wisecrow promote-items --lang <CODE> --accept <ID>...
wisecrow promote-items --lang <CODE> --reject <ID>... [--reason TEXT]
wisecrow promote-items --lang <CODE> --retire <ID>...
```

Reviews what [`generate-items`](#generate-items) produced. `--list` prints the
waiting candidates and changes nothing. `--accept` makes them servable,
`--reject` refuses them with a reason, and `--retire` withdraws an active item
without deleting it — an attempt uploaded from a device that has been offline
for a fortnight must still be gradeable against the item as the learner saw it,
so nothing in the bank is ever hard-deleted.
