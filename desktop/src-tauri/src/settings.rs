//! User settings, stored as JSON next to the notes database.

use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: crate::hotkey::DEFAULT.to_owned() }
    }
}

const FILE: &str = "settings.json";

/// A missing or unreadable file gives defaults, so a bad edit can never stop the app starting.
pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
        save(&dir, &Settings { hotkey: "Ctrl+Shift+K".into() }).unwrap();
        assert_eq!(load(&dir).hotkey, "Ctrl+Shift+K");
        std::fs::write(dir.join(FILE), "{not json").unwrap();
        assert_eq!(load(&dir).hotkey, crate::hotkey::DEFAULT);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
