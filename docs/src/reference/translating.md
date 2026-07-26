# Translating the documentation

This book is translated with [Gettext](https://www.gnu.org/software/gettext/),
the same machinery used for translating software, by way of
[`mdbook-i18n-helpers`](https://github.com/google/mdbook-i18n-helpers). The
arrangement is worth explaining before the commands, because it determines what
a translator actually edits.

## How it fits together

The English Markdown under `src/` is the single source of truth, and it is
never duplicated. Instead, every translatable string is extracted into a
template, `po/messages.pot`, and each language keeps its own catalogue of
translations at `po/<lang>.po` pairing each English string with its rendering.
At build time the `gettext` preprocessor substitutes the strings for the
requested language; anything left untranslated falls back to the English, so a
partial translation produces a usable book rather than a broken one.

This is why the configuration in `book.toml` is so small:

```toml
[preprocessor.gettext]
after = ["links"]
```

Running after the `links` preprocessor matters: it means any file pulled in by
an include directive is translated along with the file that includes it, rather
than being skipped.

The preprocessor is a no-op when building the source language, so the ordinary
`mdbook build` is unaffected by any of this.

## Prerequisites

The devshell provides everything already. Outside it, you need `mdbook`, the
helpers, and GNU Gettext's command-line tools:

```sh
cargo install mdbook-i18n-helpers --version 0.4.0 --locked
```

## Starting a new translation

Create the catalogue from the template, naming it for the language code — `gd`
for Scottish Gaelic, `cy` for Welsh, and so on:

```sh
cd docs
msginit -i po/messages.pot -l gd -o po/gd.po --no-translator
```

Then edit `po/gd.po`, filling in the `msgstr` entries. Leave anything you are
unsure of empty and it will render in English.

## Building a translation

```sh
MDBOOK_BOOK__LANGUAGE=gd mdbook build -d book/gd
```

The result lands in `book/gd/`, alongside the English book in `book/`. CI does
this automatically for every `po/*.po` file it finds, so a new translation
needs no pipeline changes — adding the file is enough.

## Keeping catalogues current

The template goes stale as soon as the English text changes, so regenerate it
and merge the differences into each language:

```sh
cd docs
MDBOOK_OUTPUT='{"xgettext": {}}' mdbook build -d po
msgmerge --update po/gd.po po/messages.pot
```

`msgmerge` preserves existing translations, marks those whose source text has
changed as fuzzy for review, and adds the new strings as untranslated. Doing
this in the same commit as a substantial documentation change spares
translators from reconstructing what moved.
