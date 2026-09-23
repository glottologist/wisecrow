# wisecrow-mobile

`wisecrow-mobile` is the Dioxus shell for Android and the desktop. Unlike
`wisecrow-web` it is offline first: every page reads the device's own SQLite
database, and a sync engine reconciles that database with the server when a
connection happens to be there. Nothing the learner does waits on the network.

## Crate structure

```text
wisecrow-mobile/
├── migrations/          # SQLite migrations, applied on open
├── src/
│   ├── main.rs          # launches the Dioxus app
│   ├── lib.rs           # app() and app_with_store()
│   ├── router.rs        # Route enum
│   ├── application/     # ports: the traits the shell is written against
│   ├── storage/         # SQLite adapter implementing those ports
│   ├── transport/       # HttpMobileApi, the server adapter
│   ├── sync/            # the phase machine
│   ├── auth/            # token handling
│   ├── platform/        # Android and desktop specifics
│   └── components/      # Dioxus pages
```

`application` declares traits and `storage` and `transport` implement them, so a
test can drive the whole shell against an in-memory database and a recording
server without touching either.

```rust,ignore
pub fn app() -> Element;
pub fn app_with_store(store: Arc<dyn LocalStore>) -> Element;
```

The grammar pages draw entirely from the device's database and need it in
context, which is what `app_with_store` provides; `app` remains for the launch
paths that have not yet built one.

## Cargo features

| Feature | Brings in |
|---------|-----------|
| `server`  | `dioxus/server` |
| `desktop` | `dioxus/desktop` |
| `mobile`  | `dioxus/mobile` |
| `web`     | `dioxus/web` |

Typical commands:

```sh
# Desktop binary
cd wisecrow-mobile
cargo run --features desktop

# Mobile (requires the dioxus mobile tooling)
dx serve --platform android --features mobile
```

## Routes

```rust,ignore
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/learn/:native/:foreign")]
        LearnPage { native: String, foreign: String },
        #[route("/nback/:native/:foreign")]
        NbackPage { native: String, foreign: String },
        #[route("/grammar/brainmap/:native/:foreign")]
        GrammarBrainmapPage { native: String, foreign: String },
        #[route("/grammar/:native/:foreign")]
        GrammarSessionPage { native: String, foreign: String },
}
```

The mobile router does **not** expose the quiz route — quizzes require a PDF
picker that has not been wired up.

## Application ports

```rust,ignore
pub trait LocalStore:
    ProfileRepository + CorpusRepository + LearningRepository
    + ContentRepository + GrammarRepository + Send + Sync {}
```

| Trait | Holds |
|-------|-------|
| `ProfileRepository` | Profiles, the active one, and per-profile identity. |
| `CorpusRepository` | Vocabulary, card cursors and the sync phase machine. |
| `LearningRepository` | Local sessions, answers and the review outbox. |
| `ContentRepository` | N-back uploads, cached quizzes and the media cache. |
| `GrammarRepository` | The grammar mirrors, the attempt outbox and their cursors. |

```rust,ignore
#[async_trait]
pub trait GrammarRepository: Send + Sync {
    async fn apply_bank_page(&self, page: &GrammarBankChangePageDto) -> Result<(), MobileError>;
    async fn apply_mastery_page(&self, page: &GrammarMasteryChangePageDto) -> Result<(), MobileError>;
    async fn grammar_cursors(&self, language: &str) -> Result<GrammarCursors, MobileError>;
    async fn grammar_items(&self, language: &str) -> Result<Vec<LocalGrammarItem>, MobileError>;
    async fn grammar_rules(&self, language: &str) -> Result<Vec<LocalGrammarRule>, MobileError>;
    async fn grammar_mastery(&self, language: &str) -> Result<Vec<LocalGrammarMastery>, MobileError>;
    async fn queue_attempt(&self, attempt: &QueuedAttempt) -> Result<(), MobileError>;
    async fn pending_attempts(&self, limit: u16) -> Result<Vec<QueuedAttempt>, MobileError>;
    async fn apply_attempt_response(&self, ..) -> Result<(), MobileError>;
}
```

`queue_attempt` both writes the outbox row and moves the local mastery
projection, so the learner sees their answer counted at once. That projection is
provisional: it grades with the same `wisecrow-learning` reduction the server
uses, and the server's own figures replace it when the feed next arrives. The
domain types are in `application::grammar`:

```rust,ignore
pub struct LocalGrammarRule;
pub struct LocalGrammarItem { pub fn gradable(&self) -> GradableItem; }
pub struct LocalGrammarMastery;
pub struct QueuedAttempt {
    pub fn submission(&self) -> Submission;
    pub fn as_upload(&self) -> OfflineAttemptDto;
}
pub struct GrammarCursors { pub bank: i64, pub mastery: i64 }
```

## Server adapter

`MobileApi` is the port and `transport::HttpMobileApi` the adapter. Protocol 2
is asked for separately:

```rust,ignore
async fn capabilities_v2(&self) -> Result<Option<MobileCapabilitiesDto>, MobileError>;
async fn grammar_bank_changes(&self, ..) -> Result<GrammarBankChangePageDto, MobileError>;
async fn grammar_mastery_changes(&self, ..) -> Result<GrammarMasteryChangePageDto, MobileError>;
async fn upload_grammar_attempts(&self, ..) -> Result<GrammarAttemptBatchResponseDto, MobileError>;
```

`capabilities_v2` returns `Ok(None)` against a server that does not offer
protocol 2, which is not an error: such a server is simply one the device syncs
vocabulary with and no more.

## Sync phases

A sync is a state machine persisted in `sync_state`, so a run killed by the
scheduler resumes at the phase it reached rather than from the beginning.

```rust,ignore
pub enum SyncPhase {
    Idle, Reviews, Nback, GrammarAttempts, Cards,
    Snapshots, Deltas, GrammarBank, GrammarMastery, Finishing,
}
```

| Phase | Direction | Work |
|-------|-----------|------|
| `Reviews` | up | The review outbox. |
| `Nback` | up | Completed n-back sessions. |
| `GrammarAttempts` | up | The grammar attempt outbox, oldest first. |
| `Cards` | down | Card changes. |
| `Snapshots` | down | First-run vocabulary for a pair. |
| `Deltas` | down | Corpus changes since the cursor. |
| `GrammarBank` | down | Item bank changes, per ready language. |
| `GrammarMastery` | down | The learner's mastery figures. |

The mastery feed covers the learner rather than a language, so one cursor serves
it and any language the device follows reports the same position.

Answers go up before anything comes down, so the mastery the device then pulls
already accounts for them; a device that pulled first would paint figures it was
about to invalidate. The three grammar phases do no work unless the server
advertises protocol 2 and all of `GrammarBankSync`, `GrammarMasterySync` and
`GrammarAttemptUpload`; they still advance, so a device talking to a version-1
server walks the same machine and syncs vocabulary only. A partial offer is
refused rather than half-taken: it would leave answers recorded on the device
with nowhere to go. The bank is pulled only for pairs whose vocabulary
sync has reached `Ready`.

Cursors move only to the `next_cursor` the server returns. An empty page returns
the cursor unchanged, and the device must not advance past it — see
[the change feeds](../reference/database-schema.md#grammar-change-feeds) for what
a client that did would lose.
