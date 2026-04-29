# wisecrow-mobile

`wisecrow-mobile` is the Dioxus mobile/desktop shell. It mirrors the web
app's routes and types but ships only client-side server-function stubs —
the real implementations live in `wisecrow-web`.

> **Warning:** The mobile crate is a skeleton. It compiles and renders the
> route tree but the server-function stubs return errors by design.
> Treat this page as a starting point, not a deployment guide.

## Crate structure

```text
wisecrow-mobile/
├── Cargo.toml
├── src/
│   ├── main.rs           # launches the Dioxus app
│   ├── router.rs         # Route enum (subset of wisecrow-web)
│   ├── server_fns.rs     # client-side stubs returning ServerFnError
│   └── components/       # mobile-friendly Dioxus components
```

## Cargo features

| Feature | Brings in |
|---------|-----------|
| `server`  | `dioxus/server` (no DB stack) |
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
}
```

The mobile router does **not** yet expose the quiz route — quizzes require a
PDF picker that has not been wired up.

## Server function surface

`server_fns.rs` declares the contract the mobile shell expects to talk to:

```rust,ignore
#[server] async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError>;
#[server] async fn create_session(user_id, native, foreign, deck_size, speed_ms) -> Result<SessionDto, _>;
#[server] async fn resume_session(user_id, native, foreign) -> Result<Option<SessionDto>, _>;
#[server] async fn answer_card(session_id, card_id, rating) -> Result<CardDto, _>;
#[server] async fn pause_session(session_id) -> Result<(), _>;
#[server] async fn complete_session(session_id) -> Result<(), _>;
#[server] async fn list_users() -> Result<Vec<UserDto>, _>;
#[server] async fn create_user(display_name) -> Result<UserDto, _>;
#[server] async fn start_nback_session(config) -> Result<(i32, Vec<DnbTrialDto>), _>;
#[server] async fn submit_nback_trial(session_id, trial_result, trial_dto) -> Result<DnbAdaptationDto, _>;
#[server] async fn complete_nback_session(...) -> Result<DnbSessionResultsDto, _>;
#[server] async fn generate_quiz(pdf_bytes, num_questions) -> Result<Vec<QuizItemDto>, _>;
#[server] async fn generate_rule_quiz(lang, level, num_questions) -> Result<Vec<QuizItemDto>, _>;
```

Every stub currently returns
`ServerFnError::new("client-side stub")`. To put it into use, wire each
function into a real Dioxus server-function on the server side (typically in
`wisecrow-web`) and point the mobile build at the resulting endpoint.

## Wiring it up

A common pattern when graduating from skeleton to production:

1. Stand up the `wisecrow-web` crate with `--features "server web"`.
2. Set the mobile crate's API base URL to that server.
3. Replace the stubs in `server_fns.rs` with thin HTTP calls (or use Dioxus'
   shared `#[server]` mechanism if both crates compile together).
4. Add a settings screen that captures the API URL and the user ID.
