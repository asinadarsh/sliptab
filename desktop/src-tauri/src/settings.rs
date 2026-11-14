//! User settings, stored as JSON next to the notes database.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Cards,
    Compact,
    Sticky,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    /// Which screen edge the bar sits on.
    pub side: Side,
    /// Where the bar sits along the edge: 0 = top, 1 = bottom.
    pub bar_position: f64,
    /// Panel width in logical pixels; None = a third of the screen.
    pub panel_width: Option<f64>,
    /// Off = the bar only opens on click or hotkey (the "pause" for accidental hovers).
    pub hover_open: bool,
    /// Off = no bar at all; the hotkey is the only way in.
    pub show_bar: bool,
    pub theme: Theme,
    pub layout: Layout,
    /// Main bar and accent colour, "#rrggbb"; None = neutral grey.
    pub bar_color: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: crate::hotkey::DEFAULT.to_owned(),
            side: Side::Right,
            bar_position: 0.5,
            panel_width: None,
            hover_open: true,
            show_bar: true,
            theme: Theme::System,
            layout: Layout::Cards,
            bar_color: None,
        }
    }
}

impl Settings {
    /// Clamp values a hand-edited or older file could get wrong.
    pub fn sanitized(mut self) -> Self {
        self.bar_position = if self.bar_position.is_finite() { self.bar_position.clamp(0.0, 1.0) } else { 0.5 };
        self.panel_width = self.panel_width.filter(|w| w.is_finite() && *w > 0.0);
        self.bar_color = self.bar_color.filter(|c| notes_core::valid_color(c));
        self
    }
}

const FILE: &str = "settings.json";

/// A missing or unreadable file gives defaults, so a bad edit can never stop the app starting.
pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Write to a temp file and rename, so a crash mid-write never leaves a half-written file.
pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_bad_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("edge-settings-{}", notes_core::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir).hotkey, crate::hotkey::DEFAULT);
        save(&dir, &Settings { hotkey: "Ctrl+Shift+K".into(), side: Side::Left, ..Default::default() }).unwrap();
        assert_eq!(load(&dir).side, Side::Left);
        assert_eq!(load(&dir).hotkey, "Ctrl+Shift+K");
        std::fs::write(dir.join(FILE), "{not json").unwrap();
        assert_eq!(load(&dir).hotkey, crate::hotkey::DEFAULT);
        // An old file with only some fields, and out-of-range values, still loads.
        std::fs::write(dir.join(FILE), r#"{"hotkey":"Alt+K","barPosition":7,"panelWidth":-3,"barColor":"red;x"}"#).unwrap();
        let s = load(&dir);
        assert_eq!((s.hotkey.as_str(), s.bar_position, s.panel_width, s.show_bar), ("Alt+K", 1.0, None, true));
        assert_eq!((s.bar_color, s.layout), (None, Layout::Cards));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
