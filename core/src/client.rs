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
    /// The email already belongs to an Sliptab account linked to another Google account.
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
            ClientError::EmailTaken => {
                "This email is already used by another Google account in Sliptab."
            }
            ClientError::UpdateRequired(_) => {
                "This version of Sliptab is too old to sync. Please update."
            }
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
    /// A subscription pays right now (not only the trial).
    #[serde(default)]
    pub subscribed: bool,
    /// The server sells subscriptions: show Subscribe.
    #[serde(default)]
    pub billing: bool,
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
        Client {
            base: base.trim_end_matches('/').to_owned(),
            agent,
        }
    }

    /// Exchanges a Google ID token for this device's session token.
    pub fn sign_in_google(&self, id_token: &str, device_name: &str) -> Result<String> {
        let body = serde_json::json!({ "id_token": id_token, "device_name": device_name });
        Ok(self
            .post::<Verified>("/v1/auth/google", None, &body)?
            .ok_or_else(|| server("empty answer"))?
            .token)
    }

    pub fn me(&self, token: &str) -> Result<Me> {
        let resp = self
            .agent
            .get(format!("{}/v1/me", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
            .call();
        read(resp)?.ok_or_else(|| server("empty answer"))
    }

    /// Ends this device's session on the server. Local notes stay.
    pub fn sign_out(&self, token: &str) -> Result<()> {
        // The server finds the current session from the token; "current" is resolved by listing.
        let resp = self
            .agent
            .get(format!("{}/v1/sessions", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
            .call();
        let sessions: Vec<serde_json::Value> = read(resp)?.unwrap_or_default();
        let Some(id) = sessions
            .iter()
            .find(|s| s["current"] == true)
            .and_then(|s| s["id"].as_str())
        else {
            return Ok(());
        };
        let resp = self
            .agent
            .delete(format!("{}/v1/sessions/{id}", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
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
        let resp = agent
            .get(format!("{}/v1/changes?after={after}", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
            .call();
        Ok(read::<Changes>(resp)?.is_some_and(|c| c.changed))
    }

    /// Whether this app should update: the latest version, the minimum allowed, and for the phone
    /// where to get it. Needs no sign-in. `platform` is "desktop" or "android".
    pub fn app_status(&self, platform: &str, version: &str) -> Result<AppStatus> {
        self.app_status_with(platform, version, "")
    }

    /// `app_status` with extra query parameters: the counting flags (`d=1&w=0…`), or `nc=1`
    /// for a check the server must not count. Only `[a-z0-9=&]` is passed on.
    pub fn app_status_with(&self, platform: &str, version: &str, extra: &str) -> Result<AppStatus> {
        let extra: String = extra
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '=' || *c == '&')
            .collect();
        let resp = self
            .agent
            .get(format!(
                "{}/v1/app/status?platform={platform}&version={version}{}{extra}",
                self.base,
                if extra.is_empty() { "" } else { "&" }
            ))
            .call();
        read(resp)?.ok_or_else(|| server("empty answer"))
    }

    /// A checkout page for a subscription ("monthly" or "yearly"), to open in the browser.
    pub fn checkout_url(&self, token: &str, plan: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Url {
            url: String,
        }
        let body = serde_json::json!({ "plan": plan });
        Ok(self
            .post::<Url>("/v1/billing/checkout", Some(token), &body)?
            .ok_or_else(|| server("empty answer"))?
            .url)
    }

    /// The payment provider's page to change or cancel the subscription.
    pub fn portal_url(&self, token: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Url {
            url: String,
        }
        Ok(self
            .post::<Url>("/v1/billing/portal", Some(token), &serde_json::json!({}))?
            .ok_or_else(|| server("empty answer"))?
            .url)
    }

    /// Deletes the account and everything synced to it. Local notes on this device stay.
    pub fn delete_account(&self, token: &str) -> Result<()> {
        let resp = self
            .agent
            .delete(format!("{}/v1/me", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
            .call();
        read::<serde::de::IgnoredAny>(resp).map(|_| ())
    }

    /// Sends every queued change and fetches everything new, repeating until both are done.
    /// Returns how many notes arrived from other devices.
    pub fn sync(&self, store: &Mutex<Store>, token: &str) -> Result<usize> {
        let mut received = 0;
        for _ in 0..1000 {
            let mut req = lock(store).sync_request().map_err(local)?;
            req.changes.truncate(MAX_CHANGES);
            let sent: std::collections::HashSet<_> =
                req.changes.iter().map(|c| c.note.id).collect();
            let resp: SyncResponse = self
                .post("/v1/sync", Some(token), &req)?
                .ok_or_else(|| server("empty answer"))?;
            received += resp.notes.iter().filter(|n| !sent.contains(&n.id)).count();
            let more = resp.more || req.changes.len() == MAX_CHANGES;
            lock(store).apply_sync(&req, &resp).map_err(local)?;
            if !more {
                return Ok(received);
            }
        }
        Err(server("sync did not finish"))
    }

    /// Uploads attachment files this device has and the server may not, so other devices can
    /// download them. Call before `sync`. A file that fails stays pending for the next time;
    /// the note syncs anyway and the other device shows the file once it arrives.
    pub fn upload_media(
        &self,
        store: &Mutex<Store>,
        token: &str,
        media_dir: &std::path::Path,
    ) -> Result<usize> {
        let mut done = 0;
        // Bound first: a guard in the `for` header would stay locked for the whole loop, and
        // `mark_uploaded` below locks again (a deadlock that froze the app, 2026-11-22).
        let pending = lock(store).media_to_upload().map_err(local)?;
        let mut failed = None;
        for sha in pending {
            let Ok(bytes) = std::fs::read(media_dir.join(&sha)) else {
                continue; // attached on another device and not downloaded here
            };
            let resp = self
                .media_agent()
                .put(format!("{}/v1/media/{sha}", self.base))
                .header("authorization", bearer(token))
                .header("x-edge-client", app_id())
                .header("content-type", "application/octet-stream")
                .send(&bytes[..]);
            // One file failing (quota full, a damaged local copy) must not hold back the others.
            // Offline or signed out: stop, the rest would fail the same way.
            match read::<serde::de::IgnoredAny>(resp) {
                Ok(_) => {
                    lock(store).mark_uploaded(&sha).map_err(local)?;
                    done += 1;
                }
                Err(
                    e @ (ClientError::Offline(_)
                    | ClientError::SignedOut
                    | ClientError::UpdateRequired(_)),
                ) => return Err(e),
                Err(e) => failed = Some(e),
            }
        }
        match failed {
            Some(e) if done == 0 => Err(e),
            _ => Ok(done),
        }
    }

    /// The local file for attachment `sha256`, downloading it first if this device lacks it.
    /// The download is checked against its hash before it gets its real name.
    pub fn media_file(
        &self,
        token: &str,
        media_dir: &std::path::Path,
        sha256: &str,
    ) -> Result<std::path::PathBuf> {
        let valid = sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid {
            return Err(server("bad file name"));
        }
        let path = media_dir.join(sha256);
        if path.exists() {
            return Ok(path);
        }
        let resp = self
            .media_agent()
            .get(format!("{}/v1/media/{sha256}", self.base))
            .header("authorization", bearer(token))
            .header("x-edge-client", app_id())
            .call();
        let mut resp = match resp {
            Ok(r) if r.status().as_u16() == 200 => r,
            other => {
                read::<serde::de::IgnoredAny>(other)?;
                return Err(server("not uploaded yet"));
            }
        };
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(crate::attachment::MAX_ATTACHMENT_BYTES)
            .read_to_vec()
            .map_err(|e| ClientError::Offline(e.to_string()))?;
        let (got, _) =
            crate::attachment::store_file(media_dir, &bytes).map_err(|e| server(&e.to_string()))?;
        if got != sha256 {
            let _ = std::fs::remove_file(media_dir.join(&got));
            return Err(server("downloaded file does not match"));
        }
        Ok(path)
    }

    /// Big files need longer than the 30 s the sync requests get.
    fn media_agent(&self) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &impl Serialize,
    ) -> Result<Option<T>> {
        let mut req = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("x-edge-client", app_id());
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
        format!(
            "https://www.youtube.com/oembed?format=json&url={}",
            percent_encode(&link.url)
        )
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
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_PAGE_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    Ok(if link.platform == "youtube" {
        parse_youtube_oembed(&body)
    } else {
        parse_html(&link.platform, &final_url, &body)
    })
}

/// Fetches details for up to `max` saves that need them, one at a time, never holding the store
/// lock during a fetch. Returns how many saves changed.
pub fn fill_details(store: &Mutex<Store>, max: usize) -> usize {
    let Ok(pending) = lock(store).saves_needing_details(max) else {
        return 0;
    };
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
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
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| ClientError::Offline(e.to_string()))?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| server(&format!("bad answer: {e}")));
    }
    let code = serde_json::from_str::<ErrorBody>(&text)
        .map(|b| b.error)
        .unwrap_or_default();
    Err(match (status, code.as_str()) {
        (401, "google_rejected") => ClientError::GoogleRejected,
        (401, _) => ClientError::SignedOut,
        (402, _) => ClientError::SubscriptionRequired,
        (426, _) => ClientError::UpdateRequired(
            serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["min"].as_str().map(str::to_owned))
                .unwrap_or_default(),
        ),
        (409, "email_taken") => ClientError::EmailTaken,
        (502..=504, _) => ClientError::Offline(format!("HTTP {status}")),
        _ => ClientError::Server(format!("HTTP {status} {code}")),
    })
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use std::io::{Read, Write};

    /// A server that answers every request with 204 (or the first with `first`), counting them.
    fn tiny_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        server_with(None)
    }

    fn server_with(
        first: Option<&'static str>,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = n.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut c = c;
                // Read the whole request (headers, then Content-Length bytes) before answering:
                // Windows aborts a connection whose unread request data is discarded.
                let mut req = Vec::new();
                let mut buf = [0u8; 65536];
                let body_end = loop {
                    let Ok(n) = c.read(&mut buf) else { break None };
                    if n == 0 {
                        break None;
                    }
                    req.extend_from_slice(&buf[..n]);
                    if let Some(h) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..h]).to_ascii_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break Some(h + 4 + len);
                    }
                };
                while body_end.is_some_and(|e| req.len() < e) {
                    match c.read(&mut buf) {
                        Ok(n) if n > 0 => req.extend_from_slice(&buf[..n]),
                        _ => break,
                    }
                }
                let nth = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let reply = match first {
                    Some(f) if nth == 0 => f,
                    _ => {
                        "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                };
                let _ = c.write_all(reply.as_bytes());
            }
        });
        (url, n)
    }

    #[test]
    fn uploading_files_finishes_and_marks_them_sent() {
        let dir = std::env::temp_dir().join(format!("edge-up-{}", uuid::Uuid::now_v7()));
        let (sha, len) = crate::attachment::store_file(&dir, b"photo bytes").unwrap();
        let mut s = Store::open_in_memory("phone").unwrap();
        let n = s.create("Trip").unwrap();
        let a =
            crate::attachment::new_attachment(sha, len, "image/jpeg", None, None, None, None, 1)
                .unwrap();
        s.add_attachment(n.id, a).unwrap();
        let store = Mutex::new(s);
        let (url, hits) = tiny_server();
        // Run on a thread with a deadline: a lock held across the loop used to hang here forever.
        let (tx, rx) = std::sync::mpsc::channel();
        let (st, d) = (std::sync::Arc::new(store), dir.clone());
        let st2 = st.clone();
        std::thread::spawn(move || {
            tx.send(Client::new(&url).upload_media(&st2, "t", &d))
                .unwrap()
        });
        let sent = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("upload_media hung (deadlock)");
        assert_eq!(sent.unwrap(), 1);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            lock(&st).media_to_upload().unwrap().is_empty(),
            "marked as sent"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_refused_file_does_not_hold_back_the_others() {
        let dir = std::env::temp_dir().join(format!("edge-up2-{}", uuid::Uuid::now_v7()));
        let mut s = Store::open_in_memory("phone").unwrap();
        let n = s.create("Trip").unwrap();
        for b in [&b"one"[..], b"two"] {
            let (sha, len) = crate::attachment::store_file(&dir, b).unwrap();
            let a = crate::attachment::new_attachment(
                sha,
                len,
                "image/jpeg",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap();
            s.add_attachment(n.id, a).unwrap();
        }
        let store = Mutex::new(s);
        let full = "HTTP/1.1 507 Insufficient Storage\r\nContent-Length: 24\r\nConnection: close\r\n\r\n{\"error\":\"storage_full\"}";
        let (url, _) = server_with(Some(full));
        assert_eq!(
            Client::new(&url).upload_media(&store, "t", &dir).unwrap(),
            1,
            "the second still went"
        );
        assert_eq!(
            lock(&store).media_to_upload().unwrap().len(),
            1,
            "the refused one is retried later"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
