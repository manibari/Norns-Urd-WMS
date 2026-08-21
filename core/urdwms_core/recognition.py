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

You are reading three things: 型號 (model code), 製造日期 (manufacture date),
and 進貨日期 (receipt date).

A box carries up to three things you care about:
1. A printed supplier label (English and/or Chinese) with a code and a
   manufacture date. The code appears in two forms and BOTH matter:
   - 型號 (model code): a short alphanumeric code such as `T6284BA`, `T7320BC`,
     `T6050BSW`, sometimes with a suffix like `-P1` or `-P6`. This is what the
     warehouse writes on its forms and is the primary thing to report.
   - The full supplier part number it is often embedded in, such as
     `2003.T7320BC-340X900-P1`.
   Report the 型號 in `model_code`, and the full string in `full_code` when one
   is present. If you can only see one of them, fill that one and leave the
   other null.
   Date formats seen in the field: `2025年09月26日`, `26-Sep-25`, `25.03.21`.
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

_USER_PROMPT = (
    "Read this box label: 型號 (model code), 製造日期 (manufacture date), and "
    "進貨日期 (the red acceptance stamp). Return null for anything you cannot read."
)


@dataclass
class Recognition:
    """What one recognition attempt produced.

    `model_code` is the 型號 the warehouse works in; `item_code` keeps the full
    supplier part number when the label carries one. Both feed the item matcher,
    which tries an exact match on the full code first and falls back to finding
    a 型號 inside the string.
    """

    model_code: str | None = None
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
            model_code: str | None = Field(description="型號 / model code such as T6284BA, or null")
            item_code: str | None = Field(description="Full supplier part number if present, or null")
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
            model_code=parsed.model_code,
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


class GeminiProvider:
    """Recognition via Google Gemini.

    Same contract as ClaudeProvider — the boundary is the point (M7
    architecture: the recognition service must not leak vendor SDK types, so
    swapping vendors changes only this class).

    Two knobs matter for this task specifically:

    media_resolution  the acceptance stamp is a small, low-contrast region of a
                      large photo. Downsampling is the difference between
                      reading it and inventing it, so this defaults to HIGH.
    thinking_level    a smudged stamp rewards deliberation. Worth A/B-ing, since
                      it also costs latency on a factory floor.
    """

    def __init__(
        self,
        # Flash by default: on the field photos it matches the pro model's
        # readings while taking under half the time (median 8.4s vs 19.1s).
        model: str = "gemini-3.7-flash",
        *,
        thinking: bool = True,
        media_resolution: str = "high",
        api_key: str | None = None,
    ) -> None:
        import os

        from google import genai
        from google.genai import types
        from pydantic import BaseModel, Field

        class LabelReading(BaseModel):
            model_code: str | None = Field(default=None, description="型號 / model code such as T6284BA, or null")
            item_code: str | None = Field(default=None, description="Full supplier part number if present, or null")
            manufacture_date: str | None = Field(default=None, description="Manufacture date in its original format, or null")
            receipt_date: str | None = Field(default=None, description="Receipt date from the red acceptance stamp, original format, or null")
            item_code_confidence: float = Field(default=0.0, ge=0.0, le=1.0)
            manufacture_date_confidence: float = Field(default=0.0, ge=0.0, le=1.0)
            receipt_date_confidence: float = Field(default=0.0, ge=0.0, le=1.0)
            stamp_visible: bool = Field(default=False, description="Whether a red acceptance stamp is visible at all")
            notes: str = Field(default="", description="What made this hard to read, if anything")

        key = api_key or os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY")
        if not key:
            raise RuntimeError("set GEMINI_API_KEY (or pass api_key=)")

        self._types = types
        self._client = genai.Client(api_key=key)
        self._schema = LabelReading
        self._model = model
        self._resolution = {
            "low": types.MediaResolution.MEDIA_RESOLUTION_LOW,
            "medium": types.MediaResolution.MEDIA_RESOLUTION_MEDIUM,
            "high": types.MediaResolution.MEDIA_RESOLUTION_HIGH,
        }[media_resolution]
        self._thinking = thinking

    def recognize(self, image_path: Path) -> Recognition:
        types = self._types
        media_type, _ = _encode(image_path)

        config = types.GenerateContentConfig(
            system_instruction=_SYSTEM_PROMPT,
            response_mime_type="application/json",
            response_schema=self._schema,
            media_resolution=self._resolution,
            temperature=0.0,
        )
        if self._thinking:
            config.thinking_config = types.ThinkingConfig(thinking_level="HIGH")

        try:
            response = self._client.models.generate_content(
                model=self._model,
                contents=[
                    types.Part.from_bytes(data=image_path.read_bytes(), mime_type=media_type),
                    _USER_PROMPT,
                ],
                config=config,
            )
        except Exception as exc:  # noqa: BLE001 — a provider outage must not stop the run
            return Recognition(error=f"{type(exc).__name__}: {exc}")

        parsed = response.parsed
        if parsed is None:
            return Recognition(error=f"unparseable_response: {(response.text or '')[:200]}")

        usage = getattr(response, "usage_metadata", None)
        return Recognition(
            model_code=parsed.model_code,
            item_code=parsed.item_code,
            manufacture_date=parsed.manufacture_date,
            receipt_date=parsed.receipt_date,
            item_code_confidence=parsed.item_code_confidence,
            manufacture_date_confidence=parsed.manufacture_date_confidence,
            receipt_date_confidence=parsed.receipt_date_confidence,
            stamp_visible=parsed.stamp_visible,
            notes=parsed.notes,
            usage={
                "input_tokens": getattr(usage, "prompt_token_count", 0) or 0,
                "output_tokens": getattr(usage, "candidates_token_count", 0) or 0,
            } if usage else {},
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
