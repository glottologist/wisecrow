# Dual n-back

`wisecrow nback` runs an adaptive dual n-back session that uses your
ingested vocabulary as audio and visual stimuli. It is independent of the
SRS but feeds back into it.

## Quick start

```sh
wisecrow nback -n en -f es --mode audio_written --n-level 2
```

| Flag | Default | Notes |
|------|---------|-------|
| `--mode` | `audio_written` | One of `audio_written`, `word_translation`, `audio_image`. |
| `--n-level` | `2` | Initial n-level. Clamped to `[1, 9]`. |
| `--user-id` | `1` | FK into `users`. |

The session needs at least 8 vocabulary items for the language pair —
ingest first.

## Stimuli per mode

| Mode | Audio channel | Visual channel |
|------|---------------|----------------|
| `audio_written` | Foreign-language phrase, spoken | Native-language phrase, written |
| `word_translation` | Foreign-language phrase, spoken | Foreign phrase, written |
| `audio_image` | Foreign-language phrase, spoken | Image associated with the translation |

Audio uses the `audio` feature (Microsoft Edge TTS, no API key). The
`audio_image` mode also requires the `images` feature and an Unsplash API
key.

## Controls

This command does not use ratatui — it draws directly with crossterm so that
key-press latency is as low as possible. Bindings during a trial:

| Key | Action |
|-----|--------|
| `A` | Toggle audio match for the current trial. |
| `L` | Toggle visual match for the current trial. |
| `Enter` | Submit the trial early (otherwise the timer fires). |
| `Q` | Abort the session. |

The screen shows the trial number, the current n-level, and a one-line
result after each trial (`Result: audio=correct, visual=wrong`).

## Trial generation

For each trial:

1. The engine picks `audio_match` and `visual_match` independently with
   probability `MATCH_PROBABILITY = 0.30`.
2. If the chosen channel is meant to match, the engine returns the same
   vocab item that appeared n trials ago.
3. If it is meant to be a non-match, a random different item is chosen
   (the engine retries until it picks an item that is **not** the n-back
   target).

The first n trials per channel are forced to be non-matches because there is
no n-back history yet.

## Adaptation

`apply_adaptation` runs at the end of every 5-trial window. It reads the
rolling accuracy on each channel and decides:

| Audio acc. | Visual acc. | Action |
|-----------:|-----------:|--------|
| ≥ 0.80 | ≥ 0.80 | Increase `n_level` by 1 (cap 9), drop interval by 200 ms (floor 1500). |
| < 0.50 | any | Decrease `n_level` by 1 (floor 1), bump interval by 200 ms (cap 5000). |
| any | < 0.50 | Same as above. |
| else | else | Hold. |

The engine also tracks how many windows in a row n has been below the
session's starting level — see [Termination](#termination).

## Termination

A session ends when:

- 50 trials elapse (`MAX_TRIALS` in `bin/wisecrow.rs`), or
- `consecutive_below_start >= 3` — three windows in a row below the starting
  n-level signal you are not at your stretch zone today, or
- both channels' rolling accuracy drops below 40 % over the last 5 trials —
  there is no point continuing if you have lost the thread.

After termination:

1. The session row is updated with peak/end n-levels and final accuracy
   figures.
2. `dnb::feedback::apply_srs_feedback` sweeps the trial log:
   - For each translation that appeared as a stimulus, count
     `(correct, incorrect)` recognitions across both channels.
   - If net-correct, apply a `Good` review with weight 0.5.
   - If net-incorrect, apply an `Again` review with weight 0.5.
   - Equal counts: skip the card.

The fractional weighting prevents one tough n-back from torpedoing your
SRS schedule.

## Tuning

The hard-coded constants live in `wisecrow-core/src/dnb/scoring.rs`:

| Constant | Value | Purpose |
|----------|------:|---------|
| `ACCURACY_INCREASE_THRESHOLD` | 0.80 | Both channels must hit this to increase n. |
| `ACCURACY_DECREASE_THRESHOLD` | 0.50 | Either channel below triggers a decrease. |
| `ACCURACY_TERMINATE_THRESHOLD` | 0.40 | Both channels below ends the session early. |
| `ADAPTATION_WINDOW` | 5 | Trials per evaluation window. |
| `TIMING_STEP_MS` | 200 | Interval delta on increase/decrease. |
| `MIN_INTERVAL_MS` | 1500 | Floor. |
| `MAX_INTERVAL_MS` | 5000 | Ceiling. |
| `MIN_N_LEVEL` | 1 | Floor. |
| `MAX_N_LEVEL` | 9 | Ceiling. |
| `CONSECUTIVE_BELOW_START_LIMIT` | 3 | Termination trigger. |

These are deliberately conservative compared to the original n-back
literature; tighten them with care.
