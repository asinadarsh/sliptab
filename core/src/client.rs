//! HTTP client for the sync server, shared by the desktop and mobile apps. Blocking (ureq):
//! apps call it from a background thread. The store is locked only to build the request and to
//! apply the answer, never during the network call, so saving a note never waits on the network.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::sync::SyncResponse;

/// "desktop/0.2.0" or "android/1.0.3", sent with every request so the server can tell an app
/// that is too old to update. Set once at start-up with `set_app`.
static APP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_app(platform: &str, version: &str) {
    let _ = APP.set(format!("{platform}/{version}"));
}

fn app_id() -> &'static str {
    APP.get().map_or("", String::as_str)
}

/// The server's limit on changes per request.
const MAX_CHANGES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// No connection, timeout or DNS failure: try again later.
    Offline(String),
    /// The session was revoked or never existed: sign in again.
    SignedOut,
    /// Trial over and not paid.
    SubscriptionRequired,
    /// This app is older than the server allows: update to keep syncing. Holds the minimum.
    UpdateRequired(String),
    /// Google's sign-in token was refused (expired, or not for this app).
    GoogleRejected,
    /// The email already belongs to an Edge account linked to another Google account.
    EmailTaken,
    Server(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ClientError::Offline(_) => "You're offline. Notes will sync when you're back.",
            ClientError::SignedOut => "This device was signed out. Sign in again to sync.",
            ClientError::SubscriptionRequired => "Your trial has ended. Subscribe to keep syncing.",
            ClientError::GoogleRejected => "Google sign-in didn't go through. Please try again.",
            ClientError::EmailTaken => "This email is already used by another Google account in Edge.",
            ClientError::UpdateRequired(_) => "This version of Edge is too old to sync. Please update.",
            ClientError::Server(m) => return write!(f, "Sync server error: {m}"),
        })
    }
}

impl std::error::Error for ClientError {}

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Me {
    pub email: String,
    pub trial_ends_at: i64,
    pub paid_until: Option<i64>,
    pub active: bool,
    /// In the beta: show Saved posts. Absent from older servers, so false.
    #[serde(default)]
    pub beta: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppStatus {
    pub latest: Option<String>,
    pub min: Option<String>,
    /// Where to download the update (the phone; the desktop updater knows on its own).
    pub url: Option<String>,
    pub update_available: bool,
    /// This version can no longer sync.
    pub update_required: bool,
}

#[derive(Deserialize)]
struct Verified {
    token: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// One server, optionally signed in.
pub struct Client {
    base: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Client { base: base.trim_end_matches('/').to_owned(), agent }
    }

    /// Exchanges a Google ID token for this device's session token.
    pub fn sign_in_google(&self, id_token: &str, device_name: &str) -> Result<String> {
        let body = serde_json::json!({ "id_token": id_token, "device_name": device_name });
        Ok(self.post::<Verified>("/v1/auth/google", None, &body)?.ok_or_else(|| server("empty answer"))?.token)
    }

    pub fn me(&self, token: &str) -> Result<Me> {
        let resp = self.agent.get(format!("{}/v1/me", self.base)).header("authorization", bearer(token)).header("x-edge-client", app_id()).call();
        read(resp)?.ok_or_else(|| server("empty answer"))
    }

    /// Ends this device's session on the server. Local notes stay.
    pub fn sign_out(&self, token: &str) -> Result<()> {
        // The server finds the current session from the token; "current" is resolved by listing.
        let resp = self.agent.get(format!("{}/v1/sessions", self.base)).header("authorization", bearer(token)).header("x-edge-client", app_id()).call();
        let sessions: Vec<serde_json::Value> = read(resp)?.unwrap_or_default();
        let Some(id) = sessions.iter().find(|s| s["current"] == true).and_then(|s| s["id"].as_str()) else {
            return Ok(());
        };
        let resp = self
            .agent
            .delete(format!("{}/v1/sessions/{id}", self.base))
            .header("authorization", bearer(token)).header("x-edge-client", app_id())
            .call();
        read::<serde::de::IgnoredAny>(resp).map(|_| ())
    }

    /// Waits (up to about 25 s, the server's limit) until another device stores something after
    /// `after`, the device's cursor. True: sync now. False: nothing yet, ask again.
    pub fn wait_for_changes(&self, token: &str, after: i64) -> Result<bool> {
        #[derive(Deserialize)]
        struct Changes {
            changed: bool,
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(40))) // longer than the server holds it
            .http_status_as_error(false)
            .build()
            .into();
        let resp = agent.get(format!("{}/v1/changes?after={after}", self.base)).header("authorization", bearer(token)).header("x-edge-client", app_id()).call();
        Ok(read::<Changes>(resp)?.is_some_and(|c| c.changed))
    }

    /// Whether this app should update: the latest version, the minimum allowed, and for the phone
    /// where to get it. Needs no sign-in. `platform` is "desktop" or "android".
    pub fn app_status(&self, platform: &str, version: &str) -> Result<AppStatus> {
        let resp = self.agent.get(format!("{}/v1/app/status?platform={platform}&version={version}", self.base)).call();
        read(resp)?.ok_or_else(|| server("empty answer"))
    }

    /// Deletes the account and everything synced to it. Local notes on this device stay.
    pub fn delete_account(&self, token: &str) -> Result<()> {
        let resp = self.agent.delete(format!("{}/v1/me", self.base)).header("authorization", bearer(token)).header("x-edge-client", app_id()).call();
        read::<serde::de::IgnoredAny>(resp).map(|_| ())
    }

    /// Sends every queued change and fetches everything new, repeating until both are done.
    /// Returns how many notes arrived from other devices.
    pub fn sync(&self, store: &Mutex<Store>, token: &str) -> Result<usize> {
        let mut received = 0;
        for _ in 0..1000 {
            let mut req = lock(store).sync_request().map_err(local)?;
            req.changes.truncate(MAX_CHANGES);
            let sent: std::collections::HashSet<_> = req.changes.iter().map(|c| c.note.id).collect();
            let resp: SyncResponse =
                self.post("/v1/sync", Some(token), &req)?.ok_or_else(|| server("empty answer"))?;
            received += resp.notes.iter().filter(|n| !sent.contains(&n.id)).count();
            let more = resp.more || req.changes.len() == MAX_CHANGES;
            lock(store).apply_sync(&req, &resp).map_err(local)?;
            if !more {
                return Ok(received);
            }
        }
        Err(server("sync did not finish"))
    }

    fn post<T: serde::de::DeserializeOwned>(&self, path: &str, token: Option<&str>, body: &impl Serialize) -> Result<Option<T>> {
        let mut req = self.agent.post(format!("{}{path}", self.base)).header("x-edge-client", app_id());
        if let Some(t) = token {
            req = req.header("authorization", bearer(t));
        }
        read(req.send_json(body))
    }
}

/// Who we say we are when reading a saved post's page. Instagram shows its preview tags to a
/// link-preview bot but not to a phone browser (S0 test), and being honest about it is right.
const PREVIEW_AGENT: &str = "EdgeBot/0.1 (link preview; +https://edge-sync.forcesync.in)";
const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Reads what a saved post's page says about itself. Ok with an empty preview when the post is
/// gone or private (nothing more to learn); Err when it is worth trying again later.
pub fn fetch_preview(link: &crate::Link) -> std::result::Result<crate::preview::Preview, String> {
    use crate::preview::{parse_html, parse_youtube_oembed, Preview};
    use ureq::ResponseExt;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .max_redirects(5)
        .user_agent(PREVIEW_AGENT)
        .build()
        .into();
    let url = if link.platform == "youtube" {
        format!("https://www.youtube.com/oembed?format=json&url={}", percent_encode(&link.url))
    } else {
        link.url.clone()
    };
    let mut resp = agent
        .get(&url)
        .header("accept-language", "en-IN,en;q=0.9")
        .header("accept", "text/html,application/json;q=0.9,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;
    match resp.status().as_u16() {
        200 => {}
        429 | 500..=599 => return Err(format!("HTTP {}", resp.status())),
        _ => return Ok(Preview::default()), // 404 deleted, 401/403 private: nothing to read
    }
    let final_url = resp.get_uri().to_string();
    let body = resp.body_mut().with_config().limit(MAX_PAGE_BYTES).lossy_utf8(true).read_to_string().map_err(|e| e.to_string())?;
    Ok(if link.platform == "youtube" { parse_youtube_oembed(&body) } else { parse_html(&link.platform, &final_url, &body) })
}

/// Fetches details for up to `max` saves that need them, one at a time, never holding the store
/// lock during a fetch. Returns how many saves changed.
pub fn fill_details(store: &Mutex<Store>, max: usize) -> usize {
    let Ok(pending) = lock(store).saves_needing_details(max) else { return 0 };
    let mut changed = 0;
    for n in pending {
        let Some(link) = n.link else { continue };
        let result = fetch_preview(&link);
        let mut s = lock(store);
        let ok = match result {
            Ok(p) => s.set_save_details(n.id, &p).is_ok(),
            Err(_) => s.record_fetch_failure(n.id).map(|_| false).unwrap_or(false),
        };
        changed += ok as usize;
    }
    changed
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn lock(store: &Mutex<Store>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

fn server(m: &str) -> ClientError {
    ClientError::Server(m.to_owned())
}

fn local(e: rusqlite::Error) -> ClientError {
    ClientError::Server(format!("local database: {e}"))
}

/// Maps a response to a value (None for 204) or to the error the server named.
fn read<T: serde::de::DeserializeOwned>(
    resp: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Option<T>> {
    let mut resp = resp.map_err(|e| ClientError::Offline(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 204 {
        return Ok(None);
    }
    let text = resp.body_mut().read_to_string().map_err(|e| ClientError::Offline(e.to_string()))?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&text).map(Some).map_err(|e| server(&format!("bad answer: {e}")));
    }
    let code = serde_json::from_str::<ErrorBody>(&text).map(|b| b.error).unwrap_or_default();
    Err(match (status, code.as_str()) {
        (401, "google_rejected") => ClientError::GoogleRejected,
        (401, _) => ClientError::SignedOut,
        (402, _) => ClientError::SubscriptionRequired,
        (426, _) => ClientError::UpdateRequired(
            serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["min"].as_str().map(str::to_owned)).unwrap_or_default(),
        ),
        (409, "email_taken") => ClientError::EmailTaken,
        (502..=504, _) => ClientError::Offline(format!("HTTP {status}")),
        _ => ClientError::Server(format!("HTTP {status} {code}")),
    })
}
