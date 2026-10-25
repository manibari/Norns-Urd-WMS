//! Image -> label fields. Stateless, and deliberately swappable.
//!
//! The provider boundary exists because this is the one part of the system
//! that changes whenever a vendor or model iterates (M7 architecture). Nothing
//! outside this module knows a vendor's wire format.
//!
//! The prompt's single most important job is to make the model say "I could
//! not read it" instead of inventing a plausible date. A hallucinated date
//! that lands on a real in-stock lot is the one failure this whole design is
//! built to prevent, and it is invisible downstream — it posts cleanly and
//! logs nothing.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::camera::media_type;

const SYSTEM_PROMPT: &str = "\
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
   Report the 型號 in `model_code`, and the full string in `item_code` when one
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
";

const USER_PROMPT: &str = "Read this box label: 型號 (model code), 製造日期 (manufacture date), and \
進貨日期 (the red acceptance stamp). Return null for anything you cannot read.";

/// What one recognition attempt produced.
///
/// `model_code` is the 型號 the warehouse works in; `item_code` keeps the full
/// supplier part number when the label carries one. Both feed the item
/// matcher.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Recognition {
    pub model_code: Option<String>,
    pub item_code: Option<String>,
    pub manufacture_date: Option<String>,
    pub receipt_date: Option<String>,
    pub item_code_confidence: f64,
    pub manufacture_date_confidence: f64,
    pub receipt_date_confidence: f64,
    pub stamp_visible: bool,
    pub notes: String,
    #[serde(skip)]
    pub error: Option<String>,
    #[serde(skip)]
    pub input_tokens: u64,
    #[serde(skip)]
    pub output_tokens: u64,
}

impl Recognition {
    pub fn failed(message: impl Into<String>) -> Self {
        Recognition { error: Some(message.into()), ..Default::default() }
    }

    pub fn is_failed(&self) -> bool {
        self.error.is_some()
    }

    /// Models sometimes answer "null" or "" for a field they could not read.
    /// Both mean the same thing as an absent value.
    fn tidy(mut self) -> Self {
        for field in [&mut self.model_code, &mut self.item_code, &mut self.manufacture_date, &mut self.receipt_date] {
            if field.as_deref().is_some_and(|v| {
                let v = v.trim();
                v.is_empty() || v.eq_ignore_ascii_case("null") || v.eq_ignore_ascii_case("none")
            }) {
                *field = None;
            }
        }
        self
    }
}

pub trait RecognitionProvider: Send + Sync {
    fn recognize(&self, image: &[u8]) -> Recognition;
}

/// Recognition via the Google Gemini REST API.
///
/// Two knobs matter for this task specifically:
///
/// * media resolution — the acceptance stamp is a small, low-contrast region
///   of a large photo. Downsampling is the difference between reading it and
///   inventing it, so this asks for HIGH.
/// * thinking level — a smudged stamp rewards deliberation.
pub struct GeminiProvider {
    agent: ureq::Agent,
    api_key: String,
    model: String,
    thinking: bool,
}

pub const DEFAULT_MODEL: &str = "gemini-3.7-flash";
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

impl GeminiProvider {
    /// Built once and reused: a fresh client per photo meant a new TLS
    /// handshake every time, and most of the wait was that, not the model
    /// (~7s warm against ~23s cold, measured).
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .build()
            .into();
        GeminiProvider { agent, api_key: api_key.into(), model: model.into(), thinking: true }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn schema() -> Value {
        let text = |desc: &str| json!({"type": "STRING", "nullable": true, "description": desc});
        let score = json!({"type": "NUMBER", "minimum": 0.0, "maximum": 1.0});
        json!({
            "type": "OBJECT",
            "properties": {
                "model_code": text("型號 / model code such as T6284BA, or null"),
                "item_code": text("Full supplier part number if present, or null"),
                "manufacture_date": text("Manufacture date in its original format, or null"),
                "receipt_date": text("Receipt date from the red acceptance stamp, original format, or null"),
                "item_code_confidence": score,
                "manufacture_date_confidence": score,
                "receipt_date_confidence": score,
                "stamp_visible": {"type": "BOOLEAN", "description": "Whether a red acceptance stamp is visible at all"},
                "notes": {"type": "STRING", "description": "What made this hard to read, if anything"},
            },
            "required": ["item_code_confidence", "manufacture_date_confidence", "receipt_date_confidence", "stamp_visible", "notes"],
        })
    }

    pub fn request_body(&self, image: &[u8]) -> Value {
        let data = base64::engine::general_purpose::STANDARD.encode(image);
        let mut generation = json!({
            "temperature": 0.0,
            "responseMimeType": "application/json",
            "responseSchema": Self::schema(),
            "mediaResolution": "MEDIA_RESOLUTION_HIGH",
        });
        if self.thinking {
            generation["thinkingConfig"] = json!({"thinkingLevel": "HIGH"});
        }
        json!({
            "systemInstruction": {"parts": [{"text": SYSTEM_PROMPT}]},
            "contents": [{
                "role": "user",
                "parts": [
                    {"inlineData": {"mimeType": media_type(image), "data": data}},
                    {"text": USER_PROMPT},
                ],
            }],
            "generationConfig": generation,
        })
    }
}

/// Pull the JSON answer out of a generateContent response.
pub fn parse_response(body: &Value) -> Recognition {
    if let Some(message) = body.pointer("/error/message").and_then(Value::as_str) {
        return Recognition::failed(format!("Gemini: {message}"));
    }
    let parts = body.pointer("/candidates/0/content/parts").and_then(Value::as_array);
    let text: String = parts
        .into_iter()
        .flatten()
        // Thought summaries are not the answer.
        .filter(|p| !p.get("thought").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect();
    if text.trim().is_empty() {
        let reason = body.pointer("/candidates/0/finishReason").and_then(Value::as_str).unwrap_or("no text");
        return Recognition::failed(format!("unparseable_response: {reason}"));
    }
    let mut reading = match serde_json::from_str::<Recognition>(text.trim()) {
        Ok(r) => r.tidy(),
        Err(_) => {
            let head: String = text.chars().take(200).collect();
            return Recognition::failed(format!("unparseable_response: {head}"));
        }
    };
    reading.input_tokens = body.pointer("/usageMetadata/promptTokenCount").and_then(Value::as_u64).unwrap_or(0);
    reading.output_tokens = body.pointer("/usageMetadata/candidatesTokenCount").and_then(Value::as_u64).unwrap_or(0);
    reading
}

impl RecognitionProvider for GeminiProvider {
    fn recognize(&self, image: &[u8]) -> Recognition {
        let url = format!("{ENDPOINT}/{}:generateContent", self.model);
        let result = self.agent.post(&url).header("x-goog-api-key", &self.api_key).send_json(self.request_body(image));
        // A provider outage must not stop the flow: the error travels with the
        // result and the screen drops into manual selection.
        let mut response = match result {
            Ok(r) => r,
            Err(e) => return Recognition::failed(format!("連不到辨識服務 — {e}")),
        };
        let status = response.status().as_u16();
        let body: Value = match response.body_mut().with_config().limit(8 * 1024 * 1024).read_json() {
            Ok(v) => v,
            Err(e) => return Recognition::failed(format!("辨識服務回應 HTTP {status}，無法解讀 — {e}")),
        };
        parse_response(&body)
    }
}

/// Stands in when no API key is configured: every photo defers to manual
/// selection with the reason on screen, rather than the flow breaking.
pub struct Unconfigured;

impl RecognitionProvider for Unconfigured {
    fn recognize(&self, _image: &[u8]) -> Recognition {
        Recognition::failed("尚未設定辨識 API 金鑰（系統設定 → 影像設定），請手動選批次")
    }
}

/// Returns a fixed reading. For demonstrations and screenshots, where the
/// point is the screen after recognition, not a call to a vendor.
pub struct Scripted(pub Recognition);

impl RecognitionProvider for Scripted {
    fn recognize(&self, _image: &[u8]) -> Recognition {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_answer_and_skips_thoughts() {
        let body = json!({
            "candidates": [{"content": {"parts": [
                {"text": "thinking about it", "thought": true},
                {"text": "{\"model_code\":\"T7320BC\",\"item_code\":null,\"manufacture_date\":\"2025年09月26日\",\"receipt_date\":\"null\",\"item_code_confidence\":0.9,\"manufacture_date_confidence\":0.95,\"receipt_date_confidence\":0.0,\"stamp_visible\":false,\"notes\":\"\"}"}
            ]}}],
            "usageMetadata": {"promptTokenCount": 1200, "candidatesTokenCount": 80}
        });
        let r = parse_response(&body);
        assert!(r.error.is_none(), "{r:?}");
        assert_eq!(r.model_code.as_deref(), Some("T7320BC"));
        assert_eq!(r.manufacture_date.as_deref(), Some("2025年09月26日"));
        assert_eq!(r.receipt_date, None, "a literal \"null\" is no reading");
        assert_eq!(r.input_tokens, 1200);
    }

    #[test]
    fn an_api_error_is_carried_not_thrown() {
        let r = parse_response(&json!({"error": {"code": 400, "message": "API key not valid"}}));
        assert!(r.error.unwrap().contains("API key not valid"));
    }

    #[test]
    fn garbage_is_reported_with_its_head() {
        let r = parse_response(&json!({"candidates": [{"content": {"parts": [{"text": "sorry, no"}]}}]}));
        assert!(r.error.unwrap().contains("sorry"));
    }

    #[test]
    fn request_carries_the_image_and_the_schema() {
        let p = GeminiProvider::new("k", DEFAULT_MODEL);
        let body = p.request_body(b"\xff\xd8\xff\xd9");
        assert_eq!(body.pointer("/contents/0/parts/0/inlineData/mimeType").unwrap(), "image/jpeg");
        assert_eq!(body.pointer("/generationConfig/responseMimeType").unwrap(), "application/json");
        assert!(body.pointer("/generationConfig/responseSchema/properties/model_code").is_some());
    }
}
