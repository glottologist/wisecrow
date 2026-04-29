# Learn TUI

`wisecrow learn` opens a `ratatui`-based flashcard runner. This guide
documents the key bindings, the auto-advance pacing model, and the
session-resume contract.

## Start a session

```sh
wisecrow learn -n en -f es --deck-size 50 --speed-ms 2500 --user-id 1
```

| Flag | Default | Effect |
|------|--------:|--------|
| `--deck-size` | `50` | Number of cards in the session. |
| `--speed-ms` | `3000` | Auto-advance interval. Clamped to `[500, 10000]` by `SpeedController`. |
| `--user-id` | `1` | Per-user session scope. |

If a paused session exists for `(user_id, native, foreign)` it is resumed
silently — the CLI logs which session is being resumed and where in the deck
it picked up.

## Deck composition

`SessionManager::create` first asks `CardManager::due_cards` for cards that
are due to review. If that does not fill the deck size, the remainder is
filled from `VocabularyQuery::unlearned`, ordered by frequency descending.
`CardManager::ensure_cards` materialises a `cards` row for each new
translation so the FSRS scheduler has somewhere to write state.

The deck order is **Relearning > Learning > New > Review**, breaking ties by
`due ASC`. New cards drift to the back so you always see the riskiest stuff
first.

## Key bindings

The TUI is single-pane. All bindings are in `wisecrow-core/src/tui/app.rs`.

| Key | Action |
|-----|--------|
| `Space` | Reveal answer if hidden, otherwise advance. |
| `1` | Rate `Again` (the card just lapsed). |
| `2` | Rate `Hard`. |
| `3` | Rate `Good`. |
| `4` | Rate `Easy`. |
| `g` | Open the Leipzig gloss overlay for the current card. Press `g` again or `Esc` to close. Auto-advance pauses while the overlay is open. Requires an LLM provider configured via `WISECROW__LLM_PROVIDER` and `WISECROW__LLM_API_KEY`. |
| `Esc` | Close the gloss overlay (when open). |
| `+` / `=` | Slow down the auto-advance by 500 ms. |
| `-` | Speed up the auto-advance by 500 ms. |
| `p` | Pause / unpause the auto-advance timer. |
| `q` | Pause the session and exit. |
| `Ctrl+C` | Same as `q` but with a hard stop. |

## Auto-advance pacing

A `SpeedController` lives in `wisecrow-dto`. It ticks down `remaining_ms` on
each TUI frame (~10 Hz, 100 ms tick rate). When the timer expires:

1. The current card is rated `Good` if untouched, otherwise the explicit
   rating is used.
2. The card is persisted via `SessionManager::answer_card` which delegates
   to `CardManager::review`.
3. The next card is fetched and `SpeedController::reset` is called.

The bottom of the screen shows a thin progress bar with
`SpeedController::remaining_fraction` so you can see how long until the
auto-advance fires.

## Pause and resume

The TUI never destroys session state. `q` calls
`SessionManager::pause(pool, session_id)` which sets `paused_at = NOW()`. The
next invocation:

```sh
wisecrow learn -n en -f es --user-id 1
```

…runs `SessionManager::resume`. Resume returns the most recent paused
session for the user/lang triple and sets `paused_at = NULL`. Cards already
answered (`session_cards.answered = TRUE`) are skipped; the resumed
`current_index` is the count of answered cards.

You will rarely lose more than a card or two: each rating commits before
the next card draws.

## Optional media

`wisecrow learn` accepts no media flags — they are configured at build time
and runtime:

- Build the binary with `--features "audio images"`.
- Set `WISECROW__UNSPLASH_API_KEY` for image fetch.
- Run `wisecrow prefetch-media` ahead of time to warm the cache.

When media is unavailable the TUI logs a warning and falls back to text-only
cards. There is no error path that aborts the session because of a media
miss.

## What the screen shows

```text
┌────────────────────── Wisecrow ──────────────────────┐
│                                                       │
│                  bonjour                              │
│                                                       │
│                   hello                               │
│                                                       │
├───────────────────────────────────────────────────────┤
│ Card 7/50  •  freq 4321  •  reps 3  •  state Learning │
│ ████████████░░░░  speed 2500 ms                       │
└───────────────────────────────────────────────────────┘
```

The footer reflects the current card's state and frequency, so you can keep
an eye on which slice of the deck you are working on.
