"""Normalisation of dates and item codes read off packaging labels.

This module is deliberately dependency-free and pure: every function here is
also needed by the v1 manual-entry path (requirement US-4), so it is written
as production code, not throwaway PoC glue.

The central idea (requirement section 2.2): recognition does not have to read
the receipt date *correctly*. It has to produce something close enough to pick
the right lot out of a small set of lots already known to be in stock. So the
normaliser never throws away an unparseable value — it degrades it into a
comparable key and lets the matcher decide.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

# Characters an OCR pass confuses for one another. Substituting within a group
# is charged half price by `confusion_distance`, so `2026-08-l2` stays close to
# `2026-08-12` while `2026-08-92` does not.
_CONFUSION_GROUPS: tuple[frozenset[str], ...] = (
    frozenset("0OoDQ"),
    frozenset("1lI|i7"),
    frozenset("2Zz"),
    frozenset("5Ss"),
    frozenset("6bG"),
    frozenset("8B"),
    frozenset("9gq"),
    frozenset("4A"),
    frozenset("3E"),
)

_CONFUSABLE: dict[str, frozenset[str]] = {
    ch: group for group in _CONFUSION_GROUPS for ch in group
}

_MONTHS: dict[str, int] = {
    m: i
    for i, m in enumerate(
        ("jan", "feb", "mar", "apr", "may", "jun",
         "jul", "aug", "sep", "oct", "nov", "dec"),
        start=1,
    )
}

# `2025年09月26日`
_CJK_DATE = re.compile(r"(\d{2,4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日?")
# `26-Sep-25`, `26 Sep 2025`
_DMY_ALPHA = re.compile(r"(\d{1,2})[\s\-/]*([A-Za-z]{3,9})[\s\-/]*(\d{2,4})")
# `2026-08-12`, `2026/08/12`, and OCR-damaged variants such as `2026-08-l2`.
#
# Month and day accept any alphanumeric, not a confusable-only whitelist, and
# the trailing lookahead forbids stopping mid-token. Both guard the same trap:
# a narrow class lets the regex match a PREFIX of a damaged field and silently
# drop the rest, so `2026-08-1X` parses as a clean `2026-08-01`. That turns
# "could not read it" into "read it confidently, wrong" — and if 08-01 happens
# to be in stock, into a false hit. Unreadable must stay visibly unreadable.
_YMD_LOOSE = re.compile(
    r"([0-9OoDQlI|iZzSsbGBgq]{4})[\s\-/.]([0-9A-Za-z|]{1,2})[\s\-/.]([0-9A-Za-z|]{1,2})(?![0-9A-Za-z|])"
)

# `22.03.2026`, `22/03/2026` — day first, four-digit year last.
#
# This is what Sealed Air prints in the `Date:` field, and the format the whole
# FIFO key was silently failing on: the year-first pattern above needs four
# digits up front, so `22.03.2026` fell through to the bare-digits fallback and
# became the key `22032026` — DDMMYYYY sitting in YYYYMMDD positions. A lot
# received as `2026-03-22` keys to `20260322`, so the same day could never match
# itself, and every box from this supplier deferred to manual selection.
#
# Day-first is an assumption, and it is only safe to make it silently when the
# first field cannot be a month. `22.03.2026` is unambiguous; `03.04.2026` is
# not, and guessing there would turn "could not read it" into "read it
# confidently, wrong" — the exact trade this module refuses elsewhere. So the
# ambiguous case still yields a key (built day-first, since that is this
# label's format) but no `iso`: it can be matched against the closed candidate
# set, and cannot be stored as a fact.
_DMY_NUMERIC = re.compile(r"(\d{1,2})[\-/.](\d{1,2})[\-/.](\d{4})(?![0-9])")

_SEPARATORS = re.compile(r"[\s\-/.年月日]+")


@dataclass(frozen=True)
class DateKey:
    """A date reduced to eight comparable characters.

    `key` is always 8 characters in YYYYMMDD positions. It may contain
    non-digits when OCR damaged the value — that is the point, since
    `confusion_distance` can still score it against a real lot date.

    `iso` is only set when the value parsed cleanly. Code that needs a real
    date (storing a lot, sorting by receipt date) must use `iso` and treat
    `None` as "not readable"; code that only compares against candidates uses
    `key`.
    """

    key: str
    iso: str | None
    raw: str

    @property
    def is_clean(self) -> bool:
        return self.iso is not None


def _expand_year(value: str) -> str:
    """`25` -> `2025`. Two-digit years on these labels are always this century."""
    if len(value) == 4:
        return value
    if len(value) == 2:
        return f"20{value}"
    return value.rjust(4, "0")


def _iso_if_valid(year: str, month: str, day: str) -> str | None:
    if not (year + month + day).isdigit():
        return None
    y, m, d = int(year), int(month), int(day)
    if not (1900 <= y <= 2999 and 1 <= m <= 12 and 1 <= d <= 31):
        return None
    return f"{y:04d}-{m:02d}-{d:02d}"


def to_date_key(raw: str | None) -> DateKey | None:
    """Parse any of the four label formats — plus OCR-damaged variants — into a key.

    Returns None only when nothing date-shaped is present at all. A value that
    looks like a date but does not parse still yields a DateKey with iso=None,
    because it remains useful for candidate matching.
    """
    if raw is None:
        return None
    text = raw.strip()
    if not text:
        return None

    if match := _CJK_DATE.search(text):
        year, month, day = (_expand_year(match.group(1)), match.group(2), match.group(3))
        return DateKey(f"{year}{month.zfill(2)}{day.zfill(2)}", _iso_if_valid(year, month, day), text)

    if match := _DMY_ALPHA.search(text):
        month_num = _MONTHS.get(match.group(2)[:3].lower())
        if month_num is not None:
            day, year = match.group(1), _expand_year(match.group(3))
            month = f"{month_num:02d}"
            return DateKey(f"{year}{month}{day.zfill(2)}", _iso_if_valid(year, month, day.zfill(2)), text)

    if match := _DMY_NUMERIC.search(text):
        day, month, year = match.group(1).zfill(2), match.group(2).zfill(2), match.group(3)
        # Only claim a real date when the first field cannot be a month.
        unambiguous = int(match.group(1)) > 12
        iso = _iso_if_valid(year, month, day) if unambiguous else None
        return DateKey(f"{year}{month}{day}", iso, text)

    if match := _YMD_LOOSE.search(text):
        year, month, day = match.group(1), match.group(2).zfill(2), match.group(3).zfill(2)
        return DateKey(f"{year}{month}{day}", _iso_if_valid(year, month, day), text)

    # Last resort: strip separators and hope for a bare YYYYMMDD / YYMMDD.
    stripped = _SEPARATORS.sub("", text)
    if len(stripped) == 8:
        year, month, day = stripped[:4], stripped[4:6], stripped[6:]
        return DateKey(stripped, _iso_if_valid(year, month, day), text)
    if len(stripped) == 6:
        year, month, day = _expand_year(stripped[:2]), stripped[2:4], stripped[4:]
        return DateKey(f"{year}{month}{day}", _iso_if_valid(year, month, day), text)
    return None


def confusion_distance(a: str, b: str) -> float:
    """Levenshtein distance where OCR-confusable substitutions cost half.

    Both inputs are short (8 characters), so the plain DP is fine.
    """
    if a == b:
        return 0.0

    def sub_cost(x: str, y: str) -> float:
        if x == y:
            return 0.0
        group = _CONFUSABLE.get(x)
        return 0.5 if group is not None and y in group else 1.0

    previous = [float(i) for i in range(len(b) + 1)]
    for i, ch_a in enumerate(a, start=1):
        current = [float(i)]
        for j, ch_b in enumerate(b, start=1):
            current.append(
                min(
                    previous[j] + 1.0,
                    current[j - 1] + 1.0,
                    previous[j - 1] + sub_cost(ch_a, ch_b),
                )
            )
        previous = current
    return previous[-1]


def normalize_item_code(raw: str | None, alias_map: dict[str, str] | None = None) -> str | None:
    """Resolve a label or form abbreviation to a canonical item code.

    `alias_map` is per-factory configuration (requirement US-11), e.g.
    `{"T7320-P1": "2003.T7320BC-340X900-P1"}`. Lookup order: exact canonical,
    configured alias, then a containment fallback so an unconfigured
    abbreviation still has a chance of resolving.
    """
    if raw is None:
        return None
    text = re.sub(r"\s+", "", raw).upper()
    if not text:
        return None

    aliases = {k.upper(): v for k, v in (alias_map or {}).items()}
    canonical = set(aliases.values())

    if text in canonical:
        return text
    if text in aliases:
        return aliases[text]

    compact = text.replace("-", "").replace(".", "")
    for code in canonical:
        if compact and compact in code.replace("-", "").replace(".", ""):
            return code
    return text
