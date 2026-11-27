//! Automatic updates. Shortly after start and every 6 hours, ask the sync server for a newer
//! signed release; if there is one, download and install it in the background, then tell the
//! page ("update-ready") so it can offer Restart. Where the platform can't install in place
//! (a .deb on Linux), the page gets "update-available" with a download link instead.
//! When the server says this version is too old to sync, the check runs at once.
//!
//! The check also lets the server count copies in use, with no ID: it carries flags for the
//! periods this install hasn't been counted in yet (first check today, this week, this month,
//! ever). A period is marked counted only after the server answered, in `counted.json`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

pub const READY_EVENT: &str = "update-ready";
pub const AVAILABLE_EVENT: &str = "update-available";
const DOWNLOADS: &str = "https://edge-sync.forcesync.in/downloads/";
const MANIFEST: &str = "https://edge-sync.forcesync.in/v1/app/latest.json";

/// The periods (UTC) this install was last counted in.
#[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Debug, Clone, Copy)]
struct Counted {
    day: i64,
    week: i64,
    month: i64,
    ever: bool,
}

/// (day, ISO week, month) numbers for a Unix time, all UTC. Weeks start on Monday, like
/// Postgres date_trunc('week'); 1970-01-01 was a Thursday.
fn periods(unix_secs: i64) -> (i64, i64, i64) {
    let day = unix_secs.div_euclid(86_400);
    let week = (day + 3).div_euclid(7);
    // Days to civil date (Howard Hinnant's algorithm), for the month.
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (day, week, y * 12 + m - 1)
}

/// The query flags for now, and what to save once the server answered.
fn flags(saved: Counted, now: i64) -> (String, Counted) {
    let (day, week, month) = periods(now);
    let f = |b: bool| if b { "1" } else { "0" };
    let q = format!(
        "d={}&w={}&m={}&n={}",
        f(saved.day != day),
        f(saved.week != week),
        f(saved.month != month),
        f(!saved.ever)
    );
    (q, Counted { day, week, month, ever: true })
}

fn counted_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    Some(app.path().app_data_dir().ok()?.join("counted.json"))
}

static CHECKING: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await; // let start-up and the first sync finish
        loop {
            check(&app).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

/// Checks now (e.g. after the server refused this version). Does nothing if a check is running
/// or an update is already installed and waiting for a restart.
pub fn check_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check(&app).await });
}

async fn check(app: &AppHandle) {
    // Microsoft Store builds are updated by the Store (its policy forbids self-updating).
    if option_env!("SLIPTAB_STORE").is_some() {
        return;
    }
    // The user turned update checks off in Settings: no request leaves the app.
    use tauri::Manager;
    if let Some(s) = app.try_state::<crate::AppSettings>() {
        if s.current.lock().map(|c| !c.check_updates).unwrap_or(false) {
            return;
        }
    }
    if INSTALLED.load(Ordering::Relaxed) || CHECKING.swap(true, Ordering::Relaxed) {
        return;
    }
    let result = install(app).await;
    CHECKING.store(false, Ordering::Relaxed);
    if let Err(e) = result {
        eprintln!("update: {e}");
    }
}

async fn install(app: &AppHandle) -> Result<(), String> {
    let path = counted_path(app);
    let saved: Counted = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (q, next) = flags(saved, now);
    // {{target}} is windows/linux/darwin, {{bundle_type}} nsis/msi/deb/appimage/rpm.
    let url = format!("{MANIFEST}?p={{{{target}}}}&v={{{{current_version}}}}&c={{{{bundle_type}}}}&{q}");
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{e}"))?])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let checked = updater.check().await.map_err(|e| e.to_string())?;
    // The server answered: this install is counted for these periods.
    if let (Some(p), true) = (&path, next != saved) {
        let _ = std::fs::write(p, serde_json::to_vec(&next).unwrap_or_default());
    }
    let Some(update) = checked else {
        return Ok(());
    };
    let version = update.version.clone();
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            INSTALLED.store(true, Ordering::Relaxed);
            let _ = app.emit(READY_EVENT, &version);
            Ok(())
        }
        Err(e) => {
            // Can't replace this install in place (Linux .deb): point at the download instead.
            let _ = app.emit(
                AVAILABLE_EVENT,
                serde_json::json!({ "version": version, "url": DOWNLOADS }),
            );
            Err(e.to_string())
        }
    }
}

/// Restarts into the installed update.
#[tauri::command]
pub fn restart_for_update(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_match_the_calendar() {
        // 2026-11-27 12:00 UTC is a Friday; its ISO week began Monday 2026-11-23.
        let fri = 1_795_780_800;
        let mon = 1_795_392_000; // 2026-11-23 00:00 UTC
        assert_eq!(periods(fri).1, periods(mon).1);
        assert_ne!(periods(mon - 1).1, periods(mon).1, "Sunday is the week before");
        assert_eq!(periods(fri).2, 2026 * 12 + 10, "November is month index 10");
        assert_eq!(periods(1_798_761_600).2, 2027 * 12, "2027-01-01 is January");
    }

    #[test]
    fn each_period_is_flagged_once() {
        let now = 1_795_780_800;
        let (q, saved) = flags(Counted::default(), now);
        assert_eq!(q, "d=1&w=1&m=1&n=1");
        assert_eq!(flags(saved, now + 3600).0, "d=0&w=0&m=0&n=0");
        assert_eq!(flags(saved, now + 86_400).0, "d=1&w=0&m=0&n=0");
        assert_eq!(flags(saved, now + 4 * 86_400).0, "d=1&w=1&m=1&n=0", "Tue 1 Dec: new week and month");
    }
}
