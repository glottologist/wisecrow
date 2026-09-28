# Grammar source documents

Input for [`import-pdf`](../docs/src/reference/cli-reference.md#import-pdf) and
[`quiz`](../docs/src/reference/cli-reference.md#quiz). Each file was checked for
an extractable text layer and then run through `pdf::extract`, so every one here
yields quiz items rather than failing at import.

The shelf is one directory per language, named by the code the CLI already
accepts: `gd`, `ga`, `cy`, `fr`, `it` and `es`. The layout is not merely tidiness.
`import-pdf` reads the language from the directory, so a document needs no
`--lang` and a whole language can be imported in one command, and both it and
`check-pdf` accept a directory wherever they accept a file.

The `Synthesis` column is read by `import-pdf`, not merely by people. Importing
sends a document's passages to the configured model, and for a hosted model that
is a reproduction to a third party, so a document is read to the model only when
its row here says `yes`. A document with no row is refused as well, since nothing
is known about it; `--force` overrides the refusal for a model hosted locally,
where no third party receives the text.

The `Levels` column says which CEFR levels a document is worth reading at,
as a range (`A1–B1`), a list (`A1, C2`) or `all`, and `import-pdf` reads it:
a run without `--level` reads each document at every level its row names,
and a run with `--level` reads only the documents whose row names that
level. An empty cell keeps a document out of a language run without changing
whether it may be sent at all -- the verb tables, the vocabulary book and the
translation course are cleared but not levelled, and are read only when
`--level` says where. The values are a reading of each title, not a
measurement, and are the place to correct when a document turns out to
belong elsewhere.

| File | Work | Year | Licence | Synthesis | Levels |
|------|------|-----:|---------|-----------|--------|
| `gd/calder-1923.pdf` | George Calder, *A Gaelic Grammar* | 1923 | public domain | yes | B1–C2 |
| `cy/evans-1910.pdf` | D. Simon Evans, *The Elements of Welsh Grammar* | 1910 | public domain | yes | A2–B2 |
| `cy/morris-jones-1913.pdf` | John Morris-Jones, *A Welsh Grammar, Historical and Comparative* | 1913 | public domain | yes | C1–C2 |
| `ga/christian-brothers-1920.pdf` | *First Irish Grammar* | 1920 | public domain | yes | A1–B1 |
| `fr/liberte-2022.pdf` | Gretchen Angelo and Emmanuelle Remy, *Liberté* | 2022 | CC BY-SA 4.0 | yes | A1–A2 |
| `fr/bevier-1896.pdf` | Louis Bevier, *A French Grammar* | 1896 | public domain | yes | B1–C1 |
| `it/daccordo.pdf` | Italian faculty, University of Iowa, *D'Accordo!* | 2021 | CC BY-NC-SA 4.0 | yes | A1–A2 |
| `es/olmsted-1920.pdf` | Everett Olmsted, *First Course in Spanish* | 1920 | public domain | yes | A1–B1 |
| `es/coester-1912.pdf` | Alfred Coester, *A Spanish Grammar* | 1912 | public domain | yes | B1–C1 |
| `ga/caighdean-oifigiuil-2017.pdf` | Houses of the Oireachtas, *Gramadach na Gaeilge: An Caighdeán Oifigiúil* | 2017 | no licence stated | no | C1–C2 |
| `gd/goc-2009.pdf` | SQA, *Gaelic Orthographic Conventions* | 2009 | reproduction limited to SQA qualifications | no | C1–C2 |
| `es/yo-puedo-1-2021.pdf` | Elizabeth Silvaggio-Adams and Rocío Vallejo-Alegre, *Yo puedo: para empezar* | 2021 | CC BY-NC 4.0 | yes | A1–A2 |
| `it/spunti-elementare-1-2019.pdf` | Daniel Leisawitz and Daniela Viale, *Spunti: Italiano elementare 1* | 2019 | CC BY-NC-SA 4.0 | yes | A1–A2 |
| `fr/interactif-ed4-2019.pdf` | Karen Kelton, Nancy Guilloteau and Carl Blyth, *Français interactif*, 4th edn | 2019 | CC BY 4.0 | yes | A1–A2 |

The Creative Commons works carry conditions the public-domain ones do not.
*Liberté* is share-alike, so a derived syllabus exported from it inherits
CC BY-SA 4.0. *D'Accordo!* and *Spunti* are non-commercial as well as
share-alike, and *Yo puedo* is non-commercial. *Français interactif* is the
only one that asks for nothing beyond attribution. All of them require
attribution to the authors named above wherever their material is
redistributed.

The two standards documents are not open at all. *An Caighdeán Oifigiúil*
states no licence, and GOC allows reproduction only in support of SQA
qualifications, with SQA acknowledged and no trade or commercial use. Both are
safe to consult by hand, since a rule statement checked against them is not
their text, but neither may be redistributed with the application nor sent to a
hosted model, which is why their `Synthesis` cell says `no`.

The Celtic school grammars are all from before the modern spelling reforms —
Calder predates the 1981 Gaelic Orthographic Conventions — so their example
sentences need reading before they reach a learner. GOC 2009 and *An Caighdeán
Oifigiúil* 2017 give the current forms for Gaelic and Irish, so a rule taken
from Calder or the Christian Brothers can be checked against them. Welsh has no
counterpart here: no modern Welsh grammar with a usable licence was found, and
the two 1910s works stand alone.

## Personal copies

The shelf also holds the maintainer's personal copies of commercial titles.
They are not in the table above because they belong to a different category:
none of them may be redistributed, and none
of them is in the repository -- `.gitignore` keeps every PDF out of it, and only
this file is tracked. They are here because the application is single-user, so a
rule synthesised from a book one owns goes no further than the machine one owns
it on. Were Wisecrow ever to serve anyone else, these would have to come out, and
the openly licensed works above are what would remain.

The authors and titles below are taken from each file's own metadata rather than
from its filename, or from the title page where the metadata is blank, and the
pages from `pdfinfo`. The publication year is left out deliberately: the files
carry a PDF creation date, which for several of them is the date the copy was
made rather than the date the edition appeared, and a plausible-looking year is
worse than none.

Three rows say `no` under `Synthesis` although the documents read. *Easy Learning
Spanish Grammar*, *Complete Irish* and *Everyday Gaelic* are scans with an OCR
text layer that damages spelling -- `bre6` for *breá*, `-i6` for *-ió*, and
`bata` for *bàta*. The last was checked against the printed pronunciation guide
on PDF page 9 of *Everyday Gaelic*. These copies stay on the shelf for reading,
not for the model.

| File | Work | Pages | Read by | Synthesis | Levels |
|------|------|------:|---------|-----------|--------|
| `es/Barron's 501 Verbs - Spanish.pdf` | Christopher Kendris, *501 Spanish Verbs* | 739 | poppler | yes | |
| `es/Collins Easy Learning Spanish Grammar.pdf` | HarperCollins, *Collins Easy Learning Spanish Grammar* (OCR layer) | 163 | pdf-extract | no | A1–B1 |
| `es/Practice Makes Perfect Advanced Spanish Grammar [True PDF].pdf` | Rogelio Alonso Vallecillos, *Practice Makes Perfect: Advanced Spanish Grammar* | 210 | poppler | yes | C1–C2 |
| `es/Practice Makes Perfect Basic Spanish.pdf` | Dorothy Devney Richmond, *Practice Makes Perfect: Basic Spanish* | 273 | pdf-extract | yes | A1–A2 |
| `es/Practice Makes Perfect Spanish Irregular Verbs.pdf` | Eric Vogt, *Practice Makes Perfect: Spanish Irregular Verbs Up Close* | 126 | pdf-extract | yes | B1–B2 |
| `es/Practice Makes Perfect Spanish Sentence Builder.pdf` | Gilda Nissenberg, *Practice Makes Perfect: Spanish Sentence Builder* | 224 | poppler | yes | B1–B2 |
| `es/Practice Makes Perfect Spanish Verb Tenses.pdf` | Dorothy Richmond, *Practice Makes Perfect: Spanish Verb Tenses*, 2nd edn | 353 | pdf-extract | yes | A2–B2 |
| `es/Practice Makes Perfect_ Complete Spanish All-in-One, Premium Second Edition.pdf` | Gilda Nissenberg, *Practice Makes Perfect: Complete Spanish All-in-One*, premium 2nd edn | 869 | poppler | yes | A1–B2 |
| `es/Spanish Grammar (Schaum's outlines).pdf` | Conrad J. Schmitt, *Schaum's Outline of Spanish Grammar* | 205 | pdf-extract | yes | B1–C1 |
| `es/[Bookflare.net] - Practice Makes Perfect Complete Spanish All-in-One, 2nd Edition.pdf` | Gilda Nissenberg, *Practice Makes Perfect: Complete Spanish All-in-One*, 2nd edn | 653 | pdf-extract | yes | |
| `fr/Les 500 exercices de grammaire + corrigés (B1) (French Edition)_nodrm.pdf` | Marie-Pierre Caquineau-Gündüz and others, *Les 500 exercices de grammaire, niveau B1* | 226 | poppler | yes | B1 |
| `fr/Modern French Grammar - A Practical Guide (2nd Ed).pdf` | Margaret Lang and Isabelle Perez, *Modern French Grammar: A Practical Guide*, 2nd edn | 387 | poppler | yes | B1–C1 |
| `fr/Practice Makes Perfect  French Sentence Builder.pdf` | Eliane Kurbegov, *Practice Makes Perfect: French Sentence Builder* | 236 | pdf-extract | yes | B1–B2 |
| `fr/Practice Makes Perfect French Pronouns and Prepositions, Second Edition (Practice Makes Perfect Series) (French Edition).pdf` | Annie Heminway, *Practice Makes Perfect: French Pronouns and Prepositions*, 2nd edn | 375 | poppler | yes | A2–B2 |
| `fr/Practice Makes Perfect French Vocabulary.pdf` | Eliane Kurbegov, *Practice Makes Perfect: French Vocabulary* | 222 | pdf-extract | yes | |
| `fr/Practice Makes Perfect_ Complete French All-in-One, Premium Second Edition (French Edition) (Jason Ridgway-Taylor's conflicted copy 2025-12-27).pdf` | Annie Heminway, *Practice Makes Perfect: Complete French All-in-One*, premium 2nd edn | 1090 | poppler | yes | A1–B2 |
| `fr/Practice Makes Perfect_ Complete French Grammar, Premium Third Edition.pdf` | Annie Heminway, *Practice Makes Perfect: Complete French Grammar*, premium 3rd edn | 490 | poppler | yes | A1–B2 |
| `fr/Practice Makes Perfect_ French Verb Tenses (Practice Makes Perfect Series).pdf` | Trudie Booth, *Practice Makes Perfect: French Verb Tenses* | 862 | poppler | yes | A2–B2 |
| `fr/Schaum's Outline of French Grammar.pdf` | Mary E. Coffman Crocker, *Schaum's Outline of French Grammar* | 398 | pdf-extract | yes | B1–C1 |
| `fr/The Vocabulary of Modern French Origins, Structure and Function.pdf` | Hilary Wise, *The Vocabulary of Modern French: Origins, Structure and Function* | 271 | pdf-extract | yes | |
| `ga/04.Teach  yourself complete Irish.pdf` | Diarmuid Ó Sé and Joseph Sheils, *Teach Yourself Complete Irish* (OCR layer) | 208 | pdf-extract | no | A1–A2 |
| `ga/06.Colloquial Irish.pdf` | Thomas Ihde, Máire Ní Neachtain, Roslyn Blyn-LaDrew and John Gillen, *Colloquial Irish* | 260 | pdf-extract | yes | A1–A2 |
| `ga/10.Irish nouns a reference guide.pdf` | Andrew Carnie, *Irish Nouns: A Reference Guide* | 363 | pdf-extract | yes | B2–C2 |
| `ga/14.Intermediate Irish A Grammar and Workbook.pdf` | Nancy Stenson, *Intermediate Irish: A Grammar and Workbook* | 257 | pdf-extract | yes | B1–B2 |
| `gd/907699838-Lamb-William-Scottish-Gaelic-a-Comprehensive-Grammar.pdf` | William Lamb, *Scottish Gaelic: A Comprehensive Grammar* | 581 | pdf-extract | yes | A1–C2 |
| `gd/Everyday_Gaelic_-_Morag_MacNeill.pdf` | Morag MacNeill, *Everyday Gaelic* (2006 edition, OCR layer) | 148 | pdf-extract | no | A1–A2 |
| `it/Modern Italian Grammar Workbook (Modern Grammar Workbooks) (Italian Edition).pdf` | Anna Proudfoot, *Modern Italian Grammar Workbook* | 221 | poppler | yes | B1–C1 |
| `it/Practice Makes Perfect Italian Pronouns And Prepositions, Second Edition (Practice Makes Perfect Series) (Jason Ridgway-Taylor's conflicted copy 2025-12-26).pdf` | Daniela Gobetti, *Practice Makes Perfect: Italian Pronouns and Prepositions*, 2nd edn | 341 | poppler | yes | A2–B2 |
| `it/Practice Makes Perfect Italian Sentence Builder (Practice Makes Perfect Series) (Jason Ridgway-Taylor's conflicted copy 2025-12-26).pdf` | Paola Nanni-Tate, *Practice Makes Perfect: Italian Sentence Builder* | 208 | poppler | yes | B1–B2 |
| `it/Practice Makes Perfect Italian Verb Tenses 2_E (EBOOK)_ With 300 Exercises + Free Flashcard App.pdf` | Paola Nanni-Tate, *Practice Makes Perfect: Italian Verb Tenses*, 2nd edn | 357 | poppler | yes | A2–B2 |
| `it/Practice Makes Perfect_ Complete Italian Grammar (Practice Makes Perfect Series).pdf` | Marcel Danesi, *Practice Makes Perfect: Complete Italian Grammar* | 437 | poppler | yes | A1–B2 |
| `it/Thinking Italian Translation_ A Course in Translation Method_ Italian to English (Thinking Translation).pdf` | Sándor Hervey, Ian Higgins, Stella Cragie and Patrizia Gambarotta, *Thinking Italian Translation* | 237 | poppler | yes | |

Lamb's copy contains the full grammar through the glossary on printed page 550.
`check-pdf` reads 652 passages at
1,822 characters per PDF page. Its chapters cover basic through advanced grammar,
so it is assigned A1–C2, providing a modern source at every level alongside
Calder's B1–C2 coverage.

Fifteen of these copies need poppler, as the `Read by` column records. `pdf-extract`
recovers between a sixteenth and a thirtieth of their text, because the
McGraw-Hill and Routledge ebooks embed fonts with custom encodings, and it panics
outright on *Les 500 exercices*. This is the reason the fallback described below
exists.

Three filenames deserve a note. *Complete French All-in-One* and the two Italian
*Practice Makes Perfect* titles arrived through Dropbox as conflicted copies, and
their names still say so; each is the only copy of its title on the shelf, and
nothing depends on the name, so they have been left as they are rather than
quietly renamed. Welsh gained nothing: the three candidates added to `cy/` were
page scans with no text layer and were removed, as were thirteen Gaelic scans,
one Irish and three Italian, so the two 1910s Welsh grammars still stand alone.

## Checking a new document

A PDF built from page scans holds no text, and neither extractor returns anything
from one. Two candidates were rejected for exactly that: the Google Books scans
of Young's *An Italian Grammar* and Fuentes's *A Practical Spanish Grammar*
carry text for the copyright notice and nothing else. A third, the *Libro Libre*
Spanish open textbook, is encrypted with a key `lopdf` will not open and fails
with `unsupported key length`.

`wisecrow check-pdf --file <path>` separates the three cases before an import is
attempted, taking a single document, a language directory such as `grammar/gd`, or
the whole of `grammar`. It reads the file with the extractor the import itself uses, so its
verdict is the one that matters: `OK` for prose at the density of a grammar book,
`THIN` for a document under two hundred characters a page, and `FAIL` for one
that cannot be opened or holds no passage long enough to state a rule. The
`pdf-text-check` tool in the agent-tools store remains useful for a candidate
that is still a URL, since it downloads before it measures.

Two extractors are tried: `pdf-extract` first, then poppler's `pdftotext` where
that reads under two hundred characters a page. The commercial French grammars
need the second — `pdf-extract` recovers barely a twentieth of their text, since
their embedded fonts carry custom encodings — so poppler is a declared dependency
in `devbox.json` rather than an optional extra. The report names the extractor
that answered, which is worth reading: a document that only poppler can open is
one to keep an eye on if the text ever looks mangled.

Several hosts refuse an automated download even when the work itself is open.
bepress repositories and the Milne and BCcampus catalogues answer `curl` with
403, and the Pressbooks export endpoints for *Yo puedo* and *Français
interactif* answer 500. The copies here came from the routes that do serve a
file: LibreTexts renders a whole book at
`batch.libretexts.org/print/Letter/Finished/human-<pageId>/Full.pdf`, where
`pageId` is in the book's cover page HTML, and *Français interactif* publishes
the full textbook at `laits.utexas.edu/fi/FrancaisInteractif-textbook.pdf`.
Only the first volume of *Yo puedo* has a rendered LibreTexts PDF; the second
returns 404.
