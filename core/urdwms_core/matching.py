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


class ItemDeferReason(str, Enum):
    NO_CODE_READ = "no_code_read"        # label code unreadable
    NO_ITEM_MATCH = "no_item_match"      # read a code, nothing in the master matches
    AMBIGUOUS_ITEM = "ambiguous_item"    # more than one 型號 could be it


@dataclass(frozen=True)
class ItemMatch:
    decision: Decision
    item_id: str | None
    matched_on: str | None = None        # "supplier_code" | "model_in_label"
    reason: ItemDeferReason | None = None
    contenders: tuple[str, ...] = ()

    @property
    def locked(self) -> bool:
        return self.decision is Decision.LOCK


def _compact(value: str) -> str:
    return value.upper().replace("-", "").replace(".", "").replace(" ", "").replace("_", "")


def match_item_code(
    label_code: str | None,
    items: list[tuple[str, str | None, str | None]],
    model_code: str | None = None,
) -> ItemMatch:
    """Work out which item a label belongs to.

    `items` is (id, 型號, 箱上完整料號). Recognition reports the 型號 it read
    (`T6284BA`) and, when the label carries one, the full supplier part number it
    sits inside (`2003.T7320BC-340X900-P1`). Both are searched: an exact hit on
    the full code first, then the 型號 as a substring.

    型號 is optional (the acceptance form's 脫氧劑 line has none), so an item
    with neither a supplier code nor a 型號 cannot be matched from a label at
    all. That is a correct outcome, not a failure — the operator picks it by hand.

    Same discipline as the lot matcher: lock only on a unique answer. Two 型號
    like `T6050B` and `T6050BSW` both sit inside the same label string, and
    picking the first one silently draws stock from the wrong item — so an
    ambiguous read defers to a human instead of guessing.
    """
    read = " ".join(filter(None, (model_code, label_code))).strip()
    if not read:
        return ItemMatch(Decision.DEFER, None, reason=ItemDeferReason.NO_CODE_READ)

    label = _compact(label_code or model_code or "")

    exact = [item_id for item_id, _, supplier_code in items
             if supplier_code and _compact(supplier_code) == label]
    if len(exact) == 1:
        return ItemMatch(Decision.LOCK, exact[0], matched_on="supplier_code")
    if len(exact) > 1:
        return ItemMatch(Decision.DEFER, None, reason=ItemDeferReason.AMBIGUOUS_ITEM,
                         contenders=tuple(exact))

    # Fall back to finding the 型號 inside the label string. Prefer the longest
    # match: `T6050BSW` beats `T6050B` on a label containing both, because the
    # longer one is the more specific reading, not a coincidence.
    # Search both what was read: the 型號 on its own, and the full string it may
    # have been embedded in.
    haystacks = [_compact(v) for v in (model_code, label_code) if v]
    contained = [(item_id, model) for item_id, model, _ in items
                 if model and _compact(model)
                 and any(_compact(model) in h for h in haystacks)]
    if not contained:
        return ItemMatch(Decision.DEFER, None, reason=ItemDeferReason.NO_ITEM_MATCH)
    if len(contained) == 1:
        return ItemMatch(Decision.LOCK, contained[0][0], matched_on="model_in_label")

    longest = max(len(_compact(model)) for _, model in contained)
    best = [item_id for item_id, model in contained if len(_compact(model)) == longest]
    if len(best) == 1:
        return ItemMatch(Decision.LOCK, best[0], matched_on="model_in_label")
    return ItemMatch(Decision.DEFER, None, reason=ItemDeferReason.AMBIGUOUS_ITEM,
                     contenders=tuple(sorted(item_id for item_id, _ in contained)))


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
