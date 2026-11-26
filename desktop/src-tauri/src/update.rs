//! Automatic updates. Shortly after start and every 6 hours, ask the sync server for a newer
//! signed release; if there is one, download and install it in the background, then tell the
//! page ("update-ready") so it can offer Restart. Where the platform can't install in place
//! (a .deb on Linux), the page gets "update-available" with a download link instead.
//! When the server says this version is too old to sync, the check runs at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

pub const READY_EVENT: &str = "update-ready";
pub const AVAILABLE_EVENT: &str = "update-available";
const DOWNLOADS: &str = "https://edge-sync.forcesync.in/downloads/";

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
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
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
