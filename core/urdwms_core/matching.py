"""Pick a lot out of the closed candidate set, or refuse to.

This is where the PoC's most important number is decided. The matcher has two
knobs and they guard different failures:

  max_distance  how far a recognised date may sit from a candidate before we
                stop believing it at all. Too loose and a hallucinated date
                lands on a real lot.
  min_margin    how much better the best candidate must be than the runner-up.
                Without this, two lots received days apart turn a one-character
                OCR slip into a confident pick of the wrong lot — a false hit,
                which posts a FIFO violation in silence.

Refusing (`defer`) is cheap: the operator picks the lot by hand (US-4). A false
hit is not recoverable, so both knobs are biased toward refusing.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum

from .normalize import DateKey, confusion_distance, to_date_key


class Decision(str, Enum):
    LOCK = "lock"
    DEFER = "defer"


class DeferReason(str, Enum):
    NO_RECOGNITION = "no_recognition"          # model returned nothing usable
    LOW_CONFIDENCE = "low_confidence"          # below the configured threshold
    NO_CANDIDATE_IN_RANGE = "no_candidate"     # nothing within max_distance
    AMBIGUOUS = "ambiguous"                    # two candidates too close together
    NO_CANDIDATES = "empty_candidate_set"      # item has no stock on hand


@dataclass(frozen=True)
class Candidate:
    """One lot of this item currently on hand."""

    lot_id: str
    receipt_date: str          # ISO, authoritative — it came from receiving, not from a photo

    @property
    def key(self) -> str:
        parsed = to_date_key(self.receipt_date)
        return parsed.key if parsed else self.receipt_date


@dataclass(frozen=True)
class MatchResult:
    decision: Decision
    lot_id: str | None
    best_distance: float | None
    runner_up_distance: float | None
    reason: DeferReason | None = None

    @property
    def locked(self) -> bool:
        return self.decision is Decision.LOCK


def match_candidates(
    recognized: DateKey | None,
    candidates: list[Candidate],
    *,
    max_distance: float = 1.5,
    min_margin: float = 1.0,
) -> MatchResult:
    """Match a recognised receipt date against the lots currently in stock."""
    if not candidates:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_CANDIDATES)
    if recognized is None:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_RECOGNITION)

    scored = sorted(
        ((confusion_distance(recognized.key, c.key), c) for c in candidates),
        key=lambda pair: pair[0],
    )
    best_distance, best = scored[0]
    runner_up_distance = scored[1][0] if len(scored) > 1 else None

    if best_distance > max_distance:
        return MatchResult(
            Decision.DEFER, None, best_distance, runner_up_distance,
            DeferReason.NO_CANDIDATE_IN_RANGE,
        )

    if runner_up_distance is not None and (runner_up_distance - best_distance) < min_margin:
        return MatchResult(
            Decision.DEFER, None, best_distance, runner_up_distance,
            DeferReason.AMBIGUOUS,
        )

    return MatchResult(Decision.LOCK, best.lot_id, best_distance, runner_up_distance)


def fifo_expected(candidates: list[Candidate]) -> list[str]:
    """Lot ids that FIFO would have the operator take: all lots sharing the earliest receipt date.

    Same-day lots are equally legal (requirement US-3), so this returns a list,
    not a single lot.
    """
    if not candidates:
        return []
    earliest = min(c.receipt_date for c in candidates)
    return [c.lot_id for c in candidates if c.receipt_date == earliest]
