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
    manufacture_date: str | None = None   # the FIFO key; printed by the supplier

    @property
    def key(self) -> str:
        parsed = to_date_key(self.receipt_date)
        return parsed.key if parsed else self.receipt_date

    @property
    def manufacture_key(self) -> str | None:
        if not self.manufacture_date:
            return None
        parsed = to_date_key(self.manufacture_date)
        return parsed.key if parsed else self.manufacture_date


@dataclass(frozen=True)
class MatchResult:
    decision: Decision
    lot_id: str | None
    best_distance: float | None
    runner_up_distance: float | None
    reason: DeferReason | None = None
    # Which date on the box identified the lot. Worth recording: a lock off the
    # printed 製造日 is a stronger claim than one off the hand-stamped 進貨日.
    matched_on: str | None = None

    @property
    def locked(self) -> bool:
        return self.decision is Decision.LOCK


def _match_on(
    recognized: DateKey,
    candidates: list[Candidate],
    key_of,
    field: str,
    max_distance: float,
    min_margin: float,
) -> MatchResult:
    """Score one read date against one field of the candidate lots."""
    usable = [c for c in candidates if key_of(c)]
    if not usable:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_CANDIDATES, field)

    scored = sorted(
        ((confusion_distance(recognized.key, key_of(c)), c) for c in usable),
        key=lambda pair: pair[0],
    )
    best_distance, best = scored[0]
    runner_up_distance = scored[1][0] if len(scored) > 1 else None

    if best_distance > max_distance:
        return MatchResult(
            Decision.DEFER, None, best_distance, runner_up_distance,
            DeferReason.NO_CANDIDATE_IN_RANGE, field,
        )

    if runner_up_distance is not None and (runner_up_distance - best_distance) < min_margin:
        return MatchResult(
            Decision.DEFER, None, best_distance, runner_up_distance,
            DeferReason.AMBIGUOUS, field,
        )

    return MatchResult(Decision.LOCK, best.lot_id, best_distance, runner_up_distance, None, field)


def match_candidates(
    recognized: DateKey | None,
    candidates: list[Candidate],
    *,
    manufacture: DateKey | None = None,
    max_distance: float = 1.5,
    min_margin: float = 1.0,
) -> MatchResult:
    """Identify which lot the photographed box is, from the dates on it.

    Two dates can name it, and 製造日 is tried first for two reasons: it is the
    field FIFO now sorts on, and it is machine-printed by the supplier, whereas
    the receipt date is a hand-applied rubber stamp — the single hardest thing
    on the box to read (§6 R1). Preferring the printed field is preferring the
    legible one.

    Falling through to the receipt date is not just a backstop: when two lots
    share a manufacture date the 製造日 pass is genuinely ambiguous, and the
    receipt date is exactly what tells them apart.

    Both passes still run against the closed candidate set of in-stock lots
    (§2.2), so a misread lands on a real lot or on nothing — never on a date
    that was never received.
    """
    if not candidates:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_CANDIDATES)
    if recognized is None and manufacture is None:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_RECOGNITION)

    attempts: list[MatchResult] = []
    if manufacture is not None:
        attempt = _match_on(manufacture, candidates, lambda c: c.manufacture_key,
                            "manufacture_date", max_distance, min_margin)
        if attempt.locked:
            return attempt
        attempts.append(attempt)

    if recognized is not None:
        attempt = _match_on(recognized, candidates, lambda c: c.key,
                            "receipt_date", max_distance, min_margin)
        if attempt.locked:
            return attempt
        attempts.append(attempt)

    if not attempts:
        return MatchResult(Decision.DEFER, None, None, None, DeferReason.NO_RECOGNITION)
    # Report the near miss over the blank one: "nothing was close" is more
    # actionable than "that field was empty on every lot".
    ranked = sorted(attempts, key=lambda r: (r.reason is DeferReason.NO_CANDIDATES,
                                             r.best_distance if r.best_distance is not None else 9e9))
    return ranked[0]


def _sort_key(candidate: Candidate) -> tuple:
    """FIFO orders by 製造日期 — how old the stock actually is.

    Receipt date only says when it reached us, which a slow supplier or a late
    delivery can scramble: film made in March but delivered in August is older
    stock than film made in July and delivered in June. Shipping by arrival
    order would leave the genuinely older film sitting.

    A lot with no manufacture date sorts LAST, not first. Unknown age must not
    jump ahead of known age — pointing at an undated lot over one that is
    provably old would be worse than the paper process this replaces. Receipt
    date breaks ties, then lot id so the order is stable.
    """
    return (
        candidate.manufacture_date is None,
        candidate.manufacture_date or "",
        candidate.receipt_date,
        candidate.lot_id,
    )


def fifo_expected(candidates: list[Candidate]) -> list[str]:
    """Lot ids FIFO accepts: every lot sharing the earliest 製造日期.

    This is the JUDGEMENT: lots made the same day are equally legal, so drawing
    any of them passes. Use `fifo_target` for what to TELL someone to take — a
    screen marking two lots "應領" has said nothing.

    With no manufacture date anywhere, this falls back to earliest receipt date:
    a shelf of undated stock would otherwise get no guidance at all, which is
    worse than guidance from a weaker signal.
    """
    if not candidates:
        return []
    dated = [c for c in candidates if c.manufacture_date]
    if not dated:
        earliest_receipt = min(c.receipt_date for c in candidates)
        return [c.lot_id for c in candidates if c.receipt_date == earliest_receipt]
    earliest = min(c.manufacture_date for c in dated if c.manufacture_date)
    return [c.lot_id for c in dated if c.manufacture_date == earliest]


def fifo_target(candidates: list[Candidate]) -> str | None:
    """The single lot to point at: earliest 製造日期, then earliest 進貨日期.

    Guidance and judgement are deliberately different. Judgement stays
    permissive because refusing a same-age lot would block a draw that is
    genuinely fine; guidance has to name one box, because "take either of these
    two" is not an instruction anyone can act on while holding a roll of film.
    """
    accepted = set(fifo_expected(candidates))
    if not accepted:
        return None
    pool = [c for c in candidates if c.lot_id in accepted]
    return min(pool, key=_sort_key).lot_id


def fifo_basis(candidates: list[Candidate]) -> str:
    """Which field the current guidance actually rests on.

    Shown on screen: guidance derived from receipt date because nothing carries
    a manufacture date is a weaker claim, and saying so is the difference
    between a stated fallback and a silent downgrade.
    """
    return "製造日期" if any(c.manufacture_date for c in candidates) else "進貨日期"
