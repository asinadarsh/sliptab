//! HTTP client for the sync server, shared by the desktop and mobile apps. Blocking (ureq):
//! apps call it from a background thread. The store is locked only to build the request and to
//! apply the answer, never during the network call, so saving a note never waits on the network.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::sync::SyncResponse;

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
        let resp = self.agent.get(format!("{}/v1/me", self.base)).header("authorization", bearer(token)).call();
        read(resp)?.ok_or_else(|| server("empty answer"))
    }

    /// Ends this device's session on the server. Local notes stay.
    pub fn sign_out(&self, token: &str) -> Result<()> {
        // The server finds the current session from the token; "current" is resolved by listing.
        let resp = self.agent.get(format!("{}/v1/sessions", self.base)).header("authorization", bearer(token)).call();
        let sessions: Vec<serde_json::Value> = read(resp)?.unwrap_or_default();
        let Some(id) = sessions.iter().find(|s| s["current"] == true).and_then(|s| s["id"].as_str()) else {
            return Ok(());
        };
        let resp = self
            .agent
            .delete(format!("{}/v1/sessions/{id}", self.base))
            .header("authorization", bearer(token))
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
        let mut req = self.agent.post(format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.header("authorization", bearer(t));
        }
        read(req.send_json(body))
    }
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
        (409, "email_taken") => ClientError::EmailTaken,
        (502..=504, _) => ClientError::Offline(format!("HTTP {status}")),
        _ => ClientError::Server(format!("HTTP {status} {code}")),
    })
}
