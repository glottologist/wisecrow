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

| Feature | Pulls in | Used for |
|---------|----------|----------|
| `server` | `wisecrow-core`, `tokio`, `sqlx`, `dotenvy`, `config`, `tempfile` | Build the server binary. |
| `web` | `dioxus/web` | Build the browser bundle. |
| `audio` | `server` + `wisecrow-core/audio` + `base64` | Serve TTS audio over the wire. |
| `images` | `server` + `wisecrow-core/images` + `base64` + `reqwest` | Serve Unsplash-fetched images. |

A typical fullstack dev loop:

```sh
cd wisecrow-web
dx serve --features "server web"
```

## Routes

```rust,ignore
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/learn/:user_id/:native/:foreign")]
        LearnPage { user_id: i32, native: String, foreign: String },
        #[route("/nback/:native/:foreign")]
        NbackPage { native: String, foreign: String },
        #[route("/quiz")]
        QuizPage {},
}
```

| Route | Purpose |
|-------|---------|
| `/` | Landing / language picker / user selector. |
| `/learn/:user_id/:native/:foreign` | The flashcard session UI. |
| `/nback/:native/:foreign` | The dual n-back trainer. |
| `/quiz` | Stand-alone quiz (PDF upload or rule-based generation). |

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
