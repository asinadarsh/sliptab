use uuid::Uuid;

use crate::link::Link;

/// One note. Timestamps are milliseconds since the Unix epoch, taken from the device clock.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Note {
    /// UUIDv7, created on the device so offline notes never collide.
    pub id: Uuid,
    /// Derived from the first line of `body`.
    pub title: String,
    /// Plain text. Checklist items are `- [ ] item` / `- [x] item` lines.
    pub body: String,
    pub pinned: bool,
    pub archived: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Device that made the last edit.
    pub device_id: String,
    /// Increments on every edit; used to detect conflicts during sync.
    pub version: i64,
    /// Set when this note is a conflict copy of another note.
    pub conflict_of: Option<Uuid>,
    /// Tab colour on the screen edge, "#rrggbb". Kept when unpinned so re-pinning keeps the colour.
    pub color: Option<String>,
    /// Set when this note is a saved post (a save). Absent in notes from before saves existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    /// Fields from a newer version of Edge that this version does not know. Kept and sent back
    /// unchanged, so an older app or server never strips what a newer one wrote.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Unknown fields from sync are kept, up to this many bytes of JSON per note and per link.
pub const MAX_EXTRA_BYTES: usize = 16 * 1024;

impl Note {
    /// Drops unknown fields that are too big to be anything Edge wrote (from an untrusted device).
    pub fn cap_extra(&mut self) {
        let too_big = |m: &serde_json::Map<String, serde_json::Value>| {
            !m.is_empty() && serde_json::to_string(m).map_or(true, |s| s.len() > MAX_EXTRA_BYTES)
        };
        if too_big(&self.extra) {
            self.extra.clear();
        }
        if let Some(l) = self.link.as_mut().filter(|l| too_big(&l.extra)) {
            l.extra.clear();
        }
    }
}

/// Accepts "#rrggbb" only, so a colour from sync or a hand-edited database can't inject CSS.
pub fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

const MAX_TITLE_CHARS: usize = 120;

/// Title = first non-blank line, without checklist or heading markers.
pub fn derive_title(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = ["- [ ] ", "- [x] ", "- [X] "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .unwrap_or(line)
        .trim_start_matches('#')
        .trim();
    crate::format::plain(line).trim().chars().take(MAX_TITLE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::derive_title;

    #[test]
    fn title_drops_formatting_markers() {
        assert_eq!(derive_title("**Launch** plan for __Q4__\nmore"), "Launch plan for Q4");
    }

    #[test]
    fn title_from_first_line() {
        assert_eq!(derive_title("Buy milk\nand eggs"), "Buy milk");
    }

    #[test]
    fn title_skips_blank_lines_and_whitespace() {
        assert_eq!(derive_title("\n   \n  Call mum  \nlater"), "Call mum");
    }

    #[test]
    fn title_of_empty_body_is_empty() {
        assert_eq!(derive_title(""), "");
        assert_eq!(derive_title("  \n\t\n"), "");
    }

    #[test]
    fn title_strips_checklist_and_heading_markers() {
        assert_eq!(derive_title("- [ ] milk\n- [x] bread"), "milk");
        assert_eq!(derive_title("- [x] done thing"), "done thing");
        assert_eq!(derive_title("## Trip ideas"), "Trip ideas");
    }

    #[test]
    fn colors_must_be_plain_hex() {
        assert!(super::valid_color("#f5a524"));
        assert!(super::valid_color("#ABCDEF"));
        for bad in ["f5a524", "#f5a52", "#f5a5245", "#zzzzzz", "red", "#fff;}x{"] {
            assert!(!super::valid_color(bad), "{bad}");
        }
    }

    #[test]
    fn title_is_capped() {
        let long = "a".repeat(500);
        assert_eq!(derive_title(&long).chars().count(), 120);
    }
}
