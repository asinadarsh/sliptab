//! "Continue with Google" on the laptop: the OAuth flow for installed apps. Opens the browser at
//! Google with PKCE, catches the answer on a one-off http://127.0.0.1:<port> listener, swaps the
//! code for an ID token. Google's Desktop client type allows any loopback port.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::{Digest, Sha256};

/// Baked in at build time from the Desktop OAuth client (see build-desktop.sh); not in git.
/// Google requires the secret for Desktop clients even with PKCE and treats it as public.
const CLIENT_ID: Option<&str> = option_env!("EDGE_GOOGLE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("EDGE_GOOGLE_CLIENT_SECRET");
const WAIT: Duration = Duration::from_secs(300);

/// Runs the whole browser flow; blocks until the user finishes or 5 minutes pass.
pub fn sign_in() -> Result<String, String> {
    let (Some(client_id), Some(secret)) = (CLIENT_ID, CLIENT_SECRET) else {
        return Err("Google sign-in isn't set up in this build.".into());
    };
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let verifier = random_b64(32)?;
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = random_b64(16)?;
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&scope=openid%20email\
         &prompt=select_account&code_challenge_method=S256&client_id={}&redirect_uri={}\
         &code_challenge={challenge}&state={state}",
        enc(client_id),
        enc(&redirect)
    );
    open_browser(&url)?;

    let params = wait_for_redirect(&listener)?;
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    if get("state") != Some(state.as_str()) {
        return Err("Sign-in was interrupted. Please try again.".into());
    }
    if let Some(e) = get("error") {
        return Err(if e == "access_denied" { "Sign-in was cancelled.".into() } else { format!("Google said: {e}") });
    }
    let code = get("code").ok_or("Google sent no code.")?;

    let mut resp = ureq::post("https://oauth2.googleapis.com/token")
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .send_form([
            ("client_id", client_id),
            ("client_secret", secret),
            ("code", code),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect.as_str()),
        ])
        .map_err(|_| "You're offline. Try again when you're connected.".to_string())?;
    let body: serde_json::Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
    body["id_token"].as_str().map(str::to_owned).ok_or_else(|| {
        format!("Google refused the sign-in: {}", body["error_description"].as_str().unwrap_or("unknown error"))
    })
}

/// Serves requests until one carries the OAuth answer (browsers also ask for /favicon.ico).
fn wait_for_redirect(listener: &TcpListener) -> Result<Vec<(String, String)>, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let target = line.split_whitespace().nth(1).unwrap_or("");
        let params = parse_query(target.split_once('?').map_or("", |(_, q)| q));
        let done = params.iter().any(|(k, _)| k == "code" || k == "error");
        let page = if done { DONE_PAGE } else { "" };
        let status = if done { "200 OK" } else { "404 Not Found" };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        if done {
            return Ok(params);
        }
    }
    Err("Sign-in timed out. Please try again.".into())
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Edge</title>\
<body style=\"font:16px system-ui;display:grid;place-items:center;height:90vh;color:#444\">\
<div>You're signed in to Edge. You can close this tab.</div>";

pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    cmd.map(drop).map_err(|e| format!("Couldn't open the browser: {e}"))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_b64(n: usize) -> Result<String, String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(b64(&buf))
}

/// Percent-encodes everything but unreserved characters.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    let dec = |s: &str| {
        let s = s.replace('+', " ");
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            let hex = (b[i] == b'%' && i + 2 < b.len())
                .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
                .flatten();
            match hex {
                Some(v) => {
                    out.push(v);
                    i += 3;
                }
                None => {
                    out.push(b[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    q.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).map(|(k, v)| (dec(k), dec(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_round_trip() {
        assert_eq!(enc("http://127.0.0.1:5000"), "http%3A%2F%2F127.0.0.1%3A5000");
        let p = parse_query("state=ab_c&code=4%2F0Ab-x%3D&scope=email+openid&x");
        assert_eq!(p[0], ("state".into(), "ab_c".into()));
        assert_eq!(p[1], ("code".into(), "4/0Ab-x=".into()));
        assert_eq!(p[2], ("scope".into(), "email openid".into()));
        assert_eq!(p[3], ("x".into(), String::new()));
        assert_eq!(parse_query("bad=%zz%4"), vec![("bad".into(), "%zz%4".into())]);
    }

    #[test]
    fn pkce_challenge_matches_rfc7636() {
        // RFC 7636 appendix B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64(&Sha256::digest(v.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
