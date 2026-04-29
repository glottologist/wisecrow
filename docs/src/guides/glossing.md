# Leipzig glossing

`wisecrow gloss` produces a Leipzig interlinear gloss of a sentence in any
supported language using a configured LLM provider. The result is cached in
the `glosses` table keyed by SHA-256 of the sentence and the language code, so
re-glossing the same sentence is instant.

Use this when:

- You're staring at a card and don't understand *why* a word takes the form
  it does (case, gender, tense, mood).
- You want to break a longer source sentence into morphemes before deciding
  whether to flashcard one of its constituent words.

## CLI

```sh
wisecrow gloss --sentence "Меня зовут Иван" --lang ru
```

Aliases: `wisecrow gl ...`.

| Flag | Required | Default | Effect |
|------|----------|---------|--------|
| `--sentence` / `-s` | yes | — | The sentence to gloss. Quote it if it contains spaces. |
| `--lang` / `-l` | yes | — | ISO-ish language code (`ru`, `es`, `ja`, etc.). Must match `wisecrow list-languages`. |
| `--refresh` | no | `false` | Bypass and overwrite the cached gloss for this `(sentence, lang)` pair. Forces a fresh LLM call. |

Output is plain text — four lines, in Leipzig conventions:

```
Меня     зовут           Иван
я-ACC    звать-3PL       Иван
1SG-ACC  call-3PL.PRES   Ivan(NOM)
"My name is Ivan."
```

## TUI overlay

While `wisecrow learn` is running, press `g` on any card to open a modal that
glosses the foreign-language side of the current card (`to_phrase`, in the
`foreign_lang` of the session). Auto-advance pauses while the modal is open.
Press `g` again or `Esc` to dismiss.

The TUI overlay shares the same `glosses` cache as the CLI — gloss a sentence
once and it's instant in both surfaces, on every device that points at the
same database.

## Configuration

Requires:

- `WISECROW__LLM_PROVIDER` set to `anthropic` or `openai`
- `WISECROW__LLM_API_KEY` set to the corresponding API key

If neither is set the CLI errors fast; the TUI logs an info message and the
`g` keypress simply does nothing.

## Cache

The `glosses` table (migration `010_glosses.sql`) holds:

| Column | Type | Notes |
|--------|------|-------|
| `id` | `SERIAL` | primary key |
| `sentence_hash` | `CHAR(64)` | SHA-256 hex of the sentence |
| `lang_code` | `VARCHAR(16)` | foreign-language code |
| `gloss_text` | `TEXT` | the gloss returned by the LLM |
| `created_at` | `TIMESTAMP WITH TIME ZONE` | row creation timestamp |

`(sentence_hash, lang_code)` is a unique constraint, so the cache is shared
across all CLI invocations and TUI sessions for the same sentence/language
pair. Hash-keying lets the same row serve both the freeform CLI path
(arbitrary input string) and the TUI path (a card's `to_phrase`) without
needing a foreign key to `translations`.

`--refresh` forces an `INSERT … ON CONFLICT DO UPDATE`, replacing any cached
row with the freshly-prompted gloss and bumping `created_at` to the current
time.

## Limitations

- Cache never expires automatically; rely on `--refresh` for invalidation.
- Quality depends entirely on the LLM. Inspect the gloss before trusting it
  for unusual constructions or low-resource languages.
