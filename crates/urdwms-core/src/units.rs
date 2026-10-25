//! Metres <-> boxes.
//!
//! Suppliers label film by length (`LENGTH (m) 900`) and deliveries are
//! sometimes counted the same way, but stock is kept in boxes: one box is one
//! roll, a draw deducts one, and there is no partial consumption (requirement
//! Q6). Metres are an input aid and a display, never the ledger unit — the
//! moment stock is kept in metres, "half a roll left" becomes representable
//! and the no-partial-deduction decision unravels.
//!
//! Which makes the remainder the interesting part. 9,500 m of 900 m rolls is
//! not 10.5 boxes; it is 10 boxes and 500 m unaccounted for, and that
//! discrepancy is worth showing the person receiving the delivery rather than
//! rounding away.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxConversion {
    pub boxes: i64,
    pub remainder: i64,
    pub per_box: i64,
    pub entered: i64,
}

impl BoxConversion {
    pub fn exact(&self) -> bool {
        self.remainder == 0
    }

    /// No unit named: an item is counted in whatever it is counted in —
    /// metres of film, sheets of foil, packs of desiccant.
    pub fn note(&self) -> Option<String> {
        if self.exact() {
            return None;
        }
        Some(format!(
            "{} ÷ {}/箱 = {} 箱，剩 {} 對不上整箱",
            thousands(self.entered),
            thousands(self.per_box),
            self.boxes,
            thousands(self.remainder)
        ))
    }
}

/// Convert a count to whole boxes, keeping the remainder visible.
///
/// Returns `None` when the item has no conversion configured — the caller
/// must then ask for boxes directly rather than guess a rate.
pub fn boxes_from_meters(entered: i64, per_box: Option<i64>) -> Option<BoxConversion> {
    let per_box = per_box.filter(|&p| p > 0)?;
    if entered <= 0 {
        return Some(BoxConversion { boxes: 0, remainder: 0, per_box, entered });
    }
    Some(BoxConversion { boxes: entered / per_box, remainder: entered % per_box, per_box, entered })
}

/// Display helper: how many metres a box count represents.
pub fn meters_from_boxes(boxes: i64, per_box: Option<i64>) -> Option<i64> {
    per_box.filter(|&p| p > 0).map(|p| boxes * p)
}

/// `9500` -> `9,500`.
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_multiple() {
        let r = boxes_from_meters(9000, Some(900)).unwrap();
        assert_eq!(r.boxes, 10);
        assert!(r.exact());
        assert!(r.note().is_none());
    }

    #[test]
    fn remainder_is_reported_not_rounded() {
        let r = boxes_from_meters(9500, Some(900)).unwrap();
        assert_eq!((r.boxes, r.remainder), (10, 500));
        let note = r.note().unwrap();
        assert!(note.contains("500"));
        assert!(note.contains("9,500"));
        assert!(!note.contains('米'));
    }

    #[test]
    fn less_than_one_box() {
        let r = boxes_from_meters(400, Some(900)).unwrap();
        assert_eq!((r.boxes, r.remainder), (0, 400));
    }

    #[test]
    fn unconfigured_item_refuses_to_guess() {
        for rate in [None, Some(0), Some(-1)] {
            assert!(boxes_from_meters(9000, rate).is_none());
        }
    }

    #[test]
    fn zero_metres() {
        let r = boxes_from_meters(0, Some(900)).unwrap();
        assert_eq!((r.boxes, r.remainder), (0, 0));
    }

    #[test]
    fn display_direction() {
        assert_eq!(meters_from_boxes(3, Some(900)), Some(2700));
        assert_eq!(meters_from_boxes(3, None), None);
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(10800), "10,800");
        assert_eq!(thousands(-1234567), "-1,234,567");
    }
}
