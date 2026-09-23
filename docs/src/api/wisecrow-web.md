# wisecrow-web

`wisecrow-web` is an experimental [Dioxus](https://dioxuslabs.com) 0.7
fullstack application. It uses the `dioxus-fullstack` server-functions
feature to share routing and types between the browser bundle and the
server.

> **Note:** The TUI is the primary interface today. Treat the web UI as a
> preview — names, routes, and types may shift between releases.

## Crate structure

```text
wisecrow-web/
├── Cargo.toml
├── src/
│   ├── main.rs        # entry point
│   ├── router.rs      # Route enum
│   ├── components/    # client-side Dioxus components
│   │   ├── home.rs, layout.rs, learn/, nback/, quiz/
│   └── server/        # server-feature only modules
│       ├── learn.rs, nback.rs, quiz.rs, media.rs, sync.rs, mod.rs
└── assets/
```

The `server` feature gates everything that touches the database. Without it,
the crate compiles to WASM and the server-functions resolve at runtime
against the colocated server.

## Cargo features

| Feature | Default | Pulls in | Used for |
|---------|---------|----------|----------|
| `audio` | yes | `wisecrow-core/tts` + `base64` | Serve Edge TTS pronunciation over the wire. |
| `images` | yes | `wisecrow-core/images` + `base64` + `reqwest` | Serve Unsplash-fetched images. |
| `server` | no | `wisecrow-core`, `tokio`, `sqlx`, … | Build the server binary (`dx` enables this). |
| `web` | no | `dioxus/web` | Build the browser bundle (`dx` enables this). |

`audio` and `images` are on by default. Use `--no-default-features` for a
minimal build. Images also need `WISECROW__UNSPLASH_API_KEY` at runtime.

A typical fullstack dev loop:

```sh
cd wisecrow-web
dx serve
```

## Routes

```rust,ignore
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[route("/login")]
    LoginPage {},
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/learn/:native/:foreign")]
        LearnPage { native: String, foreign: String },
        #[route("/nback/:native/:foreign")]
        NbackPage { native: String, foreign: String },
        #[route("/fast/:native/:foreign")]
        FastPage { native: String, foreign: String },
        #[route("/quiz")]
        QuizPage {},
        #[route("/grammar/placement/:native/:foreign")]
        PlacementPage { native: String, foreign: String },
        #[route("/grammar/brainmap/:native/:foreign")]
        BrainmapPage { native: String, foreign: String },
        #[route("/grammar/:native/:foreign")]
        GrammarPage { native: String, foreign: String },
        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
}
```

| Route | Purpose |
|-------|---------|
| `/login` | Sign-in, outside the layout so the chrome does not offer links a signed-out reader cannot follow. |
| `/` | Landing / language picker. |
| `/learn/:native/:foreign` | The flashcard session UI. |
| `/nback/:native/:foreign` | The dual n-back trainer. |
| `/fast/:native/:foreign` | The auto-advancing deck. |
| `/quiz` | Stand-alone quiz (PDF upload or rule-based generation). |
| `/grammar/:native/:foreign` | A grammar practice session, twelve items drawn by mastery. |
| `/grammar/placement/:native/:foreign` | The placement test, one CEFR level at a time. |
| `/grammar/brainmap/:native/:foreign` | The brainmap: every syllabus point, banded by what is known. |
| `/:..segments` | Not found. Declared last, inside the layout, so a wrong URL keeps the site's chrome. |

The four-segment grammar routes are declared before the three-segment one only
for readability. They cannot collide: the router matches on segment count first.

## Server functions

The server module groups the database-touching server functions. All take
`wisecrow-dto` types so the WASM bundle can call them without depending on
`wisecrow-core`. See `wisecrow-mobile` for the matching client-side stub
declarations.

| Module | Reaches |
|--------|---------|
| `server::learn` | `SessionManager`, `CardManager` |
| `server::nback` | `DnbEngine`, `DnbSessionRepository`, `apply_srs_feedback` |
| `server::quiz`  | `grammar::ai_exercises`, `grammar::pdf` |
| `server::media` | `MediaCache`, optional Edge TTS / Unsplash |
| `server::sync`  | endpoints consumed by `wisecrow-core` `SyncClient` |

### Endpoints

Dioxus server functions take a single argument, so a request body is wrapped:
`{"request": { .. }}`, not the payload flat.

| Endpoint | Purpose |
|----------|---------|
| `/api/auth/login`, `/api/auth/logout` | Browser session cookies. |
| `/api/learn/*` | Language list, session create/resume/pause/complete, card answer, fast deck. |
| `/api/nback/{start,trial,complete}` | The dual n-back trainer. |
| `/api/quiz/{pdf,rule}` | Ad-hoc quiz generation. |
| `/api/media/{audio,image}` | Cached pronunciation and imagery. |
| `/api/grammar/session/{start,submit,complete}` | A grammar practice session. |
| `/api/grammar/placement/{start,submit}` | A placement run, level by level. |
| `/api/grammar/brainmap` | Every syllabus point with its band. |
| `/api/mobile/{login,me,logout}` | Device tokens. |
| `/api/mobile/capabilities`, `/api/mobile/v2/capabilities` | Protocol negotiation. |
| `/api/mobile/devices/register` | Device enrolment. |
| `/api/mobile/corpus/{snapshot,changes}`, `/api/mobile/cards/changes` | Vocabulary feeds (protocol 1). |
| `/api/mobile/{reviews,nback}/upload` | Review and n-back events (protocol 1). |
| `/api/mobile/v2/grammar/bank/changes` | The item bank feed, answers included. |
| `/api/mobile/v2/grammar/mastery/changes` | The mastery feed for the calling learner. |
| `/api/mobile/v2/grammar/attempts/upload` | Offline answers, judged one at a time. |

`/api/grammar/*` serves items with the answers stripped: the server grades, so a
browser cannot read the answer out of the payload it was tested with. The
`/api/mobile/v2/*` bank feed is the exception and carries them, because a device
out of contact has to grade for itself — see
[the DTO reference](wisecrow-dto.md#grammar-sync-dtos-protocol-2).

The attempts upload adopts the session identifier the device chose, so a session
begun offline needs no round trip before its answers can be recorded. It
resolves the language and level before opening its transaction: a query issued on
the pool while a transaction is held takes a second connection, and under a small
pool that is how a request comes to wait on itself.

## Configuration

The server crate reuses `wisecrow-core`'s configuration loader, so the same
`WISECROW__*` variables apply. The pool is initialised once at start-up
in `main.rs`:

```rust,ignore
#[cfg(feature = "server")]
{
    tokio::runtime::Runtime::new()?
        .block_on(server::init_pool())?;
}
launch(app);
```
