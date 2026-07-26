# Frequency ranking

Wisecrow is built on the premise that frequency drives fluency, so the order in
which cards reach you is not a cosmetic detail — it is the product. This guide
covers where that ordering comes from, why a freshly ingested corpus produces
almost no cards until it has been ranked, and what to do for languages whose
frequency lists nobody has published.

## Why ranking is a required step

Ingestion sets each new translation's `frequency` to 1 and increments it when
the same pair arrives again, so the column starts as a duplicate count rather
than a measure of commonality. Deck selection then asks for rows with
`frequency > 1`, which means a corpus ingested once and never ranked offers the
learner almost nothing: on a 25,000-pair Welsh sample, ten rows cleared that
filter. Running `frequency` is what turns a pile of aligned sentences into an
ordered curriculum.

```sh
wisecrow frequency --lang es
```

With no `--file`, the command fetches the Hermit Dave FrequencyWords list for
the language, which is derived from OpenSubtitles and covers 62 languages.

## What a list can and cannot reach

Two properties of the matching are worth understanding before you judge the
results, because both bound how much of a corpus a list can touch.

The first is that matching applies to **whole phrases**. A list holds single
words; a subtitle corpus holds sentences. A row is ranked only when its entire
phrase equals a listed word, so a word list reaches the single-word rows and
leaves the sentences alone. On the Welsh sample above, 749 of 25,000 rows were
single words. This is a real ceiling, not a defect to work around: sentence-level
ranking would need a scoring function over each row's tokens.

The second is that matching **folds case and edge punctuation**. Corpus phrases
arrive as `Beth` and `O.` where lists hold `beth` and `o`, and comparing them
raw would discard most of the overlap. Both sides are lower-cased and stripped
of leading and trailing `.,!?;:"'¡¿` before comparison. Migration
`016_normalised_phrase_indexes.sql` maintains expression indexes over exactly
that normalisation, so the update stays index-driven; if you change the
character set in `frequency.rs`, change the migration to match, and a unit test
will hold you to it.

## Either side of the pair

A list ranks a translation from whichever side of the pair its language sits
on. A Welsh list therefore reaches rows ingested as `-n en -f cy`, where Welsh
is the target, just as it reaches rows ingested as `-n cy -f en`. Ingest
direction is irrelevant, which matters because the ordinary direction for an
English speaker studying Welsh puts Welsh in `to_phrase`.

One consequence follows from a row carrying a single `frequency` column: if you
apply lists for both languages of a pair, the later application wins. Rank by
the language you are studying, and apply the other only if you want the deck
ordered by the prompt language instead.

## Languages Hermit Dave does not cover

Welsh, Irish and Scottish Gaelic are all absent from the FrequencyWords
collection, so `frequency --lang cy` fails with a 404 and `--file` becomes the
only route. The [Leipzig Corpora
Collection](https://wortschatz.uni-leipzig.de/en/download) fills the gap for
two of the three, and its word files are read in their native layout without
reshaping:

```sh
curl -O https://downloads.wortschatz-leipzig.de/corpora/cym_wikipedia_2021_100K.tar.gz
tar xzf cym_wikipedia_2021_100K.tar.gz
wisecrow frequency --lang cy \
  --file cym_wikipedia_2021_100K/cym_wikipedia_2021_100K-words.txt
```

Leipzig publishes Welsh (`cym`) and Irish (`gle`) Wikipedia corpora but no
Scottish Gaelic one, which the next two sections address.

`--file` accepts three layouts, choosing per line rather than per file, so a
list carrying comment rows or a stray blank causes no trouble:

| Layout | Shape | Typical source |
|--------|-------|----------------|
| `word count` | space-separated, two fields | Hermit Dave FrequencyWords |
| `rank<TAB>word<TAB>count` | tab-separated, three fields | Leipzig Corpora Collection |
| `word,count` | comma-separated, trailing comma tolerated | published CSV lists |

A line is accepted under the first layout whose count parses as a number, which
keeps a `word count` entry containing a comma from being mangled by the CSV
rule. Anything matching none of the three is skipped, and a file that yields no
entries at all is rejected rather than reported as a successful update of
nothing.

## Scottish Gaelic

Gaelic is absent from both of the usual collections, but two community lists
exist. [`innesmck/GaelicFrequencyLists`](https://github.com/innesmck/GaelicFrequencyLists)
is the better of them: 10,000 word forms with counts, MIT licensed, built from
LearnGaelic's *Litir do Luchd-ionnsachaidh* and *Watch Gaelic* BBC ALBA
transcripts. It counts surface forms rather than lemmas, which is what the
matcher wants, and its CSV loads without conversion:

```sh
curl -O https://raw.githubusercontent.com/innesmck/GaelicFrequencyLists/main/output/combined/frequency.csv
wisecrow frequency --lang gd --file frequency.csv
```

A second list from [iGàidhlig](http://www.igaidhlig.net/en/gaelic-word-frequencies/)
offers 12,800 forms drawn from a web corpus, but it ships as an ODS spreadsheet
with the columns reversed, states no licence, and skews towards institutional
vocabulary. Prefer the first.

## Deriving a list from the corpus

Where no list exists, or where a published one comes from the wrong genre, the
ingested corpus can supply its own counts:

```sh
wisecrow frequency --lang gd --from-corpus
```

This tokenises every phrase stored for the language, counts the word forms, and
applies the result through the same matching path a file would take. Two things
recommend it. The counts describe the material actually being studied rather
than a corpus of another register, and the vocabulary matches by construction,
since every word counted came from a phrase already in the table. Measured on
the OpenSubtitles Gaelic release, it ranked 725 rows where the best published
Gaelic list reached 369.

It changes where the counts come from, not what they can rank: matching remains
whole-phrase, so this ranks single-word rows and leaves the sentences
containing those words alone. A tokeniser is required, so Khmer, Lao and
Burmese are refused rather than mis-segmented.

## Checking the result

The command reports how many rows it updated, and the count of rows that clear
the deck filter is the figure that matters:

```sh
psql -d wisecrow -c \
  "SELECT count(*) FILTER (WHERE frequency > 1) AS ranked, count(*) AS total
     FROM translations;"
```

If the update reports zero, work through three causes in order: the language
has no row in `languages` at all, in which case the command logs a warning and
does nothing; the list parsed but nothing matched, which on a sentence corpus
with a word list is expected to be a small fraction; or the file was in neither
supported layout, which is now an error rather than a silent success.
