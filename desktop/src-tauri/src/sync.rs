//! Sync for the desktop app. One background thread syncs every 60 s, 2 s after a local edit
//! and when the panel opens; the network call never holds the note store's lock. Before each
//! sync it fetches details for new saves.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use notes_core::client::{Client, ClientError};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{CmdResult, Notes};

const SERVER: &str = "https://edge-sync.forcesync.in";
pub const STATUS_EVENT: &str = "sync-status";
/// Saves got their title, thumbnail or folder: the page reloads its lists.
pub const SAVES_EVENT: &str = "saves-updated";

#[derive(Serialize, Deserialize)]
struct Account {
    email: String,
    token: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    email: Option<String>,
    last_synced_at: Option<i64>,
    error: Option<String>,
    /// Notes that arrived from other devices in the last sync.
    received: usize,
}

pub struct SyncState {
    dir: PathBuf,
    kick: Mutex<Sender<bool>>,
    status: Mutex<Status>,
}

impl SyncState {
    /// Ask for a sync: `now` skips the 2 s wait that lets typing settle.
    pub fn kick(&self, now: bool) {
        if let Ok(k) = self.kick.lock() {
            let _ = k.send(now);
        }
    }

    fn account_path(&self) -> PathBuf {
        self.dir.join("account.json")
    }

    fn account(&self) -> Option<Account> {
        serde_json::from_str(&std::fs::read_to_string(self.account_path()).ok()?).ok()
    }
}

pub fn start(app: &AppHandle, dir: &Path) {
    let (tx, rx) = channel::<bool>();
    let state = SyncState { dir: dir.to_owned(), kick: Mutex::new(tx), status: Mutex::new(Status::default()) };
    state.status.lock().unwrap().email = state.account().map(|a| a.email);
    app.manage(state);
    app.state::<SyncState>().kick(true); // first sync right after start-up
    let app = app.clone();
    std::thread::spawn(move || loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(now) => {
                if !now {
                    std::thread::sleep(Duration::from_secs(2));
                }
                while rx.try_recv().is_ok() {} // one sync covers every request so far
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        run(&app);
    });
}

fn run(app: &AppHandle) {
    let st = app.state::<SyncState>();
    let notes = app.state::<Notes>();
    // New saves first, so they go up with their details in this same sync. Works signed out too.
    if notes_core::client::fill_details(&notes.0, 20) > 0 {
        let _ = app.emit(SAVES_EVENT, ());
    }
    let Some(account) = st.account() else { return };
    let result = Client::new(SERVER).sync(&notes.0, &account.token);
    let mut status = st.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    status.received = 0;
    match result {
        Ok(received) => {
            status.error = None;
            status.received = received;
            status.last_synced_at = Some(now_ms());
        }
        Err(ClientError::SignedOut) => {
            let _ = std::fs::remove_file(st.account_path());
            status = Status { error: Some(ClientError::SignedOut.to_string()), ..Status::default() };
        }
        Err(e) => status.error = Some(e.to_string()),
    }
    *st.status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
    let _ = app.emit(STATUS_EVENT, status);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// "Edge on <hostname>", shown in the list of signed-in devices.
fn device_name() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty());
    host.map_or_else(|| "Laptop".to_owned(), |h| format!("Laptop ({h})"))
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600); // the token is a password
    opts.open(path)?.write_all(text.as_bytes())
}

/// Network calls must not block the main thread, which draws the window.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn sync_status(st: State<SyncState>) -> Status {
    st.status.lock().map(|s| s.clone()).unwrap_or_default()
}

/// "Continue with Google": browser sign-in, then this laptop's own session on the sync server.
#[tauri::command]
pub async fn sync_sign_in_google(app: AppHandle) -> CmdResult<Status> {
    let (email, token) = blocking(|| {
        let id_token = crate::google::sign_in()?;
        let client = Client::new(SERVER);
        let token = client.sign_in_google(&id_token, &device_name()).map_err(|e| e.to_string())?;
        let email = client.me(&token).map_err(|e| e.to_string())?.email;
        Ok((email, token))
    })
    .await?;
    let st = app.state::<SyncState>();
    let json = serde_json::to_string(&Account { email: email.clone(), token }).map_err(|e| e.to_string())?;
    write_private(&st.account_path(), &json).map_err(|e| e.to_string())?;
    let status = Status { email: Some(email), ..Status::default() };
    *st.status.lock().map_err(|e| e.to_string())? = status.clone();
    st.kick(true);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_focus(); // back from the browser
    }
    Ok(status)
}

/// Ends this device's session (best effort) and forgets it. Notes stay on this laptop.
#[tauri::command]
pub async fn sync_sign_out(app: AppHandle) -> CmdResult<Status> {
    let st = app.state::<SyncState>();
    if let Some(a) = st.account() {
        let _ = blocking(move || Client::new(SERVER).sign_out(&a.token).map_err(|e| e.to_string())).await;
    }
    let _ = std::fs::remove_file(st.account_path());
    *st.status.lock().map_err(|e| e.to_string())? = Status::default();
    Ok(Status::default())
}

#[tauri::command]
pub fn sync_now(st: State<SyncState>) {
    st.kick(true);
}
