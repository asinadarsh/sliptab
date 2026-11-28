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
pub const NOTES_EVENT: &str = "notes-changed";
/// Saves got their title, thumbnail or folder: the page reloads its lists.
pub const SAVES_EVENT: &str = "saves-updated";

#[derive(Serialize, Deserialize)]
struct Account {
    email: String,
    token: String,
    /// In the beta (Saved posts). Refreshed from /v1/me now and then.
    #[serde(default)]
    beta: bool,
    /// When the trial ends and until when it is paid (ms), from /v1/me: AI agents (MCP) are part
    /// of the subscription, and the MCP server checks these offline. None in older files.
    #[serde(default)]
    trial_ends_at: Option<i64>,
    #[serde(default)]
    paid_until: Option<i64>,
    #[serde(default)]
    billing: bool,
}

/// Signed in with a running trial or subscription (what sync and AI agents need), from the
/// account file in `dir`. Used by the MCP server, which runs without the app.
pub fn subscribed(dir: &Path) -> bool {
    let Some(a) = std::fs::read_to_string(dir.join("account.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Account>(&s).ok())
    else {
        return false;
    };
    let now = now_ms();
    match (a.trial_ends_at, a.paid_until) {
        (None, None) => true, // from before these were stored: the next refresh fills them in
        (t, p) => t.unwrap_or(0) > now || p.unwrap_or(0) > now,
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    email: Option<String>,
    last_synced_at: Option<i64>,
    error: Option<String>,
    /// Notes that arrived from other devices in the last sync.
    received: usize,
    /// The account is in the beta: show Saved posts. False when signed out.
    beta: bool,
    /// The plan, for Settings → Sync: trial end and paid-until (ms), and whether Subscribe can
    /// be offered (the server sells subscriptions).
    trial_ends_at: Option<i64>,
    paid_until: Option<i64>,
    billing: bool,
}

/// When the account was last re-read from the server; 0 makes the next sync re-read it (after a
/// checkout, so a new subscription shows within a minute).
static LAST_ME: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

impl Status {
    fn plan(mut self, a: &Account) -> Self {
        (self.trial_ends_at, self.paid_until, self.billing) =
            (a.trial_ends_at, a.paid_until, a.billing);
        self
    }
}

pub struct SyncState {
    app: AppHandle,
    dir: PathBuf,
    kick: Mutex<Sender<bool>>,
    status: Mutex<Status>,
}

impl SyncState {
    /// Ask for a sync: `now` skips the 2 s wait that lets typing settle. A local edit (`now` false)
    /// also tells the other window (edge panel or app window) to reload its lists.
    pub fn kick(&self, now: bool) {
        if !now {
            let _ = self.app.emit(NOTES_EVENT, ());
        }
        if let Ok(k) = self.kick.lock() {
            let _ = k.send(now);
        }
    }

    /// Attachment files, named by their SHA-256.
    pub fn media_dir(&self) -> PathBuf {
        self.dir.join("media")
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
    let state = SyncState {
        app: app.clone(),
        dir: dir.to_owned(),
        kick: Mutex::new(tx),
        status: Mutex::new(Status::default()),
    };
    {
        let mut status = state.status.lock().unwrap();
        status.email = state.account().map(|a| a.email);
        status.beta = state.account().is_some_and(|a| a.beta);
        if let Some(a) = state.account() {
            *status = std::mem::take(&mut *status).plan(&a);
        }
    }
    app.manage(state);
    app.state::<SyncState>().kick(true); // first sync right after start-up
    listen(app.clone());
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

/// Holds a long poll open to the server, so a change made on another device (a pin on the phone)
/// syncs here within about a second instead of on the next 60 s round.
fn listen(app: AppHandle) {
    std::thread::spawn(move || loop {
        let st = app.state::<SyncState>();
        let Some(account) = st.account() else {
            std::thread::sleep(Duration::from_secs(5)); // signed out: check again shortly
            continue;
        };
        let cursor = app
            .state::<Notes>()
            .0
            .lock()
            .ok()
            .and_then(|s| s.cursor().ok())
            .unwrap_or(0);
        match Client::new(SERVER).wait_for_changes(&account.token, cursor) {
            Ok(true) => {
                st.kick(true);
                // Let that sync move the cursor before asking again; never spins if it fails.
                std::thread::sleep(Duration::from_secs(2));
            }
            Ok(false) => {}
            Err(_) => std::thread::sleep(Duration::from_secs(15)), // offline or server down
        }
    });
}

fn run(app: &AppHandle) {
    let st = app.state::<SyncState>();
    let notes = app.state::<Notes>();
    // A few new saves get their details first (they go up with them); a few at a time, so slow
    // pages never hold a typed note back for long. The rest follow next round. Signed out too.
    if notes_core::client::fill_details(&notes.0, 3) > 0 {
        let _ = app.emit(SAVES_EVENT, ());
    }
    let Some(account) = st.account() else { return };
    let client = Client::new(SERVER);
    // Files first, so another device can open an attachment as soon as its note arrives. A
    // failed upload is retried next time; the notes sync either way.
    if let Err(e) = client.upload_media(&notes.0, &account.token, &st.media_dir()) {
        eprintln!("media upload: {e}");
    }
    let result = client.sync(&notes.0, &account.token);
    let mut status = st.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    status.received = 0;
    match result {
        Ok(received) => {
            status.error = None;
            status.received = received;
            status.last_synced_at = Some(now_ms());
            refresh_beta(&st, &account, &mut status);
        }
        Err(ClientError::SignedOut) => {
            let _ = std::fs::remove_file(st.account_path());
            status = Status {
                error: Some(ClientError::SignedOut.to_string()),
                ..Status::default()
            };
        }
        Err(e) => {
            if matches!(e, ClientError::UpdateRequired(_)) {
                crate::update::check_soon(app); // the server refuses this version: update now
            }
            status.error = Some(e.to_string());
        }
    }
    *st.status.lock().unwrap_or_else(|e| e.into_inner()) = status.clone();
    let _ = app.emit(STATUS_EVENT, status);
}

/// Re-reads the beta flag from the server at most every 10 minutes, so a newly granted beta
/// shows up without signing in again.
fn refresh_beta(st: &SyncState, account: &Account, status: &mut Status) {
    *status = std::mem::take(status).plan(account);
    let now = now_ms();
    if now - LAST_ME.load(std::sync::atomic::Ordering::Relaxed) < 600_000 {
        return;
    }
    LAST_ME.store(now, std::sync::atomic::Ordering::Relaxed);
    let Ok(me) = Client::new(SERVER).me(&account.token) else {
        return;
    };
    status.beta = me.beta;
    let updated = Account {
        email: account.email.clone(),
        token: account.token.clone(),
        beta: me.beta,
        trial_ends_at: Some(me.trial_ends_at),
        paid_until: me.paid_until,
        billing: me.billing,
    };
    *status = std::mem::take(status).plan(&updated);
    if let Ok(json) = serde_json::to_string(&updated) {
        let _ = write_private(&st.account_path(), &json);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
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
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
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
        let token = client
            .sign_in_google(&id_token, &device_name())
            .map_err(|e| e.to_string())?;
        let me = client.me(&token).map_err(|e| e.to_string())?;
        Ok((me, token))
    })
    .await?;
    let (trial_ends_at, paid_until, billing) =
        (Some(email.trial_ends_at), email.paid_until, email.billing);
    let (email, beta) = (email.email, email.beta);
    let st = app.state::<SyncState>();
    let json = serde_json::to_string(&Account {
        email: email.clone(),
        token,
        beta,
        trial_ends_at,
        paid_until,
        billing,
    })
    .map_err(|e| e.to_string())?;
    // A different account than this laptop last synced with: start over (see Store::use_account).
    if let Ok(mut notes) = app.state::<Notes>().0.lock() {
        if let Err(e) = notes.use_account(&email) {
            eprintln!("use_account: {e}");
        }
    }
    write_private(&st.account_path(), &json).map_err(|e| e.to_string())?;
    let status = Status {
        email: Some(email),
        beta,
        trial_ends_at,
        paid_until,
        billing,
        ..Status::default()
    };
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
        let _ = blocking(move || {
            Client::new(SERVER)
                .sign_out(&a.token)
                .map_err(|e| e.to_string())
        })
        .await;
    }
    let _ = std::fs::remove_file(st.account_path());
    *st.status.lock().map_err(|e| e.to_string())? = Status::default();
    Ok(Status::default())
}

/// Deletes the account on the server (notes there and every device's session) and signs this
/// laptop out. Notes on this laptop stay.
#[tauri::command]
pub async fn sync_delete_account(app: AppHandle) -> CmdResult<Status> {
    let st = app.state::<SyncState>();
    let Some(a) = st.account() else {
        return Ok(Status::default());
    };
    blocking(move || {
        Client::new(SERVER)
            .delete_account(&a.token)
            .map_err(|e| e.to_string())
    })
    .await?;
    // The account is gone from the server; a new one starts counting from zero (Store::start_over).
    if let Ok(mut notes) = app.state::<Notes>().0.lock() {
        if let Err(e) = notes.start_over() {
            eprintln!("start_over: {e}");
        }
    }
    let _ = std::fs::remove_file(st.account_path());
    *st.status.lock().map_err(|e| e.to_string())? = Status::default();
    Ok(Status::default())
}

/// The bytes of attachment `sha256`, downloaded from the server first when this device lacks
/// them (attached on another device). Raw bytes, not JSON, so a 50 MB file is not inflated.
#[tauri::command]
pub async fn media_get(app: AppHandle, sha256: String) -> CmdResult<tauri::ipc::Response> {
    blocking(move || {
        let path = local_media(&app, &sha256)?;
        std::fs::read(path)
            .map(tauri::ipc::Response::new)
            .map_err(|e| e.to_string())
    })
    .await
}

/// The local path of attachment `sha256`, downloading it first when this device lacks it.
fn local_media(app: &AppHandle, sha256: &str) -> CmdResult<PathBuf> {
    // A name, never a path: 64 hex characters, so nothing from the page can reach other files.
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("not an attachment".into());
    }
    let st = app.state::<SyncState>();
    let dir = st.media_dir();
    if dir.join(sha256).exists() {
        return Ok(dir.join(sha256));
    }
    let account = st
        .account()
        .ok_or("Sign in to sync to see this file here.")?;
    Client::new(SERVER)
        .media_file(&account.token, &dir, sha256)
        .map_err(|e| e.to_string())
}

/// Saves a copy of an attachment in the Downloads folder under its own name ("photo (2).jpg"
/// when that name is taken). Returns where it went.
#[tauri::command]
pub async fn media_save(app: AppHandle, sha256: String, name: String) -> CmdResult<String> {
    blocking(move || {
        let src = local_media(&app, &sha256)?;
        let dir = app.path().download_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = safe_name(&name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut dst = dir.join(&name);
        for i in 2.. {
            if !dst.exists() {
                break;
            }
            dst = dir.join(format!("{stem} ({i}){ext}"));
        }
        std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        Ok(dst.to_string_lossy().into_owned())
    })
    .await
}

/// Text on the clipboard, for the app's own Paste (WebKitGTK does not let the page read it).
#[tauri::command]
pub async fn clip_read(app: AppHandle) -> CmdResult<String> {
    #[cfg(target_os = "linux")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let text = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).wait_for_text();
            let _ = tx.send(text.map(|t| t.to_string()).unwrap_or_default());
        })
        .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Err("use the page clipboard".into())
    }
}

/// Puts text on the clipboard, for the app's own Cut and Copy.
#[tauri::command]
pub async fn clip_write(app: AppHandle, text: String) -> CmdResult<()> {
    #[cfg(target_os = "linux")]
    {
        app.run_on_main_thread(move || {
            gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_text(&text)
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, text);
        Err("use the page clipboard".into())
    }
}

/// Puts an image attachment on the clipboard (Linux: GTK's clipboard, as the page's own
/// clipboard API cannot write images in WebKitGTK).
#[tauri::command]
pub async fn media_copy_image(app: AppHandle, sha256: String) -> CmdResult<()> {
    let path = {
        let app = app.clone();
        blocking(move || local_media(&app, &sha256)).await?
    };
    #[cfg(target_os = "linux")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let r = gtk::gdk_pixbuf::Pixbuf::from_file(&path).map(|p| {
                gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD).set_image(&p);
            });
            let _ = tx.send(r.map_err(|e| e.to_string()));
        })
        .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, path);
        Err("Copying images is not available here yet; use Save to Downloads.".into())
    }
}

/// Opens a file attachment (a PDF, a document) in its app: a copy under its own name in the
/// cache, so the app shows a sensible title. Files that could run code are not opened.
#[tauri::command]
pub async fn media_open(app: AppHandle, sha256: String, name: String) -> CmdResult<()> {
    blocking(move || {
        let src = local_media(&app, &sha256)?;
        let name = safe_name(&name);
        if !openable(&name) {
            return Err("Sliptab opens documents, images and media only. Use Save to Downloads for this file.".into());
        }
        let out = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("open")
            .join(&sha256[..12]);
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let dst = out.join(&name);
        std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        crate::google::open_browser(&dst.to_string_lossy())
    })
    .await
}

/// The last path part only, without characters a file system refuses.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c))
        .take(120)
        .collect();
    let clean = clean
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_owned();
    if clean.is_empty() {
        "file".into()
    } else {
        clean
    }
}

/// Files Sliptab opens in their app: documents, images, audio and video, by extension. An
/// allowlist, because every OS has file types that run code (and new ones keep appearing);
/// anything else can still be saved to Downloads.
fn openable(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "pdf"
            | "txt"
            | "md"
            | "csv"
            | "tsv"
            | "rtf"
            | "json"
            | "log"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "pages"
            | "numbers"
            | "key"
            | "epub"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "heic"
            | "heif"
            | "bmp"
            | "tif"
            | "tiff"
            | "avif"
            | "mp3"
            | "m4a"
            | "aac"
            | "wav"
            | "flac"
            | "ogg"
            | "oga"
            | "opus"
            | "weba"
            | "amr"
            | "mp4"
            | "m4v"
            | "mov"
            | "webm"
            | "mkv"
            | "avi"
            | "zip"
    )
}

#[test]
fn opened_files_get_safe_names_and_programs_are_refused() {
    assert_eq!(safe_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_name("C:\\x\\Report: Q3?.pdf"), "Report Q3.pdf");
    assert_eq!(safe_name(".bashrc"), "bashrc");
    assert_eq!(safe_name(""), "file");
    assert_eq!(safe_name("setup.exe. . "), "setup.exe");
    for no in [
        "setup.EXE",
        "x.desktop",
        "a.sh",
        "x.chm",
        "a.url",
        "b.msc",
        "c.terminal",
        "d.svg",
        "e.html",
        "README",
        "setup.exe.",
    ] {
        assert!(!openable(&safe_name(no)), "{no}");
    }
    for yes in [
        "notes.pdf",
        "song.mp3",
        "Photo.HEIC",
        "plan.docx",
        "voice.m4a",
    ] {
        assert!(openable(yes), "{yes}");
    }
}

/// Opens the checkout for a subscription ("monthly" or "yearly") in the browser.
#[tauri::command]
pub async fn billing_checkout(app: AppHandle, plan: String) -> CmdResult<()> {
    blocking(move || {
        let account = app.state::<SyncState>().account().ok_or("Sign in first.")?;
        let url = Client::new(SERVER)
            .checkout_url(&account.token, &plan)
            .map_err(|e| e.to_string())?;
        LAST_ME.store(0, std::sync::atomic::Ordering::Relaxed);
        crate::google::open_browser(&url)
    })
    .await
}

/// Opens the payment provider's page to change or cancel the subscription.
#[tauri::command]
pub async fn billing_portal(app: AppHandle) -> CmdResult<()> {
    blocking(move || {
        let account = app.state::<SyncState>().account().ok_or("Sign in first.")?;
        let url = Client::new(SERVER)
            .portal_url(&account.token)
            .map_err(|e| e.to_string())?;
        LAST_ME.store(0, std::sync::atomic::Ordering::Relaxed);
        crate::google::open_browser(&url)
    })
    .await
}

#[tauri::command]
pub fn sync_now(st: State<SyncState>) {
    st.kick(true);
}
