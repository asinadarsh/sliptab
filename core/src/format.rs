//! Formatting inside a note body: `**bold**`, `*italic*`, `__underline__`, `~~strike~~`, and
//! links found automatically. The body stays plain text; this only reads it. The desktop
//! (format.ts) and phone (format.dart) implement the same rules, all checked against
//! tests/fixtures/format.json.

/// One run of text with the same style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// A formatting marker (`**`, `__` ...), shown faintly by editors and left out of `plain`.
    pub marker: bool,
    /// Set when the text is a link: what to open.
    pub href: Option<String>,
}

#[derive(Clone, Copy)]
enum Style {
    Bold,
    Italic,
    Underline,
    Strike,
}

const MARKERS: [(&str, Style); 4] = [
    ("**", Style::Bold),
    ("__", Style::Underline),
    ("~~", Style::Strike),
    ("*", Style::Italic),
];

fn is_trailing(c: char) -> bool {
    ".,;:!?)]'\"*_~".contains(c)
}

/// Links in one line as char ranges with their href. Trailing punctuation and markers are not
/// part of a link.
fn links(line: &[char]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    let starts = |i: usize, p: &str| {
        p.chars()
            .enumerate()
            .all(|(d, c)| line.get(i + d) == Some(&c))
    };
    while i < line.len() {
        let word_start = i == 0 || !line[i - 1].is_alphanumeric();
        if word_start && (starts(i, "https://") || starts(i, "http://") || starts(i, "www.")) {
            let mut e = i;
            while e < line.len() && !line[e].is_whitespace() && !"<>\"".contains(line[e]) {
                e += 1;
            }
            while e > i && is_trailing(line[e - 1]) {
                e -= 1;
            }
            let text: String = line[i..e].iter().collect();
            let prefix_only = matches!(text.as_str(), "https://" | "http://" | "www.");
            if e > i && !prefix_only {
                let href = if text.starts_with("www.") {
                    format!("https://{text}")
                } else {
                    text
                };
                out.push((i, e, href));
                i = e;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[derive(Clone, Default, PartialEq)]
struct Flags {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

fn spans_of_line(line: &[char], out: &mut Vec<Span>) {
    let n = line.len();
    let mut style = vec![Flags::default(); n];
    let mut marker = vec![false; n];
    let mut href: Vec<Option<String>> = vec![None; n];
    for (s, e, h) in links(line) {
        for slot in &mut href[s..e] {
            *slot = Some(h.clone());
        }
    }
    for (mk, st) in MARKERS {
        let mk: Vec<char> = mk.chars().collect();
        let k = mk.len();
        let at = |i: usize, marker: &[bool]| {
            i + k <= n
                && line[i..i + k] == mk[..]
                && (k > 1 || ((i == 0 || line[i - 1] != '*') && line.get(i + 1) != Some(&'*')))
                && (i..i + k).all(|d| href[d].is_none() && !marker[d])
        };
        let mut i = 0;
        while i < n {
            let opens = at(i, &marker)
                && line.get(i + k).is_some_and(|c| !c.is_whitespace())
                && (i == 0 || line[i - 1] != mk[0]);
            if opens {
                let mut j = i + k + 1;
                while j < n
                    && !(at(j, &marker)
                        && !line[j - 1].is_whitespace()
                        && line.get(j + k) != Some(&mk[0]))
                {
                    j += 1;
                }
                if j < n {
                    for d in 0..k {
                        marker[i + d] = true;
                        marker[j + d] = true;
                    }
                    for f in &mut style[i + k..j] {
                        match st {
                            Style::Bold => f.bold = true,
                            Style::Italic => f.italic = true,
                            Style::Underline => f.underline = true,
                            Style::Strike => f.strike = true,
                        }
                    }
                    i = j + k;
                    continue;
                }
            }
            i += 1;
        }
    }
    for i in 0..n {
        let f = &style[i];
        let same = out.last().is_some_and(|l: &Span| {
            (l.bold, l.italic, l.underline, l.strike, l.marker, &l.href)
                == (f.bold, f.italic, f.underline, f.strike, marker[i], &href[i])
        });
        if same {
            out.last_mut().expect("checked").text.push(line[i]);
        } else {
            out.push(Span {
                text: line[i].to_string(),
                bold: f.bold,
                italic: f.italic,
                underline: f.underline,
                strike: f.strike,
                marker: marker[i],
                href: href[i].clone(),
            });
        }
    }
}

/// Spans for a whole body. Formatting never crosses a line break.
pub fn format(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            match out.last_mut() {
                Some(l)
                    if *l
                        == Span {
                            text: l.text.clone(),
                            ..Span::default()
                        } =>
                {
                    l.text.push('\n')
                }
                _ => out.push(Span {
                    text: "\n".into(),
                    ..Span::default()
                }),
            }
        }
        let chars: Vec<char> = line.chars().collect();
        let mut line_spans = Vec::new();
        spans_of_line(&chars, &mut line_spans);
        let style = |s: &Span| {
            (
                s.bold,
                s.italic,
                s.underline,
                s.strike,
                s.marker,
                s.href.clone(),
            )
        };
        for s in line_spans {
            if out.last().is_some_and(|l| style(l) == style(&s)) {
                out.last_mut().expect("checked").text += &s.text;
            } else {
                out.push(s);
            }
        }
    }
    out
}

/// The text without formatting markers, for titles, previews and search snippets.
pub fn plain(text: &str) -> String {
    if !text.contains(['*', '_', '~']) {
        return text.to_owned(); // the common case, no allocation of spans
    }
    format(text)
        .into_iter()
        .filter(|s| !s.marker)
        .map(|s| s.text)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &Span) -> String {
        let mut c: Vec<char> = [
            (s.bold, 'b'),
            (s.italic, 'i'),
            (s.underline, 'u'),
            (s.strike, 's'),
            (s.href.is_some(), 'l'),
            (s.marker, 'm'),
        ]
        .into_iter()
        .filter_map(|(on, c)| on.then_some(c))
        .collect();
        c.sort();
        c.into_iter().collect()
    }

    #[test]
    fn shared_vectors() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/format.json"
        ))
        .unwrap();
        let cases: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
        for c in cases {
            let input = c["in"].as_str().unwrap();
            let got: Vec<(String, String)> = format(input)
                .iter()
                .map(|s| (s.text.clone(), code(s)))
                .collect();
            let want: Vec<(String, String)> = c["out"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    let mut k: Vec<char> = p[1].as_str().unwrap().chars().collect();
                    k.sort();
                    (p[0].as_str().unwrap().to_owned(), k.into_iter().collect())
                })
                .collect();
            assert_eq!(got, want, "{input:?}");
        }
    }

    #[test]
    fn plain_drops_markers_only() {
        assert_eq!(
            plain("a **bold** and __u__ https://x.io"),
            "a bold and u https://x.io"
        );
        assert_eq!(plain("snake_case stays"), "snake_case stays");
        assert_eq!(
            format("go www.a.io")
                .iter()
                .find_map(|s| s.href.clone())
                .as_deref(),
            Some("https://www.a.io")
        );
    }
}
