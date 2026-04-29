# Subtitle preview

`wisecrow preview --file <path>` parses an `.srt`, `.vtt`, `.ass`, or `.ssa`
subtitle file, tokenises the text, and produces an annotated frequency table
showing which words you already know, which you're currently learning, and
which are entirely new.

Use this when:

- You're about to watch tonight's episode of a show in your target language
  and want to triage vocabulary first.
- You want to know which lemmata in a script are corpus-rare so you can
  pre-load them as cards before they ambush you.

## CLI

```sh
wisecrow preview --file episode.srt -n en -f es
```

Alias: `wisecrow pv ...`.

| Flag | Required | Default | Effect |
|------|----------|---------|--------|
| `--file` | yes | — | Path to the subtitle file. Format inferred from extension: `.vtt`, `.ass`, `.ssa`, or default to SRT. |
| `--native-lang` / `-n` | yes | — | Native language code (used by `--gloss-unknowns`). |
| `--foreign-lang` / `-f` | yes | — | Foreign language code (selects tokenizer + drives DB lookups). |
| `--unknown-only` | no | `false` | Print only tokens not in your SRS deck (`[new]` and `[?]`). |
| `--no-srs` | no | `false` | Skip the SRS state lookup. Only deduplicates tokens. |
| `--top-n` | no | `None` | Limit output to the N highest-frequency rows. |
| `--gloss-unknowns` | no | `false` | LLM-translate corpus-unknown tokens (`[?]`) into your native language. Requires LLM provider configured. |

## Tokenizers

The preview pipeline picks a tokenizer per `--foreign-lang`:

| Lang | Tokenizer | Notes |
|------|-----------|-------|
| `zh` | `jieba-rs` | Chinese word segmentation, always available. |
| `ja` | `lindera` 3.x with embedded IPADIC | Japanese morphological analysis, dict embedded at compile time. |
| `th` | `kham-core` | Pure-Rust Thai segmenter. |
| `km`, `lo`, `my` | None | Returns `UnsupportedLanguage` error — these languages need segmenters not yet integrated. |
| Everything else | Whitespace + punctuation strip | Correct for ~90 languages: Latin, Cyrillic, Arabic, Devanagari, Hangul (eojeol-level), etc. |

## Annotation legend

Each token is tagged with one of:

- `[known]` — present in `translations` AND has a `cards` row in FSRS state 2 (Review). You've internalised this word.
- `[learning]` — present in `translations` AND has a `cards` row in FSRS state 1 (Learning) or 3 (Relearning).
- `[new]` — present in `translations` AND has either no `cards` row, or a row in state 0 (New).
- `[?]` — not present in `translations` for this language pair. You haven't ingested this word, the corpus doesn't contain it, or it's an inflected form not in the corpus's surface-form index.

## Status semantics

Output is sorted by corpus frequency descending. With `--top-n 30` you get
the 30 highest-frequency tokens in the file. With `--unknown-only`, only
`[new]` and `[?]` tokens are included — that's "what does this script
demand of me that I haven't yet drilled."

## LLM gloss for corpus-unknowns

`--gloss-unknowns` collects all `[?]` tokens (those completely missing from
the `translations` table for this language pair), sends them to the LLM in
one batch with a "translate these foreign words to <native>" prompt, and
appends the returned gloss inline:

```
        [?]        - bizarro → weird
        [?]        - acoso → harassment
```

This catches words from the script that your corpus simply doesn't cover —
inflected forms not in the surface-form index, slang, names, or rare lemmata.

`--gloss-unknowns` requires `WISECROW__LLM_PROVIDER` and
`WISECROW__LLM_API_KEY` set; without them the CLI errors fast.

## Examples

```sh
# Triage tonight's Spanish episode, top 30 unknowns.
wisecrow preview --file ep1.srt -n en -f es --unknown-only --top-n 30

# Same, but also LLM-translate corpus-misses.
wisecrow preview --file ep1.srt -n en -f es --unknown-only --gloss-unknowns

# Japanese ass file with full annotation.
wisecrow preview --file anime.ass -n en -f ja
```

## Limitations

- v1 is corpus-only by default — no fuzzy matching of inflected forms, no
  lemmatisation. A Spanish word like `casas` might tag `[?]` even if `casa`
  is in your deck. Use `--gloss-unknowns` to bridge the gap.
- The `translations.to_phrase` join is exact match (case-folded by the
  tokenizer). Multi-word phrases in the corpus won't match single tokens.
- `[new]` vs `[learning]` distinguishes "I haven't seen it" from "I'm
  actively drilling it" — both are useful signals but they're FSRS-state
  driven, not based on actual review history.
