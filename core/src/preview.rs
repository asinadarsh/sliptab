//! What a saved post's page says about itself: title, author, text, thumbnail and the post's own
//! address. Pure parsing, no network, so it is tested against real pages saved in
//! `tests/fixtures`. Fetching lives in `client`.
//!
//! What each platform gives a logged-out preview fetch (S0 test, 2026-11-14):
//! - X, Threads: the full text in `og:description`, the author in `og:title`
//!   ("Name (@handle) on X"), the post address in `og:url` (Threads short links redirect to it).
//! - Instagram: the caption inside `og:title` (`Name on Instagram: "caption"`), the handle in
//!   `twitter:title`. Deleted or private posts give no tags at all.
//! - YouTube: its oEmbed JSON (title, channel, thumbnail).
//! - Other sites: `og:*` tags, else `<title>` and the meta description.

use std::collections::HashMap;

/// The details found for one save. Empty fields mean the page did not say.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    pub title: Option<String>,
    pub author: Option<String>,
    pub text: Option<String>,
    pub thumbnail: Option<String>,
    /// The post's own address when the page names it (e.g. where a Threads short link leads).
    pub canonical: Option<String>,
}

impl Preview {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.author.is_none() && self.text.is_none() && self.thumbnail.is_none()
    }
}

const MAX_TITLE: usize = 160;
const MAX_TEXT: usize = 1500;

/// Reads a page's preview tags. `final_url` is where any redirects ended.
pub fn parse_html(platform: &str, final_url: &str, html: &str) -> Preview {
    let tags = meta_tags(html);
    let get = |k: &str| tags.get(k).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_owned);
    let page_title = title_tag(html);
    let thumbnail = get("og:image").or_else(|| get("twitter:image")).filter(|u| u.starts_with("https://"));
    // rel=canonical first: Instagram's og:url adds the username (/user/reel/id), while its
    // canonical is the /reel/id form that shares use, so a repeated share still matches.
    let canonical = canonical_link(html)
        .or_else(|| get("og:url"))
        .or_else(|| (final_url.starts_with("http")).then(|| final_url.to_owned()));

    let mut p = match platform {
        "x" | "threads" => {
            let text = get("og:description").or_else(|| get("description"));
            let author = get("og:title").and_then(|t| handle_in(&t)).or_else(|| get("twitter:creator"));
            Preview { title: text.as_deref().map(first_line), text, author, ..Preview::default() }
        }
        "instagram" => {
            // og:title: `Saakshi Gondaliya on Instagram: "#chess #chessplayer"` (the caption, cut).
            // description: `<likes> - handle on <date>: "<caption>"` or `<caption> - @handle on <date>`.
            let caption = get("og:title")
                .and_then(|t| t.split_once(" on Instagram: ").map(|(_, c)| c.trim().trim_matches('"').to_owned()))
                .filter(|c| !c.is_empty())
                .or_else(|| get("description").and_then(|d| instagram_caption(&d)));
            let tt = get("twitter:title"); // "Hanjla Ali (@info.byhanzla) • Instagram reel"
            let author = tt.as_deref().and_then(handle_in);
            // No caption: name the post after its kind and author, so the card isn't a bare link.
            let fallback = tt.as_deref().and_then(|t| {
                let (who, kind) = t.split_once(" • ")?;
                Some(format!("{kind} by {}", who.split(" (@").next().unwrap_or(who).trim()))
            });
            Preview { title: caption.as_deref().map(first_line).or(fallback), text: caption, author, ..Preview::default() }
        }
        _ => {
            let title = get("og:title").or_else(|| get("twitter:title")).or(page_title);
            let text = get("og:description").or_else(|| get("description"));
            let author = get("og:site_name").or_else(|| get("author")).filter(|a| !a.starts_with("http"));
            Preview { title, text, author, ..Preview::default() }
        }
    };
    p.thumbnail = thumbnail;
    p.canonical = canonical;
    p.title = p.title.map(|t| cap(&t, MAX_TITLE));
    p.text = p.text.map(|t| cap(&t, MAX_TEXT));
    if p.is_empty() {
        p.canonical = None; // an empty page (deleted post, login wall) proves nothing about the address
    }
    p
}

/// Reads YouTube's oEmbed answer.
pub fn parse_youtube_oembed(json: &str) -> Preview {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let s = |k: &str| v[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    Preview {
        title: s("title").map(|t| cap(&t, MAX_TITLE)),
        author: s("author_name"),
        text: None,
        thumbnail: s("thumbnail_url").filter(|u| u.starts_with("https://")),
        canonical: None,
    }
}

/// "Nick Saraev (@nick_saraev) on Threads" → "@nick_saraev".
fn handle_in(s: &str) -> Option<String> {
    let start = s.find("(@")?;
    let end = s[start..].find(')')? + start;
    Some(s[start + 1..end].to_owned())
}

/// `154K likes, 1,718 comments - christina.codes on September 28, 2023: "caption"` → caption;
/// `caption - @handle on <date>` → caption.
fn instagram_caption(d: &str) -> Option<String> {
    if let Some(i) = d.find(": \"") {
        return Some(d[i + 3..].trim_end_matches(['"', '.', ' ']).to_owned()).filter(|c| !c.is_empty());
    }
    d.rsplit_once(" - @").map(|(c, _)| c.trim().to_owned()).filter(|c| !c.is_empty())
}

fn first_line(s: &str) -> String {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    cap(line, 100)
}

fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// `<meta property|name="k" content="v">` pairs, keys lowercased, values unescaped. The first
/// value for a key wins. Attribute order and quote style vary between sites, and values may span
/// lines.
fn meta_tags(html: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta") {
        let start = at + i;
        let Some(len) = html[start..].find('>') else { break };
        let tag = &html[start..start + len];
        at = start + len;
        let attrs = attributes(tag);
        let key = attrs.get("property").or_else(|| attrs.get("name")).or_else(|| attrs.get("itemprop"));
        if let (Some(k), Some(v)) = (key, attrs.get("content")) {
            out.entry(k.to_ascii_lowercase()).or_insert_with(|| unescape(v));
        }
    }
    out
}

fn canonical_link(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<link") {
        let start = at + i;
        let len = html[start..].find('>')?;
        let attrs = attributes(&html[start..start + len]);
        at = start + len;
        if attrs.get("rel").is_some_and(|r| r.eq_ignore_ascii_case("canonical")) {
            return attrs.get("href").map(|h| unescape(h));
        }
    }
    None
}

/// The first non-generic `<title>` (Instagram's pages start with three that say "Instagram").
fn title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<title") {
        let open = at + i;
        let body = open + html[open..].find('>')? + 1;
        let end = body + lower[body..].find("</title>")?;
        at = end;
        let t = unescape(html[body..end].trim());
        if !t.is_empty() && !matches!(t.as_str(), "Instagram" | "X" | "Threads" | "YouTube") {
            return Some(t);
        }
    }
    None
}

/// `k="v"`, `k='v'` and `k=v` attributes of one tag, keys lowercased.
fn attributes(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let b = tag.as_bytes();
    let mut i = tag.find(char::is_whitespace).unwrap_or(tag.len());
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let k0 = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = tag[k0..i].to_ascii_lowercase();
        if i >= b.len() || b[i] != b'=' {
            if key.is_empty() {
                i += 1;
            }
            continue;
        }
        i += 1;
        let value = match b.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let end = tag[i + 1..].find(q as char).map_or(tag.len(), |e| i + 1 + e);
                let v = &tag[i + 1..end];
                i = end + 1;
                v
            }
            _ => {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                &tag[v0..i]
            }
        };
        out.entry(key).or_insert_with(|| value.to_owned());
    }
    out
}

/// HTML character references: named ones pages actually use, and numeric ones.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
            _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn instagram_reel_gives_caption_handle_and_image() {
        let p = parse_html("instagram", "https://www.instagram.com/reel/DftSYUwtKNT/", &fixture("instagram_reel.html"));
        assert_eq!(p.title.as_deref(), Some("#chess #chessplayer #explore #reels #fyp"));
        assert_eq!(p.author.as_deref(), Some("@saakshi.chess"));
        assert!(p.thumbnail.unwrap().starts_with("https://scontent.cdninstagram.com/"));
        assert_eq!(p.canonical.as_deref(), Some("https://www.instagram.com/reel/DftSYUwtKNT/"));
    }

    #[test]
    fn instagram_post_caption_keeps_its_quotes_and_apostrophes() {
        let p = parse_html("instagram", "", &fixture("instagram_post.html"));
        let t = p.text.unwrap();
        assert!(t.starts_with("While updating an investor today I used the phrase \"shopping for unicorn saddles.\""), "{t}");
        assert!(t.contains("here's what"));
        assert_eq!(p.author.as_deref(), Some("@aarondinin"));
    }

    #[test]
    fn an_instagram_reel_without_a_caption_is_named_by_kind_and_author() {
        let html = r#"<meta name="twitter:title" content="Hanjla Ali (&#064;info.byhanzla) &#x2022; Instagram reel" />
<meta name="description" content="8,197 likes, 14 comments - info.byhanzla on November 13, 2026" />
<meta property="og:title" content="Hanjla Ali on Instagram" />"#;
        let p = parse_html("instagram", "", html);
        assert_eq!((p.title.as_deref(), p.text, p.author.as_deref()), (Some("Instagram reel by Hanjla Ali"), None, Some("@info.byhanzla")));
    }

    #[test]
    fn a_deleted_instagram_post_gives_nothing() {
        let p = parse_html("instagram", "https://www.instagram.com/p/BdA94kVgQhU/", &fixture("instagram_gone.html"));
        assert!(p.is_empty());
        assert_eq!(p.canonical, None);
    }

    #[test]
    fn x_post_gives_the_whole_text_and_handle() {
        let p = parse_html("x", "", &fixture("x_post.html"));
        assert_eq!(p.title.as_deref(), Some("Can I ask a dumb question?"));
        assert!(p.text.unwrap().contains("why are we sacrificing our best years trying to get rich?"));
        assert_eq!(p.author.as_deref(), Some("@b4dgerr"));
        assert_eq!(p.canonical.as_deref(), Some("https://x.com/b4dgerr/status/2121498943662023026"));
    }

    #[test]
    fn threads_short_link_resolves_to_the_post() {
        let p = parse_html("threads", "https://www.threads.com/@nick_saraev/post/Dfui2UBE4Sv?xmt=1", &fixture("threads_share.html"));
        assert_eq!(p.title.as_deref(), Some("Meta just open sourced the design system behind over 13K of its apps."));
        assert_eq!(p.author.as_deref(), Some("@nick_saraev"));
        assert_eq!(p.canonical.as_deref(), Some("https://www.threads.com/@nick_saraev/post/Dfui2UBE4Sv"));
    }

    #[test]
    fn an_article_gives_title_description_and_image() {
        let p = parse_html("web", "", &fixture("web_article.html"));
        assert_eq!(p.title.as_deref(), Some("Top 10 Pocket Alternatives in 2026"));
        assert!(p.text.unwrap().starts_with("Discover the best Pocket alternatives"));
        assert!(p.thumbnail.unwrap().starts_with("https://storage.ghost.io/"));
    }

    #[test]
    fn youtube_oembed() {
        let p = parse_youtube_oembed(&fixture("youtube_oembed.json"));
        assert_eq!(p.title.as_deref(), Some("Finally Karma Caught Up to Abhi & Niyu? 😨"));
        assert_eq!(p.author.as_deref(), Some("Rameshth Neekhra"));
        assert_eq!(p.thumbnail.as_deref(), Some("https://i.ytimg.com/vi/gE6EZSLnFtI/hqdefault.jpg"));
        assert!(parse_youtube_oembed("Not Found").is_empty());
    }

    #[test]
    fn markup_variations() {
        let html = "<META content='a &amp; b &#x2022; c' property=og:title><meta name=description content=\"x\ny\">";
        let p = parse_html("web", "", html);
        assert_eq!(p.title.as_deref(), Some("a & b • c"));
        assert_eq!(p.text.as_deref(), Some("x\ny"));
        assert_eq!(unescape("5 &lt 6 &bogus; &#128512;"), "5 &lt 6 &bogus; 😀");
        assert_eq!(cap(&"é".repeat(200), 10).chars().count(), 10);
    }
}
