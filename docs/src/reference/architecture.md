# Architecture overview

Wisecrow is a Cargo workspace with one production crate (`wisecrow-core`) and
three satellite crates that share types through `wisecrow-dto`. Almost every
behaviour you can invoke from the CLI is also reachable as a library function,
so the `bin/wisecrow.rs` entry point is intentionally a thin orchestration
layer.

## Workspace layout

```text
wisecrow/
├── wisecrow-core/      # library + `wisecrow` binary, all production logic
│   ├── src/
│   │   ├── bin/wisecrow.rs    # CLI entry point
│   │   ├── cli.rs             # clap definitions and language whitelist
│   │   ├── config.rs          # WISECROW__ env loader, SecureString
│   │   ├── downloader.rs      # HTTP fetch + gzip/zip decompression
│   │   ├── files.rs           # OPUS URL construction
│   │   ├── ingesting/         # parse + persist pipeline
│   │   ├── srs/               # FSRS scheduler and session lifecycle
│   │   ├── dnb/               # dual n-back engine
│   │   ├── grammar/           # CEFR rules, AI seeding, quizzes, PDF import
│   │   ├── llm/               # Anthropic + OpenAI provider trait
│   │   ├── media/             # audio/image cache and prefetch
│   │   ├── sync/              # remote-pull sync client
│   │   ├── tui/               # ratatui flashcard runner and quiz
│   │   ├── frequency.rs       # Hermit Dave frequency-list import
│   │   ├── vocabulary.rs      # unlearned-words query
│   │   └── users.rs           # users repository
│   └── migrations/            # 9 SQL files run in order
├── wisecrow-dto/       # serde DTOs shared between server and clients
├── wisecrow-web/       # Dioxus fullstack web UI (experimental)
└── wisecrow-mobile/    # Dioxus mobile/desktop shell (experimental)
```

## Crate responsibilities

### `wisecrow-core`

The library half exposes the [`Langs`](../api/wisecrow-core.md#langs)
newtype, the `Ingester`, `SessionManager`, `CardManager`, `DnbEngine`, and the
`LlmProvider` trait. The binary half (`bin/wisecrow.rs`) parses CLI args,
constructs a connection pool, and dispatches to library functions.

### `wisecrow-dto`

Plain serializable structs and enums (`CardDto`, `SessionDto`, `DnbConfigDto`,
`GrammarRuleDto`, …) plus a small `SpeedController` value type used by the
session UI. The crate has no database or framework dependencies — it can be
consumed by the WASM bundle and the mobile shell without dragging in
`wisecrow-core`.

### `wisecrow-web`

Dioxus 0.7 fullstack app. The `server` cargo feature gates everything that
needs the database; the WASM bundle ships only the UI components and routes
through Dioxus' `#[server]` server-functions. The router exposes
`Home`, `LearnPage`, `NbackPage`, and `QuizPage`.

### `wisecrow-mobile`

A skeleton mobile/desktop shell. The router is similar to the web crate but
the `server_fns.rs` are client-side stubs — the real implementations live in
`wisecrow-web` and are reached over HTTP.

## Ingestion pipeline

Ingestion is the most complex flow in the system. It is a producer-consumer
pipeline over a bounded `tokio::sync::mpsc` channel.

```text
┌──────────────┐   download      ┌──────────────────┐
│ files::URLs  ├────────────────▶│  Downloader      │
└──────────────┘                 │  retry+gzip+zip  │
                                 └────────┬─────────┘
                                          │ local file path
                                          ▼
                                 ┌──────────────────┐
                                 │  CorpusParser    │
                                 │  TMX or OPUS XML │
                                 └────────┬─────────┘
                                          │ TranslationPair via mpsc
                                          ▼
                                 ┌──────────────────┐
                                 │ DatabasePersister│
                                 │  batch of 1000   │
                                 └────────┬─────────┘
                                          │
                                          ▼
                                ┌──────────────────┐
                                │  PostgreSQL      │
                                │  translations    │
                                └──────────────────┘
```

Key constants and invariants:

- **Channel bound** is `1000` (`ingesting::CHANNEL_BOUND`). Producers block when
  the consumer cannot keep up — back-pressure is intentional.
- **Batch size** is `1000` (`ingesting::persisting::TRANSLATION_BATCH_SIZE`). A
  smaller batch increases round-trips; a larger batch increases the lock window.
- **Decompression** is bounded to 1 GiB (`MAX_DECOMPRESSED_BYTES`) to defuse
  zip-bombs.
- **Path traversal** is blocked: ZIP entries that start with `/`, `\`, or
  contain `..` are dropped, and entries are rejected if they would canonicalize
  outside the extraction root.
- **Idempotency**: `INSERT … ON CONFLICT DO UPDATE SET frequency = frequency + 1`
  makes re-ingestion deterministic and additive.

## Spaced repetition

`wisecrow-core` uses [`rs-fsrs`](https://crates.io/crates/rs-fsrs) for the
algorithm. The wrapper at `srs::scheduler::CardManager` is responsible for:

1. Enumerating due cards filtered by language pair.
2. Computing the next FSRS state from a `ReviewRating`.
3. Persisting `stability`, `difficulty`, `state`, `due` back into PostgreSQL.

Sessions in `srs::session::SessionManager` glue the deck together — they
select a fixed-size cohort of cards (due first, then unlearned vocabulary),
record positions in `session_cards`, and let the user pause and resume.

The session selection ordering is deterministic and ranks **Relearning >
Learning > New > Review**, so cards in the riskiest states are surfaced first.

## Grammar mastery

Grammar is scheduled by the same FSRS code, over a different record. The whole
subsystem rests on one decision: `grammar_attempts` is the record, and every
number a learner sees is a projection of it.

**The syllabus.** Every ingested language gets a CEFR-pinned syllabus, either
from a curated inventory or generated to fill the gaps
([`ensure-syllabus`](cli-reference.md#ensure-syllabus)). Points are identified
by slug, not by title, so prose can be improved without orphaning the attempts
recorded against them.

**The bank.** Exercises are generated once and stored
([`generate-items`](cli-reference.md#generate-items)), then reviewed before
they are served ([`promote-items`](cli-reference.md#promote-items)). Generation
is an accumulating investment rather than a cost paid per request, and the
review step exists because a subtly broken item does not merely teach the wrong
thing — it writes a false signal into the learner's mastery and makes selection
drill a point they already know.

**Grading.** `wisecrow-learning::grading` is the only code that decides whether
an answer is right, and all three clients call it. Text is compared after NFC
composition, full lowercasing and whitespace collapse — accent-sensitive, since
an accent is usually the thing being taught. The server regrades every
submission against the stored item revision rather than trusting the client;
that is a correctness measure before it is an anti-tampering one, because the
stored row is the only copy all three parties can see.

**Interactions.** Within one session the first item served for a point opens an
interaction, and every submission against it belongs to that interaction.
Correct on the first submission with no hint rates `Good`; correct later or
after a hint rates `Hard`; closing without a correct answer rates `Again`.
`Easy` is unused: nothing in a cloze answer distinguishes effortless recall
from merely correct recall. The reduction is a function of the whole stream
rather than of arrival order, so an attempt arriving late from a device that
was offline — even one older than attempts already processed — re-forms its
interaction and yields what it would have had it never been delayed.

**Two projections.** FSRS answers "when is this due"; an exponentially weighted
accuracy answers "how often do you get this wrong". Retrievability is right for
scheduling and wrong for a brainmap, where red means repeated error rather than
decay. Both are derived from the one stream, so they cannot drift apart. The
band policy is in [Database schema](database-schema.md#grammar_mastery-and-grammar_review_baselines).

**Selection.** A practice session serves, in order: points that are due,
ordered by how overdue; points never attempted, in syllabus order; then the
rest by ascending accuracy, so the points a learner keeps failing surface ahead
of the ones they merely have not seen lately. Within a point, the item served
is the active one least recently shown to that learner.

**Placement.** A placement run climbs level by level on an ordinary grammar
session, so its answers are graded and recorded exactly like practice. It stops
after two consecutive levels go badly, and marks only the points it actually
asked about — the rest of the map stays unmarked rather than being inferred
from a neighbouring success.

## Dual n-back

Independent of the SRS, the `dnb` module implements an adaptive dual n-back
trainer that uses the user's own vocabulary as audio + visual stimuli. It is
two systems welded together:

- A pure scheduling core: `DnbEngine` generates trials, picks matches with a
  fixed 30% probability, and avoids accidental matches when generating
  non-match items.
- An adaptation layer: `scoring::apply_adaptation` updates `n_level` and
  `interval_ms` every 5 trials based on the rolling accuracy window. It
  terminates early when accuracy drops below 40% or after three consecutive
  windows below the starting `n_level`.

After a session the trial outcomes are folded back into the SRS
(`dnb::feedback::apply_srs_feedback`) — net-correct trials nudge cards toward
`Good`, net-incorrect toward `Again`.

## LLM integration

A minimal trait keeps providers swappable:

```rust,ignore
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError>;
    fn name(&self) -> &str;
}
```

The factory `llm::create_provider` reads `llm_provider` and `llm_api_key` from
config and returns a boxed implementation. Two are bundled: `anthropic`
(Claude Sonnet) and `openai` (GPT-4o).

Prompts are deliberately strict ("Return ONLY the JSON array") and the
seeders/exercises tolerate fenced code-blocks (```` ```json ````) when
parsing the response.

## Sync

`sync::SyncClient` walks three paginated endpoints on a remote Wisecrow
server:

| Endpoint | Mirrors |
|----------|---------|
| `/api/sync_languages`     | `languages` |
| `/api/sync_translations`  | `translations` (idempotent merge by `GREATEST(frequency)`) |
| `/api/sync_grammar_rules` | `grammar_rules` + `rule_examples` |

The cursor is the remote primary key (`after_id`); successful runs update
`sync_metadata.last_synced_at` per `(remote_url, table_name)`.

### Mobile protocol versions

The mobile protocol is negotiated, not assumed. Both ends compare the version
number for equality, so raising it would make every installed app refuse to
sync. Version 2 is therefore advertised **beside** version 1 rather than
replacing it:

| Endpoint | Answers | Adds |
|----------|--------:|------|
| `/api/mobile/capabilities` | 1 | — |
| `/api/mobile/v2/capabilities` | 2 | `GrammarBankSync`, `GrammarMasterySync`, `GrammarAttemptUpload` |

A device asks the version-2 endpoint first. A server that has never heard of it,
or that answers without all three grammar capabilities, is not a fault: the
device syncs vocabulary exactly as before and leaves grammar alone. Two out of
three is refused as well, because a device that could download the bank but not
upload its answers would lose them.

`MobileFeatureDto` carries an `Unknown` fallback variant, so a capability a
newer server advertises cannot fail the whole payload to deserialise — the very
outcome negotiating capabilities exists to avoid.

**Known limitation.** A version-1 client sees vocabulary only. Grammar practice,
placement and the brainmap require version 2 on both ends.

### Grammar sync phases

The engine's phase machine gained three steps, each resumable on its own so
that a sync killed midway restarts where it stopped:

```text
Idle → Reviews → Nback → GrammarAttempts
     → Cards → Snapshots → Deltas → GrammarBank → GrammarMastery → Finishing
```

The outbox goes before anything is downloaded, so the mastery the device then
pulls already accounts for the answers it took offline.

The device mirrors the bank with the answer included — it has to mark an answer
with no network, and a practice session that cannot say whether the learner was
right is not practice. That verdict is provisional: the server regrades every
answer when the outbox drains, and the mastery feed then overwrites whatever
the device projected for itself. The device projects only the accuracy that
colours its map, folding each answer forward with the same reduction the server
applies over the whole stream; the FSRS schedule is the server's alone.

## TUI

The TUI uses [`ratatui`](https://ratatui.rs) with a tick rate of 100 ms. The
`learn` command's `app::run_tui` owns:

- the deck and current index,
- a `SpeedController` for the auto-advance pace,
- an optional `MediaContext` for audio/image fetch when the relevant features
  are enabled.

The dual n-back command does **not** use ratatui — it uses raw crossterm
to keep latency low and avoid frame-buffer flicker on the trial timer.

## Configuration

Configuration is loaded once at start-up:

1. `dotenvy::dotenv()` reads `.env` if present.
2. `config::Config::builder().add_source(Environment::with_prefix("WISECROW").separator("__"))`
   builds a layered configuration from `WISECROW__*` variables.
3. The result is deserialised into `wisecrow_core::config::Config`. Sensitive
   fields are wrapped in `SecureString`, which zeroes its buffer on drop.

`Config::database_url()` returns either the supplied URL or one assembled
from the component fields. URL construction goes through `url::Url` so we
never use `format!()` for URL parts.

## Where to look

- The CLI command surface — [CLI reference](../reference/cli-reference.md)
- Database tables — [Database schema](../reference/database-schema.md)
- Public types per module — [API: wisecrow-core](../api/wisecrow-core.md),
  [wisecrow-dto](../api/wisecrow-dto.md)
