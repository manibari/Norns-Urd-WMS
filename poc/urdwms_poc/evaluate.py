"""Turn recognitions into the three rates the PoC exists to measure.

Never collapse these into one accuracy number. They are not interchangeable:

  hit        the system locked the right lot
  false_hit  the system locked the WRONG lot and believed it — this posts a
             FIFO violation with no error, no alert and a clean log
  defer      the system said it was unsure and handed off to a human (US-4)

A defer costs seconds. A false hit costs the audit trail its meaning. The
report therefore reports them separately and the pass criteria differ by an
order of magnitude.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from enum import Enum

from .matching import Candidate, Decision, DeferReason, MatchResult, match_candidates
from .normalize import to_date_key
from .recognition import Recognition


class Outcome(str, Enum):
    HIT = "hit"
    FALSE_HIT = "false_hit"
    DEFER = "defer"


@dataclass(frozen=True)
class Sample:
    """One annotated photo. Ground truth is transcribed from the physical box."""

    photo_id: str
    item_code_truth: str
    receipt_date_truth: str | None      # None when this face of the box carries no stamp
    manufacture_date_truth: str | None
    stratum: str
    candidates: list[Candidate]

    @property
    def stamp_present(self) -> bool:
        return self.receipt_date_truth is not None

    @property
    def candidate_bucket(self) -> str:
        n = len(self.candidates)
        if n <= 1:
            return "1"
        if n == 2:
            return "2"
        if n <= 4:
            return "3-4"
        return "5+"


@dataclass(frozen=True)
class Evaluation:
    sample: Sample
    recognition: Recognition
    match: MatchResult
    outcome: Outcome
    locked_receipt_date: str | None
    hallucinated_stamp: bool

    def row(self) -> dict:
        return {
            "photo_id": self.sample.photo_id,
            "stratum": self.sample.stratum,
            "candidates": len(self.sample.candidates),
            "outcome": self.outcome.value,
            "truth": self.sample.receipt_date_truth,
            "read": self.recognition.receipt_date,
            "locked": self.locked_receipt_date,
            "confidence": self.recognition.receipt_date_confidence,
            "distance": self.match.best_distance,
            "runner_up": self.match.runner_up_distance,
            "defer_reason": self.match.reason.value if self.match.reason else None,
            "hallucinated_stamp": self.hallucinated_stamp,
            "error": self.recognition.error,
            "notes": self.recognition.notes,
        }


def evaluate(
    sample: Sample,
    recognition: Recognition,
    *,
    confidence_threshold: float = 0.0,
    max_distance: float = 1.5,
    min_margin: float = 1.0,
) -> Evaluation:
    """Score one recognition against ground truth."""
    # A model that reports a receipt date for a face with no stamp on it has
    # invented one. Worth counting on its own: it is the hallucination the
    # closed candidate set is supposed to catch, measured directly.
    hallucinated = not sample.stamp_present and bool(recognition.receipt_date)

    def deferred(reason: DeferReason) -> Evaluation:
        return Evaluation(
            sample, recognition,
            MatchResult(Decision.DEFER, None, None, None, reason),
            Outcome.DEFER, None, hallucinated,
        )

    if recognition.failed:
        return deferred(DeferReason.NO_RECOGNITION)
    if not recognition.receipt_date:
        return deferred(DeferReason.NO_RECOGNITION)
    if recognition.receipt_date_confidence < confidence_threshold:
        return deferred(DeferReason.LOW_CONFIDENCE)

    result = match_candidates(
        to_date_key(recognition.receipt_date),
        sample.candidates,
        max_distance=max_distance,
        min_margin=min_margin,
    )

    if not result.locked:
        return Evaluation(sample, recognition, result, Outcome.DEFER, None, hallucinated)

    locked = next((c.receipt_date for c in sample.candidates if c.lot_id == result.lot_id), None)
    outcome = Outcome.HIT if locked == sample.receipt_date_truth else Outcome.FALSE_HIT
    return Evaluation(sample, recognition, result, outcome, locked, hallucinated)


def _rates(evaluations: list[Evaluation]) -> dict:
    total = len(evaluations)
    if total == 0:
        return {"n": 0, "hit_rate": None, "false_hit_rate": None, "defer_rate": None}
    counts = Counter(e.outcome for e in evaluations)
    return {
        "n": total,
        "hit": counts[Outcome.HIT],
        "false_hit": counts[Outcome.FALSE_HIT],
        "defer": counts[Outcome.DEFER],
        "hit_rate": counts[Outcome.HIT] / total,
        "false_hit_rate": counts[Outcome.FALSE_HIT] / total,
        "defer_rate": counts[Outcome.DEFER] / total,
    }


def summarize(evaluations: list[Evaluation]) -> dict:
    """Overall rates plus the two stratified tables the PoC spec requires."""
    by_bucket: dict[str, list[Evaluation]] = defaultdict(list)
    by_stratum: dict[str, list[Evaluation]] = defaultdict(list)
    for e in evaluations:
        by_bucket[e.sample.candidate_bucket].append(e)
        by_stratum[e.sample.stratum].append(e)

    no_stamp = [e for e in evaluations if not e.sample.stamp_present]
    return {
        "overall": _rates(evaluations),
        "by_candidate_count": {
            k: _rates(v) for k, v in sorted(by_bucket.items())
        },
        "by_stratum": {k: _rates(v) for k, v in sorted(by_stratum.items())},
        "defer_reasons": dict(Counter(
            e.match.reason.value for e in evaluations
            if e.outcome is Outcome.DEFER and e.match.reason
        )),
        "hallucination": {
            "n_faces_without_stamp": len(no_stamp),
            "invented_a_date": sum(1 for e in no_stamp if e.hallucinated_stamp),
        },
        "recognition_errors": sum(1 for e in evaluations if e.recognition.failed),
    }


def false_hit_upper_bound(false_hits: int, n: int) -> float | None:
    """95% upper bound on the false-hit rate.

    Zero observed false hits does not mean zero. With n=200 and 0 observed, the
    rule of three puts the 95% upper bound at 1.5% — above the 0.5% pass
    criterion. The PoC cannot prove that criterion; production monitoring (US-9)
    has to. Report this number so nobody reads "0 false hits" as "proven safe".
    """
    if n == 0:
        return None
    if false_hits == 0:
        return 3.0 / n
    return None
