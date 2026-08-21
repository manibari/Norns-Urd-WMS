"""Image -> label fields. Stateless, and deliberately swappable.

The provider boundary exists because this is the one part of the system that
changes whenever a vendor or model iterates (M7 architecture, component table).
Nothing outside this module may import a vendor SDK type.

The prompt's single most important job is to make the model say "I could not
read it" instead of inventing a plausible date. A hallucinated date that lands
on a real in-stock lot is the one failure this whole design is built to
prevent, and it is invisible downstream — it posts cleanly and logs nothing.
"""

from __future__ import annotations

import base64
import mimetypes
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol

_SYSTEM_PROMPT = """\
You read labels off boxes of food-packaging film in a Taiwanese factory.

A box carries up to three things you care about:
1. A printed supplier label (English and/or Chinese) with an item code and a
   manufacture date. Formats seen in the field: `2003.T7320BC-340X900-P1`,
   `2025年09月26日`, `26-Sep-25`.
2. A hand-stamped red rubber stamp applied by the warehouse on acceptance,
   carrying the company name and a RECEIPT DATE, e.g. `2026-08-12`. This stamp
   is usually on the side of the box. It is often faint, crooked, smudged, or
   overlapping printed text.
3. Nothing at all — some faces of the box carry no stamp.

Report only what you can actually see.

CRITICAL: if a field is not legible, or is not present in this image, return
null for it. Do not infer it, do not complete it from the other fields, and do
not guess a plausible date. A null costs a human ten seconds. A wrong value
that looks right is worse than useless — it is acted on as if it were true.

Set each confidence to your genuine certainty that the characters you report
are the characters on the box. If you had to reconstruct any character from
context rather than read it, that field is below 0.5.

Transcribe dates exactly as printed or stamped, including the original format.
Do not reformat them.
"""

_USER_PROMPT = "Read this box label. Return null for anything you cannot read."


@dataclass
class Recognition:
    """What one recognition attempt produced."""

    item_code: str | None = None
    manufacture_date: str | None = None
    receipt_date: str | None = None
    item_code_confidence: float = 0.0
    manufacture_date_confidence: float = 0.0
    receipt_date_confidence: float = 0.0
    stamp_visible: bool = False
    notes: str = ""
    error: str | None = None
    usage: dict[str, int] = field(default_factory=dict)

    @property
    def failed(self) -> bool:
        return self.error is not None


class RecognitionProvider(Protocol):
    def recognize(self, image_path: Path) -> Recognition: ...


def _encode(image_path: Path) -> tuple[str, str]:
    media_type = mimetypes.guess_type(image_path.name)[0] or "image/jpeg"
    return media_type, base64.standard_b64encode(image_path.read_bytes()).decode("utf-8")


class ClaudeProvider:
    """Recognition via the Anthropic Messages API with a constrained output schema."""

    def __init__(
        self,
        model: str = "claude-opus-5",
        *,
        thinking: bool = True,
        effort: str = "high",
        max_tokens: int = 4096,
    ) -> None:
        import anthropic  # imported lazily so the mock provider needs no SDK
        from pydantic import BaseModel, Field

        class LabelReading(BaseModel):
            item_code: str | None = Field(description="Item code exactly as printed, or null")
            manufacture_date: str | None = Field(description="Manufacture date in its original format, or null")
            receipt_date: str | None = Field(description="Receipt date from the red acceptance stamp, original format, or null")
            item_code_confidence: float = Field(ge=0.0, le=1.0)
            manufacture_date_confidence: float = Field(ge=0.0, le=1.0)
            receipt_date_confidence: float = Field(ge=0.0, le=1.0)
            stamp_visible: bool = Field(description="Whether a red acceptance stamp is visible at all")
            notes: str = Field(description="What made this hard to read, if anything")

        self._client = anthropic.Anthropic()
        self._schema = LabelReading
        self._model = model
        self._thinking = thinking
        self._effort = effort
        self._max_tokens = max_tokens

    def recognize(self, image_path: Path) -> Recognition:
        media_type, data = _encode(image_path)
        request: dict = {
            "model": self._model,
            "max_tokens": self._max_tokens,
            "system": _SYSTEM_PROMPT,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}},
                    {"type": "text", "text": _USER_PROMPT},
                ],
            }],
            "output_format": self._schema,
        }
        if self._thinking:
            request["thinking"] = {"type": "adaptive"}
        if self._effort:
            request["output_config"] = {"effort": self._effort}

        try:
            response = self._client.messages.parse(**request)
        except Exception as exc:  # noqa: BLE001 — a provider outage must not stop the run
            return Recognition(error=f"{type(exc).__name__}: {exc}")

        parsed = response.parsed_output
        if parsed is None:
            return Recognition(error="empty_parsed_output")

        return Recognition(
            item_code=parsed.item_code,
            manufacture_date=parsed.manufacture_date,
            receipt_date=parsed.receipt_date,
            item_code_confidence=parsed.item_code_confidence,
            manufacture_date_confidence=parsed.manufacture_date_confidence,
            receipt_date_confidence=parsed.receipt_date_confidence,
            stamp_visible=parsed.stamp_visible,
            notes=parsed.notes,
            usage={
                "input_tokens": response.usage.input_tokens,
                "output_tokens": response.usage.output_tokens,
            },
        )


class ReplayProvider:
    """Replays recognitions recorded by an earlier run.

    Sweeping max_distance / min_margin / confidence thresholds must not re-bill
    the API — the recognition step is fixed, only the matching rules change.
    Record once, sweep offline.
    """

    def __init__(self, recorded: dict[str, Recognition]) -> None:
        self._recorded = recorded

    def recognize(self, image_path: Path) -> Recognition:
        found = self._recorded.get(image_path.name)
        if found is None:
            return Recognition(error="not_in_recording")
        return found
