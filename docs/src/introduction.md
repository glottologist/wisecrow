# Introduction

Wisecrow is an intensive language-cramming toolkit built around the idea that
*frequency drives fluency*. It mines large multilingual subtitle and translation
corpora from [OPUS](https://opus.nlpl.eu) — OpenSubtitles, CCMatrix, and NLLB —
and turns them into a frequency-ordered flashcard deck backed by an
[FSRS](https://github.com/open-spaced-repetition/rs-fsrs) spaced-repetition
scheduler.

It is designed for learners who want to **front-load high-frequency vocabulary**
in a target language, see it in subtitle context, and review it in a focused
loop without leaving the terminal.

## What Wisecrow does

- **Ingests** TMX and OPUS XML alignment archives directly from OPUS, with
  retry, decompression, and size limits.
- **Persists** translation pairs into PostgreSQL with frequency counts and a
  unique `(from_phrase, to_phrase)` constraint that makes re-ingestion safe.
- **Schedules** review using FSRS, with sessions that can be paused and resumed.
- **Drills** with two TUI modes: a Leitner-style flashcard runner, and a
  research-grade dual n-back that uses your own vocabulary as stimuli.
- **Generates** CEFR-graded grammar rules and quizzes via Anthropic or OpenAI
  LLM providers.
- **Syncs** translations and grammar rules between Wisecrow instances over
  HTTP.
- **Augments** cards with on-demand audio (Microsoft Edge TTS) and imagery
  (Unsplash) — both feature-gated and optional.

## What it is not

- Wisecrow is **not a translator**. It assumes the target corpora already
  contain translations.
- It is **not a course**. It will not teach you grammar progressively;
  it ranks by frequency and lets you grind.
- The web and mobile front-ends in this workspace are **early-stage**.
  The TUI is the production interface today.

## Workspace at a glance

| Crate | Purpose |
|-------|---------|
| `wisecrow-core` | Library plus the `wisecrow` CLI binary. All ingestion, scheduling, drilling, and persistence lives here. |
| `wisecrow-dto`  | Plain-old serializable types shared between the server and front-end clients. |
| `wisecrow-web`  | [Dioxus](https://dioxuslabs.com) fullstack web UI. Server features gate the database-backed routes. |
| `wisecrow-mobile` | Dioxus mobile/desktop shell that calls into the web server functions. |

> **Note:** Wisecrow expects you to bring your own PostgreSQL database. The
> first `ingest` (or any database-touching command) automatically applies the
> bundled migrations from `wisecrow-core/migrations/`.

## Where to start

- New here? Jump to [Installation](./getting-started/installation.md), then
  [Configuration](./getting-started/configuration.md).
- Trying to drill on existing data? See the [Learn TUI guide](./guides/learn-tui.md).
- Building on the library? Open the [Architecture overview](./reference/architecture.md)
  followed by the [API references](./api/wisecrow-core.md).
