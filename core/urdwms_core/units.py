"""Metres <-> boxes.

Suppliers label film by length (`LENGTH (m) 900`, `900M / 捲`) and deliveries
are sometimes counted the same way, but stock is kept in boxes: one box is one
roll, a draw deducts one, and there is no partial consumption (requirement Q6).
Metres are therefore an input aid and a display, never the ledger unit — the
moment stock is kept in metres, "half a roll left" becomes representable and
the whole no-partial-deduction decision unravels.

Which makes the remainder the interesting part. 9,500 m of 900 m rolls is not
10.5 boxes; it is 10 boxes and 500 m unaccounted for, and that discrepancy is
worth showing the person receiving the delivery rather than rounding away.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class BoxConversion:
    boxes: int
    remainder_m: int
    meters_per_box: int
    entered_m: int

    @property
    def exact(self) -> bool:
        return self.remainder_m == 0

    @property
    def note(self) -> str | None:
        if self.exact:
            return None
        return (f"{self.entered_m:,} 米 ÷ {self.meters_per_box:,} 米/箱 = {self.boxes} 箱，"
                f"剩 {self.remainder_m:,} 米對不上整箱")


def boxes_from_meters(meters: int, meters_per_box: int | None) -> BoxConversion | None:
    """Convert a metre count to whole boxes, keeping the remainder visible.

    Returns None when the item has no conversion configured — the caller must
    then ask for boxes directly rather than guess a rate.
    """
    if not meters_per_box or meters_per_box <= 0:
        return None
    if meters <= 0:
        return BoxConversion(0, 0, meters_per_box, meters)
    return BoxConversion(
        boxes=meters // meters_per_box,
        remainder_m=meters % meters_per_box,
        meters_per_box=meters_per_box,
        entered_m=meters,
    )


def meters_from_boxes(boxes: int, meters_per_box: int | None) -> int | None:
    """Display helper: how many metres a box count represents."""
    if not meters_per_box or meters_per_box <= 0:
        return None
    return boxes * meters_per_box
