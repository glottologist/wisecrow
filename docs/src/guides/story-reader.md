# Story reader

Graded readers are translated from the PDFs on the reader shelf. Nothing is
generated from a public-domain retelling, and a scanned page is out of scope.

## Ledger

`readers/SOURCES.md` is the authority. A row names the series, level, title,
file, CEFR, pages, and `Translate`. The job reads a book only when
`Translate` is `yes`. There is no --force, and a local model may not skip
the ledger. A table with a `File` column and no `Translate` column is an
error.

Penguin levels map as 0–1 to A1, 2 to A2, 3–4 to B1, and 5–6 to B2. A row's
own CEFR cell overrides that map.

## The job

`prepare-readers` does one piece of work and stops. The order is:

1. Store one cleared book whose text layer is dense enough.
2. Translate one run of sentences.
3. Gloss one batch of words.
4. Voice one batch of sentences, then one batch of word notes.

Repeat the command until `reader-status` prints `idle`. A run the model
refuses twice is reported by `reader-status` and is not retried. If the
model cannot be reached, `prepare-readers` exits 75 and leaves the cursor
where it was. `reader-status` exits 10 while work remains.

`check-reader-pdf --file PATH` prints `ok` or `thin` before a book is added
to the ledger. It does not translate anything.

## Decks and the Read stage

A tapped word can be added to a named deck. A translation this job creates
for that word sets `owns_translation`, so the word does not enter the
evidence view. A translation that already existed stays in the view.

On the home page, stage 4 is Read. It opens `/read/{native}/{foreign}`. A
book is linked only when every sentence is translated and no run has failed.
The page can show the English under each sentence, play a sentence or a
word, and resume the saved place.
