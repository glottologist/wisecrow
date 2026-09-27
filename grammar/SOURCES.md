# Grammar source documents

Input for [`import-pdf`](../docs/src/reference/cli-reference.md#import-pdf) and
[`quiz`](../docs/src/reference/cli-reference.md#quiz). Each file was checked for
an extractable text layer and then run through `pdf::extract`, so every one here
yields quiz items rather than failing at import.

| File | Work | Year | Licence |
|------|------|-----:|---------|
| `gaelic-calder-1923.pdf` | George Calder, *A Gaelic Grammar* | 1923 | public domain |
| `welsh-evans-1910.pdf` | D. Simon Evans, *The Elements of Welsh Grammar* | 1910 | public domain |
| `welsh-morris-jones-1913.pdf` | John Morris-Jones, *A Welsh Grammar, Historical and Comparative* | 1913 | public domain |
| `irish-christian-brothers-1920.pdf` | *First Irish Grammar* | 1920 | public domain |
| `french-liberte-2022.pdf` | Gretchen Angelo and Emmanuelle Remy, *Liberté* | 2022 | CC BY-SA 4.0 |
| `french-bevier-1896.pdf` | Louis Bevier, *A French Grammar* | 1896 | public domain |
| `italian-daccordo.pdf` | Italian faculty, University of Iowa, *D'Accordo!* | 2021 | CC BY-NC-SA 4.0 |
| `spanish-olmsted-1920.pdf` | Everett Olmsted, *First Course in Spanish* | 1920 | public domain |
| `spanish-coester-1912.pdf` | Alfred Coester, *A Spanish Grammar* | 1912 | public domain |
| `irish-caighdean-oifigiuil-2017.pdf` | Houses of the Oireachtas, *Gramadach na Gaeilge: An Caighdeán Oifigiúil* | 2017 | no licence stated |
| `gaelic-goc-2009.pdf` | SQA, *Gaelic Orthographic Conventions* | 2009 | reproduction limited to SQA qualifications |
| `spanish-yo-puedo-1-2021.pdf` | Elizabeth Silvaggio-Adams and Rocío Vallejo-Alegre, *Yo puedo: para empezar* | 2021 | CC BY-NC 4.0 |
| `italian-spunti-elementare-1-2019.pdf` | Daniel Leisawitz and Daniela Viale, *Spunti: Italiano elementare 1* | 2019 | CC BY-NC-SA 4.0 |
| `french-interactif-ed4-2019.pdf` | Karen Kelton, Nancy Guilloteau and Carl Blyth, *Français interactif*, 4th edn | 2019 | CC BY 4.0 |

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
safe to consult and to seed grammar rules from, since a rule statement is not
their text, but neither may be redistributed with the application.

The Celtic school grammars are all from before the modern spelling reforms —
Calder predates the 1981 Gaelic Orthographic Conventions — so their example
sentences need reading before they reach a learner. GOC 2009 and *An Caighdeán
Oifigiúil* 2017 give the current forms for Gaelic and Irish, so a rule taken
from Calder or the Christian Brothers can be checked against them. Welsh has no
counterpart here: no modern Welsh grammar with a usable licence was found, and
the two 1910s works stand alone.

## Checking a new document

A PDF built from page scans holds no text, and `pdf-extract` returns nothing
from one. Two candidates were rejected for exactly that: the Google Books scans
of Young's *An Italian Grammar* and Fuentes's *A Practical Spanish Grammar*
carry text for the copyright notice and nothing else. A third, the *Libro Libre*
Spanish open textbook, is encrypted with a key `lopdf` will not open and fails
with `unsupported key length`.

`pdf-text-check` in the agent-tools store reports characters per page, which
separates the three cases before an import is attempted.

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
