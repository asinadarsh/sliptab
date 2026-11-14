//! Saved posts: a note with a `Link` is a save. The link holds the cleaned URL and, once fetched,
//! what the page says about itself. The user-set fields (`folder`, `done`) merge like pin.

/// What a saved post points at. Every field except `url` and `platform` is optional, so a save
/// made offline or whose page could not be read still works.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Link {
    /// Cleaned: tracking parameters removed, host normalised, so the same post saved twice matches.
    pub url: String,
    /// "instagram", "threads", "x", "youtube" or "web", derived from `url`.
    pub platform: String,
    /// From the page, filled in by the details fetch (S2). None until fetched.
    pub title: Option<String>,
    pub author: Option<String>,
    pub text: Option<String>,
    pub thumbnail: Option<String>,
    /// When the details were fetched; the side with the later fetch wins when merging.
    pub fetched_at: Option<i64>,
    /// Set by the user or the sorter. None = not sorted yet.
    pub folder: Option<String>,
    /// Watched, read or used: the user marked it done.
    pub done: bool,
    /// Last time the original was opened from Edge.
    pub opened_at: Option<i64>,
    /// Fields from a newer version, kept unchanged (see `Note::extra`).
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

const MAX_URL: usize = 2048;
const MAX_FIELD: usize = 2000;

impl Link {
    /// The first http(s) link in shared text, cleaned. None when the text has no link.
    pub fn from_text(text: &str) -> Option<Link> {
        let start = text.find("https://").or_else(|| text.find("http://"))?;
        let raw = text[start..].split(char::is_whitespace).next()?;
        let raw = raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '>']);
        let url = clean_url(raw)?;
        Some(Link { platform: platform_of(&url).into(), url, ..Link::default() })
    }

    /// Links from sync are untrusted: the URL must be a clean http(s) URL, the platform is
    /// rederived, and text fields are capped. None drops the link (the note stays as a note).
    pub fn sanitize(mut self) -> Option<Link> {
        if self.url.len() > MAX_URL {
            return None;
        }
        self.url = clean_url(&self.url)?;
        self.platform = platform_of(&self.url).into();
        for s in [&mut self.title, &mut self.author, &mut self.text, &mut self.folder].into_iter().flatten() {
            if s.len() > MAX_FIELD {
                let mut end = MAX_FIELD;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
            }
        }
        if self.thumbnail.as_deref().is_some_and(|t| t.len() > MAX_URL || !t.starts_with("https://")) {
            self.thumbnail = None;
        }
        Some(self)
    }
}

/// Lowercases the host, unifies the platforms' alternative hosts and drops tracking parameters
/// and the fragment. None if `raw` is not an http(s) URL with a host.
pub fn clean_url(raw: &str) -> Option<String> {
    let (scheme, rest) = raw.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (host_port, path_query) = match rest.find(['/', '?']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let mut host = host_port.to_ascii_lowercase();
    if host.is_empty() || host.contains('@') || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) {
        return None; // userinfo or odd characters: not something we want to open later
    }
    let mut path = path.to_string();
    let mut params: Vec<(&str, &str)> =
        query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).collect();

    match host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("mobile.") {
        "twitter.com" | "x.com" => {
            host = "x.com".into();
            params.clear(); // s, t, a: share tracking only
        }
        "instagram.com" => {
            host = "www.instagram.com".into();
            params.clear(); // igsh, vrfl, utm_*
            if let Some(p) = path.strip_prefix("/reels/") {
                path = format!("/reel/{p}");
            }
        }
        "threads.net" | "threads.com" => {
            host = "www.threads.com".into();
            params.clear(); // xmt, slof, igshid
        }
        "youtu.be" => {
            let id = path.trim_matches('/').to_string();
            if id.is_empty() {
                return None;
            }
            let t = params.iter().find(|(k, _)| *k == "t").map(|(_, v)| v.to_string());
            return Some(match t {
                Some(t) => format!("https://www.youtube.com/watch?v={id}&t={t}"),
                None => format!("https://www.youtube.com/watch?v={id}"),
            });
        }
        "youtube.com" => {
            host = "www.youtube.com".into();
            params.retain(|(k, _)| matches!(*k, "v" | "t" | "list")); // drops si, feature, pp
        }
        _ => params.retain(|(k, _)| !k.starts_with("utm_") && !matches!(*k, "fbclid" | "gclid" | "igsh" | "igshid" | "mc_eid")),
    }
    // A trailing slash is noise on the social platforms; keep the path as is elsewhere.
    if path.len() > 1 && path.ends_with('/') && platform_of_host(&host) != "web" {
        path.pop();
    }
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    let query: Vec<String> = params.iter().map(|(k, v)| if v.is_empty() { k.to_string() } else { format!("{k}={v}") }).collect();
    let q = if query.is_empty() { String::new() } else { format!("?{}", query.join("&")) };
    Some(format!("{}://{host}{path}{q}", scheme.to_ascii_lowercase()))
}

pub fn platform_of(url: &str) -> &'static str {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?']).next().unwrap_or("");
    platform_of_host(&host.to_ascii_lowercase())
}

fn platform_of_host(host: &str) -> &'static str {
    match host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("mobile.") {
        "instagram.com" => "instagram",
        "threads.net" | "threads.com" => "threads",
        "x.com" | "twitter.com" => "x",
        "youtube.com" | "youtu.be" => "youtube",
        _ => "web",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> String {
        Link::from_text(s).map(|l| format!("{} {}", l.platform, l.url)).unwrap_or_default()
    }

    #[test]
    fn real_shares_are_cleaned() {
        // Links Asin shared from the apps on 2026-11-14, and the YouTube forms.
        assert_eq!(
            clean("https://www.instagram.com/reel/DftSYUwtKNT/?vrfl=MWswMXN0YzAxc25nOA=="),
            "instagram https://www.instagram.com/reel/DftSYUwtKNT"
        );
        assert_eq!(
            clean("https://x.com/b4dgerr/status/2121498943662023026?a=8reIdJXuleLrNyZ4HhQ&s=19"),
            "x https://x.com/b4dgerr/status/2121498943662023026"
        );
        assert_eq!(clean("https://www.threads.com/share/_7fwIRJAV"), "threads https://www.threads.com/share/_7fwIRJAV");
        assert_eq!(
            clean("Watch this https://youtu.be/hdMszZ2ah5Q?si=abc123&t=42 !"),
            "youtube https://www.youtube.com/watch?v=hdMszZ2ah5Q&t=42"
        );
        assert_eq!(
            clean("https://m.youtube.com/watch?v=hdMszZ2ah5Q&feature=share&pp=x"),
            "youtube https://www.youtube.com/watch?v=hdMszZ2ah5Q"
        );
        assert_eq!(clean("https://youtube.com/shorts/ai1gIWacf0I?si=Q"), "youtube https://www.youtube.com/shorts/ai1gIWacf0I");
    }

    #[test]
    fn alternative_hosts_match_one_url() {
        let a = clean("https://twitter.com/jack/status/20?s=20");
        assert_eq!(a, clean("https://mobile.twitter.com/jack/status/20/"));
        assert_eq!(a, "x https://x.com/jack/status/20");
        assert_eq!(
            clean("https://threads.net/@zuck/post/DMF6uUgx9f9?xmt=1&slof=1"),
            "threads https://www.threads.com/@zuck/post/DMF6uUgx9f9"
        );
        assert_eq!(clean("https://instagram.com/reels/Cxvwz76vBus"), "instagram https://www.instagram.com/reel/Cxvwz76vBus");
    }

    #[test]
    fn other_sites_keep_their_query_but_lose_tracking() {
        assert_eq!(
            clean("see (https://Example.com/a/b/?id=7&utm_source=x&fbclid=y#top)."),
            "web https://example.com/a/b/?id=7"
        );
    }

    #[test]
    fn not_links() {
        for s in ["just a note", "ftp://x.com/a", "https://", "https://user:pw@evil.com/", "javascript:alert(1)"] {
            assert_eq!(clean(s), "", "{s}");
        }
    }

    #[test]
    fn sanitize_rederives_and_caps() {
        let l = Link {
            url: "https://twitter.com/a/status/1?s=9".into(),
            platform: "web".into(),
            title: Some("é".repeat(1500)),
            thumbnail: Some("http://insecure/img.jpg".into()),
            ..Link::default()
        }
        .sanitize()
        .unwrap();
        assert_eq!((l.url.as_str(), l.platform.as_str()), ("https://x.com/a/status/1", "x"));
        assert!(l.title.unwrap().len() <= MAX_FIELD);
        assert_eq!(l.thumbnail, None);
        assert_eq!(Link { url: "file:///etc/passwd".into(), ..Link::default() }.sanitize(), None);
    }
}
