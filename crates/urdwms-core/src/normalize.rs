//! Normalisation of dates and item codes read off packaging labels.
//!
//! Deliberately dependency-light and pure: every function here is also needed
//! by the manual-entry path (requirement US-4), so it is production code, not
//! PoC glue.
//!
//! The central idea (requirement section 2.2): recognition does not have to
//! read the receipt date *correctly*. It has to produce something close enough
//! to pick the right lot out of a small set of lots already known to be in
//! stock. So the normaliser never throws away an unparseable value — it
//! degrades it into a comparable key and lets the matcher decide.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

/// Characters an OCR pass confuses for one another. Substituting within a
/// group is charged half price by [`confusion_distance`], so `2026-08-l2`
/// stays close to `2026-08-12` while `2026-08-92` does not.
const CONFUSION_GROUPS: &[&str] = &["0OoDQ", "1lI|i7", "2Zz", "5Ss", "6bG", "8B", "9gq", "4A", "3E"];

fn confusable(a: char, b: char) -> bool {
    CONFUSION_GROUPS.iter().any(|group| group.contains(a) && group.contains(b))
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `2025年09月26日`
static CJK_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{2,4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日?").unwrap());

/// `26-Sep-25`, `26 Sep 2025`
static DMY_ALPHA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})[\s\-/]*([A-Za-z]{3,9})[\s\-/]*(\d{2,4})").unwrap());

/// `2026-08-12`, `2026/08/12`, and OCR-damaged variants such as `2026-08-l2`.
///
/// Month and day accept any alphanumeric, not a confusable-only whitelist, and
/// the match may not stop mid-token (checked by [`find_whole_token`], since the
/// regex engine has no lookahead). Both guard the same trap: a narrow class
/// lets the pattern match a PREFIX of a damaged field and silently drop the
/// rest, so `2026-08-1X` parses as a clean `2026-08-01`. That turns "could not
/// read it" into "read it confidently, wrong" — and if 08-01 happens to be in
/// stock, into a false hit. Unreadable must stay visibly unreadable.
static YMD_LOOSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9OoDQlI|iZzSsbGBgq]{4})[\s\-/.]([0-9A-Za-z|]{1,2})[\s\-/.]([0-9A-Za-z|]{1,2})").unwrap()
});

/// `22.03.2026`, `22/03/2026` — day first, four-digit year last.
///
/// This is what Sealed Air prints in the `Date:` field. The year-first pattern
/// needs four digits up front, so without this `22.03.2026` fell through to
/// the bare-digits fallback and became `22032026` — DDMMYYYY in YYYYMMDD
/// positions — and every box from this supplier deferred to manual selection.
///
/// Day-first is only safe to assume silently when the first field cannot be a
/// month. `22.03.2026` is unambiguous; `03.04.2026` is not, so the ambiguous
/// case still yields a key (built day-first, since that is this label's
/// format) but no `iso`: it can be matched against the closed candidate set,
/// and cannot be stored as a fact.
static DMY_NUMERIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2})[\-/.](\d{1,2})[\-/.](\d{4})").unwrap());

static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s\-/.年月日]+").unwrap());

/// A date reduced to eight comparable characters.
///
/// `key` is always in YYYYMMDD positions. It may contain non-digits when OCR
/// damaged the value — that is the point, since [`confusion_distance`] can
/// still score it against a real lot date.
///
/// `iso` is only set when the value parsed cleanly. Code that needs a real
/// date (storing a lot, sorting by receipt date) must use `iso` and treat
/// `None` as "not readable"; code that only compares against candidates uses
/// `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateKey {
    pub key: String,
    pub iso: Option<String>,
    pub raw: String,
}

impl DateKey {
    pub fn is_clean(&self) -> bool {
        self.iso.is_some()
    }
}

/// `25` -> `2025`. Two-digit years on these labels are always this century.
fn expand_year(value: &str) -> String {
    match value.chars().count() {
        4 => value.to_string(),
        2 => format!("20{value}"),
        _ => format!("{value:0>4}"),
    }
}

fn zfill2(value: &str) -> String {
    format!("{value:0>2}")
}

fn iso_if_valid(year: &str, month: &str, day: &str) -> Option<String> {
    let all_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !(all_digits(year) && all_digits(month) && all_digits(day)) {
        return None;
    }
    let (y, m, d): (i32, u32, u32) = (year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    if !(1900..=2999).contains(&y) {
        return None;
    }
    // A calendar check, not just ranges: 2026-02-31 is a misread, and storing
    // it as a fact would break every later date comparison on that lot.
    NaiveDate::from_ymd_opt(y, m, d).map(|date| date.format("%Y-%m-%d").to_string())
}

/// Leftmost match of `re` that does not stop mid-token — what a trailing
/// negative lookahead would do, retried from each later start position the
/// way a backtracking engine would.
fn find_whole_token<'t>(re: &Regex, text: &'t str, continues: impl Fn(char) -> bool) -> Option<regex::Captures<'t>> {
    let mut start = 0;
    while start <= text.len() {
        let caps = re.captures_at(text, start)?;
        let whole = caps.get(0).unwrap();
        match text[whole.end()..].chars().next() {
            Some(next) if continues(next) => {
                // Advance one character past where this attempt began.
                let step = text[whole.start()..].chars().next().map_or(1, char::len_utf8);
                start = whole.start() + step;
            }
            _ => return Some(caps),
        }
    }
    None
}

/// Parse any of the label formats — plus OCR-damaged variants — into a key.
///
/// Returns `None` only when nothing date-shaped is present at all. A value
/// that looks like a date but does not parse still yields a [`DateKey`] with
/// `iso = None`, because it remains useful for candidate matching.
pub fn to_date_key(raw: Option<&str>) -> Option<DateKey> {
    let text = raw?.trim();
    if text.is_empty() {
        return None;
    }
    let make = |key: String, iso: Option<String>| Some(DateKey { key, iso, raw: text.to_string() });

    if let Some(c) = CJK_DATE.captures(text) {
        let year = expand_year(&c[1]);
        let (month, day) = (zfill2(&c[2]), zfill2(&c[3]));
        let iso = iso_if_valid(&year, &month, &day);
        return make(format!("{year}{month}{day}"), iso);
    }

    if let Some(c) = DMY_ALPHA.captures(text) {
        let abbrev: String = c[2].chars().take(3).collect::<String>().to_lowercase();
        if let Some(index) = MONTHS.iter().position(|m| *m == abbrev) {
            let day = zfill2(&c[1]);
            let year = expand_year(&c[3]);
            let month = format!("{:02}", index + 1);
            let iso = iso_if_valid(&year, &month, &day);
            return make(format!("{year}{month}{day}"), iso);
        }
    }

    if let Some(c) = find_whole_token(&DMY_NUMERIC, text, |ch| ch.is_ascii_digit()) {
        let (day, month, year) = (zfill2(&c[1]), zfill2(&c[2]), c[3].to_string());
        // Only claim a real date when the first field cannot be a month.
        let unambiguous = c[1].parse::<u32>().is_ok_and(|first| first > 12);
        let iso = if unambiguous { iso_if_valid(&year, &month, &day) } else { None };
        return make(format!("{year}{month}{day}"), iso);
    }

    if let Some(c) = find_whole_token(&YMD_LOOSE, text, |ch| ch.is_ascii_alphanumeric() || ch == '|') {
        let (year, month, day) = (c[1].to_string(), zfill2(&c[2]), zfill2(&c[3]));
        let iso = iso_if_valid(&year, &month, &day);
        return make(format!("{year}{month}{day}"), iso);
    }

    // Last resort: strip separators and hope for a bare YYYYMMDD / YYMMDD.
    let stripped = SEPARATORS.replace_all(text, "").to_string();
    let chars: Vec<char> = stripped.chars().collect();
    let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    match chars.len() {
        8 => {
            let iso = iso_if_valid(&part(0, 4), &part(4, 6), &part(6, 8));
            make(stripped, iso)
        }
        6 => {
            let (year, month, day) = (expand_year(&part(0, 2)), part(2, 4), part(4, 6));
            let iso = iso_if_valid(&year, &month, &day);
            make(format!("{year}{month}{day}"), iso)
        }
        _ => None,
    }
}

/// Convenience for the common case of an owned optional string.
pub fn date_key(raw: &Option<String>) -> Option<DateKey> {
    to_date_key(raw.as_deref())
}

/// Levenshtein distance where OCR-confusable substitutions cost half.
///
/// Both inputs are short (8 characters), so the plain DP is fine.
pub fn confusion_distance(a: &str, b: &str) -> f64 {
    if a == b {
        return 0.0;
    }
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<f64> = (0..=b.len()).map(|i| i as f64).collect();
    for (i, ch_a) in a.chars().enumerate() {
        let mut current = vec![(i + 1) as f64];
        for (j, &ch_b) in b.iter().enumerate() {
            let sub = if ch_a == ch_b {
                0.0
            } else if confusable(ch_a, ch_b) {
                0.5
            } else {
                1.0
            };
            let value = (previous[j + 1] + 1.0).min(current[j] + 1.0).min(previous[j] + sub);
            current.push(value);
        }
        previous = current;
    }
    *previous.last().unwrap()
}

/// Resolve a label or form abbreviation to a canonical item code.
///
/// `alias_map` is per-factory configuration (requirement US-11), e.g.
/// `{"T7320-P1": "2003.T7320BC-340X900-P1"}`. Lookup order: exact canonical,
/// configured alias, then a containment fallback so an unconfigured
/// abbreviation still has a chance of resolving.
pub fn normalize_item_code(raw: Option<&str>, alias_map: &HashMap<String, String>) -> Option<String> {
    let text: String = raw?.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
    if text.is_empty() {
        return None;
    }
    let aliases: HashMap<String, &String> = alias_map.iter().map(|(k, v)| (k.to_uppercase(), v)).collect();
    if alias_map.values().any(|v| *v == text) {
        return Some(text);
    }
    if let Some(found) = aliases.get(&text) {
        return Some((*found).clone());
    }
    let strip = |s: &str| s.replace(['-', '.'], "");
    let compact = strip(&text);
    for code in alias_map.values() {
        if !compact.is_empty() && strip(code).contains(&compact) {
            return Some(code.clone());
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(raw: &str) -> DateKey {
        to_date_key(Some(raw)).unwrap()
    }

    #[test]
    fn all_four_field_formats_reach_the_same_key() {
        for raw in ["2025年09月26日", "26-Sep-25", "2025-09-26", "20250926"] {
            let parsed = key(raw);
            assert_eq!(parsed.key, "20250926", "{raw}");
            assert_eq!(parsed.iso.as_deref(), Some("2025-09-26"), "{raw}");
        }
    }

    #[test]
    fn stamp_format() {
        assert_eq!(key("2026-08-12").iso.as_deref(), Some("2026-08-12"));
    }

    #[test]
    fn ocr_damage_degrades_instead_of_vanishing() {
        let parsed = key("2026-08-l2");
        assert_eq!(parsed.key, "202608l2");
        assert!(parsed.iso.is_none());
        assert!(!parsed.is_clean());
    }

    #[test]
    fn a_partly_unreadable_field_never_looks_clean() {
        let parsed = key("2026-08-1X");
        assert!(parsed.iso.is_none(), "a damaged field must not produce a clean date");
        assert_eq!(parsed.key, "2026081X");
    }

    #[test]
    fn impossible_dates_are_not_clean() {
        assert!(key("2026-19-45").iso.is_none());
        assert!(key("2026-02-31").iso.is_none());
    }

    #[test]
    fn nothing_date_shaped() {
        for raw in [Some(""), Some("   "), None, Some("高阻氧食品包裝拉伸膜")] {
            assert!(to_date_key(raw).is_none(), "{raw:?}");
        }
    }

    #[test]
    fn day_first_with_four_digit_year() {
        let parsed = key("22.03.2026");
        assert_eq!(parsed.key, "20260322");
        assert_eq!(parsed.iso.as_deref(), Some("2026-03-22"));
    }

    #[test]
    fn ambiguous_day_first_keys_but_is_not_a_fact() {
        let parsed = key("03.04.2026");
        assert_eq!(parsed.key, "20260403");
        assert!(parsed.iso.is_none());
    }

    #[test]
    fn a_trailing_digit_is_not_dropped() {
        // `2026-08-123` must not quietly become 2026-08-12.
        let parsed = to_date_key(Some("2026-08-123"));
        assert!(parsed.as_ref().and_then(|p| p.iso.clone()).is_none(), "{parsed:?}");
    }

    #[test]
    fn a_trailing_cjk_unit_is_a_boundary() {
        assert_eq!(key("2026-08-12日").iso.as_deref(), Some("2026-08-12"));
    }

    #[test]
    fn ocr_confusable_substitution_is_half_price() {
        assert_eq!(confusion_distance("202608l2", "20260812"), 0.5);
        assert_eq!(confusion_distance("2O26O812", "20260812"), 1.0);
    }

    #[test]
    fn a_genuinely_different_digit_costs_full() {
        assert_eq!(confusion_distance("20260892", "20260812"), 1.0);
    }

    #[test]
    fn a_different_month_is_far() {
        assert!(confusion_distance("20260112", "20260812") >= 1.0);
    }

    #[test]
    fn identical() {
        assert_eq!(confusion_distance("20260812", "20260812"), 0.0);
    }

    fn aliases() -> HashMap<String, String> {
        HashMap::from([("T7320-P1".to_string(), "2003.T7320BC-340X900-P1".to_string())])
    }

    #[test]
    fn form_abbreviation_resolves_to_canonical() {
        assert_eq!(normalize_item_code(Some("T7320-P1"), &aliases()).as_deref(), Some("2003.T7320BC-340X900-P1"));
    }

    #[test]
    fn canonical_passes_through() {
        assert_eq!(
            normalize_item_code(Some("2003.T7320BC-340X900-P1"), &aliases()).as_deref(),
            Some("2003.T7320BC-340X900-P1")
        );
    }

    #[test]
    fn whitespace_and_case() {
        assert_eq!(normalize_item_code(Some(" t7320-p1 "), &aliases()).as_deref(), Some("2003.T7320BC-340X900-P1"));
    }

    #[test]
    fn unknown_code_is_returned_not_dropped() {
        assert_eq!(
            normalize_item_code(Some("2003.T6240BA-334X600"), &aliases()).as_deref(),
            Some("2003.T6240BA-334X600")
        );
    }
}
