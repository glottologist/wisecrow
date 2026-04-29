# Graded reader

`wisecrow graded-reader` generates a personalised CEFR-graded passage in the
foreign language, seeded from the vocabulary you've actually learned. The
LLM is instructed to reuse most of your seed words and add only a small
amount of new vocabulary at the level you specify.

Use this when:

- You're between A2 and B2 and want reading material that isn't humiliating
  or insultingly easy.
- You want to consolidate vocabulary you've already drilled, in connected
  prose rather than isolated cards.
- You want a glossary of the new words side-by-side, ready to feed back into
  your SRS deck.

## CLI

```sh
wisecrow graded-reader -n en -f es --cefr B1
```

Alias: `wisecrow gr ...`.

| Flag | Required | Default | Effect |
|------|----------|---------|--------|
| `--native-lang` / `-n` | yes | — | Native language code (used in glossary). |
| `--foreign-lang` / `-f` | yes | — | Foreign language code (passage language). |
| `--cefr` | yes | — | CEFR level: `A1`, `A2`, `B1`, `B2`, `C1`, or `C2`. |
| `--seed-states` | no | `2` | FSRS states that count as "learned". Comma-separated `i16` list. `2` = Review only. Pass `2,3` to also include Relearning (lapsed) cards. |
| `--seed-min-stability` | no | `None` | Optional FSRS-stability threshold. Cards below this stability are excluded from the seed list. |
| `--seed-limit` | no | `30` | Max number of seed words pulled from the database. |
| `--length-words` | no | `200` | Approximate target length of the passage. |
| `--format` | no | `md` | Output format: `md` (Markdown) or `html` (self-contained HTML page). |
| `--output` | no | stdout | Write to a file instead of stdout. |

## How "learned" is defined

The seed-vocab query (`VocabularyQuery::learned`) joins `cards` against
`translations` filtered by:

- `cards.state = ANY($seed_states)` — defaults to `[2]` (FSRS Review state).
- Optional `cards.stability >= $seed_min_stability` — extra filter for "I've
  actually internalised this," not just "FSRS marked it Review."
- Ordered by `translations.frequency DESC` — corpus-frequency ranking so the
  most useful learned words seed the passage.

This means:

- A fresh `wisecrow learn` user will get an empty seed list and the command
  will error with `No learned vocabulary found for the given filters`.
- The seed list is **language-pair-agnostic per user — currently** (see
  Limitations).

## Output formats

**Markdown** (`--format md`):

```markdown
# Graded Reader

<passage in foreign language>

## Glossary

- **<foreign>** — <native>
- ...
```

**HTML** (`--format html`):

A self-contained `<!doctype html>` page with `<h1>`/`<p>`/`<h2>`/`<ul>`. User
content is HTML-escaped (`&`, `<`, `>`, `"`). No external CSS or JS — open
the file in any browser. Pipe to a printer-CSS preprocessor for print
output, or render via `pandoc` for PDF.

## Tips

- Use `--seed-min-stability` once you've been learning for a while to avoid
  the LLM being "fed" cards you technically passed once but haven't actually
  retained.
- For a denser passage, raise `--seed-limit` (more vocabulary diversity) and
  `--length-words` together.
- For a more challenging text, set the CEFR level one notch above your
  comfort zone and pass a larger `--seed-limit` so the LLM has more to work
  with.

## Limitations

- v1 operates on a single-user assumption: the `cards` table has no
  `user_id`. "Learned" means "any card row in this state, regardless of who
  reviewed it." Multi-user-aware semantics is a follow-up.
- Quality depends on the LLM. The first paragraph is usually best; later
  paragraphs sometimes drift in style or introduce more new vocabulary than
  asked.
- Length is approximate — the LLM may overshoot or undershoot
  `--length-words` by ~30%.
