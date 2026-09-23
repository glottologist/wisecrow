# ADR 0001 — Grammar mastery holds two states, not one

**Status:** Accepted
**Date:** 2026-09-22
**Design plan:** `agents/2026-09-22-001-feature-cefr-grammar-mastery-design.md`

## Context

Wisecrow schedules vocabulary with FSRS, which models a card as stability, difficulty and a due date, and from those predicts the probability that a learner can recall the card at a given moment. When we came to model grammatical knowledge — a score per grammar point, driving which exercises the learner receives — the obvious move was to reuse that machinery wholesale, since it is already written, already tested, and already synchronised to mobile devices.

Reuse turns out to answer only half the question. FSRS retrievability answers "how likely is this learner to recall this point right now", which decays with time since the last encounter and is precisely the right quantity for deciding *when* a point should come back. It is a poor quantity for showing the learner what they know. A point answered correctly every time for a year reads as low retrievability the moment enough time passes, and a learner looking at a map of their own knowledge would see alarm where there is none. Conversely a point the learner has fumbled three times in a row, but fumbled recently, reads as high retrievability. The brainmap we set out to build is modelled on Kwiziq's, whose red cells mean "mistakes you keep making" — a claim about repeated error, not about decay.

## Decision

Each `(user, grammar_rule)` pair carries two projections rather than one. An FSRS state — stability, difficulty, repetitions, lapses, state and due date, structurally identical to a vocabulary card — determines when the point is next served. An exponentially weighted accuracy figure over the learner's attempts determines what the brainmap displays, with green at or above 0.85, amber between 0.60 and 0.85, red below 0.60 once at least three attempts exist, and grey where the point has never been attempted.

Crucially, neither figure is a primary record. Every attempt appends to `grammar_review_events`, an immutable stream carrying the rule, the item, the rating, correctness, timestamp and originating device; both the FSRS state and the accuracy figure are recomputed by replaying that stream from a stored baseline. The existing vocabulary path already works this way, which is what makes the duplication affordable.

## Alternatives considered

**FSRS alone, with retrievability as the displayed confidence.** The cheapest option, and the one we started from: one state, one set of invariants, and `rs-fsrs` exposes `Card::get_retrievability` so the number is available for free. We rejected it because the displayed figure would then wander for reasons the learner cannot act on — time passing is not a mistake — and because the selection rule we actually want, "surface the points they keep getting wrong", cannot be expressed in retrievability at all.

**Accuracy alone, with a bespoke scheduler.** Kwiziq's own vocabulary is confidence percentages, so a rolling accuracy score is the most faithful single number. We rejected it because it carries no notion of when a point should return, which would have meant designing, testing and synchronising a second scheduling algorithm alongside the FSRS one we already trust, purely to avoid storing eight extra columns.

## Consequences

The pleasant consequence is that grammar scheduling inherits the vocabulary path's replay, baseline, conflict and offline-reconciliation semantics unchanged; mobile synchronisation for mastery becomes a change feed and a data-transfer object rather than a new algorithm.

The cost is that two projections can, in principle, disagree, and every change to the event schema must keep both replays correct. We accept that risk on the strength of its mitigation: because both are derived from the same stream and neither is authoritative, a disagreement is always a bug in a projection and never a corrupted record, and recovery is a replay rather than a repair.

A further consequence worth recording is that the accuracy figure is only as trustworthy as the item bank feeding it. An item that marks a correct answer wrong does not merely annoy; it writes a false signal into the learner's map and causes the selection algorithm to drill a point they already know. This is the reasoning behind the human promotion gate on generated items described in the design plan, and the two decisions should be revisited together if either is ever relaxed.
