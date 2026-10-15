# ADR 0002 — The corpus unit mines, glosses, and voices phrases

**Status:** Accepted
**Date:** 2026-10-15
**Design plan:** `agents/2026-10-15-001-feature-corpus-phrases-design.md`

## Context

The 13 October pipeline design put phrase extraction, phrase translation, and phrase audio out of scope in the same sentence. The corpus unit did one archive, one ranking, or one batch of single-word audio and pictures, then exited. That bound was comfortable while phrases were a hand command. Welsh shows the volume it was avoiding: a hand run stored 725,397 staged phrases and glossed 2,000 of them. Gaelic stored 6,365 and glossed 302, and the unit had voiced none of them. French, Italian, and Spanish had no phrases in the deck at all.

## Decision

The corpus unit mines, glosses, and voices phrases, with no numeric ceiling. A mine starts only after every configured archive for the pair is recorded and the rank stamp matches the live evidence checksum. It pages every qualifying 2–5 token phrase into `phrases`, resumes from the last committed page if it is killed, and is kept until its stamp matches. A gloss turn repairs a broken link from the stored English text, without calling the model, then sends 100 unglossed phrases to the model. Phrase speech voices 100 glossed phrases that lack a current clip, on the same speech path as a word, and does not fetch a picture. Word audio remains a batch of up to 200 single-word rows. After the stamps match, that cycle is gloss, phrase speech, then word audio, so word audio arrives on every third step. `extract-phrases` and `translate-phrases` remain the hand commands. One hundred is the restart slice of one invocation, not a cap.

## Alternatives considered

**A numeric cap.** A ceiling on staged phrases, glosses, or clips would have kept a French mine from filling the database and the model queue. We rejected it because the request is every qualifying phrase, and a cap would be a product rule the hand commands do not have.

**Folding speech into the gloss step.** Voicing a phrase in the same invocation that glossed it would have delivered a clip sooner. We rejected it because a gloss outage and a speech outage are different failures, and because the word-audio batch also searches for a picture. A phrase must be voiced with no image provider configured. Separate turns keep that true, and keep word audio on its own step.

## Consequences

A French mine can occupy the unit for hours. While it runs, glossing, phrase speech, and word audio wait, and an unsettled archive or a stale rank on any configured pair still cuts in front of it. After the stamps match, word audio arrives on every third step.

Phrase pictures are not fetched. Existing English glosses are not rewritten. A staged phrase that a later mine counts below five sentences is left in place. The deck query is unchanged, and the grammar unit is unchanged. Pairs that are not in the pair file are not mined, glossed, or voiced.
