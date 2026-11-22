//! Images, audio and other files attached to a note. The note carries only their description; the
//! bytes live in a file named by their SHA-256, on each device and on the sync server, so the same
//! file attached twice is stored once and a description can be checked against its bytes.

use uuid::Uuid;

/// Largest attachment accepted, in bytes (Asin's limit, 2026-11-21).
pub const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;
/// Attachments per note, deleted ones included, so a buggy or hostile device can't grow a note
/// without bound.
pub const MAX_ATTACHMENTS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    /// UUIDv7, made on the device that attached it.
    pub id: Uuid,
    /// "image", "audio" or "file" (anything else: a PDF, a document).
    pub kind: String,
    /// e.g. "image/jpeg", "audio/mp4".
    pub mime: String,
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the file: its name in storage.
    pub sha256: String,
    /// The original file name, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub created_at: i64,
    /// Removed from the note. Kept as a tombstone so the removal syncs; the file can be dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
}

impl Attachment {
    /// Checks an attachment from an untrusted device; None drops it.
    pub fn sanitize(mut self) -> Option<Self> {
        let hex = self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let mime_ok = self.mime.len() <= 100
            && self.mime.split('/').count() == 2
            && (self.kind == "file" || self.mime.starts_with(&format!("{}/", self.kind)))
            && self
                .mime
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/.+-".contains(&b));
        if !matches!(self.kind.as_str(), "image" | "audio" | "file")
            || !hex
            || !mime_ok
            || self.bytes > MAX_ATTACHMENT_BYTES
        {
            return None;
        }
        if let Some(n) = &self.name {
            self.name = Some(n.chars().filter(|c| !c.is_control()).take(200).collect());
        }
        Some(self)
    }

    pub fn is_live(&self) -> bool {
        self.deleted_at.is_none()
    }
}

/// Every attachment either side has, by id. When both have one, a removal wins (it can only be
/// removed, never edited), then the earlier removal time so both sides agree. Ordered by when it
/// was attached, so the result does not depend on which side calls it.
// ponytail: fixed order by attach time; drag-to-reorder needs an order field merged like pins.
pub fn merge(a: &[Attachment], b: &[Attachment]) -> Vec<Attachment> {
    let mut out: Vec<Attachment> = a.to_vec();
    for x in b {
        match out.iter_mut().find(|y| y.id == x.id) {
            None => out.push(x.clone()),
            Some(y) => {
                y.deleted_at = match (y.deleted_at, x.deleted_at) {
                    (Some(p), Some(q)) => Some(p.min(q)),
                    (p, q) => p.or(q),
                }
            }
        }
    }
    out.sort_by_key(|x| (x.created_at, x.id));
    out.truncate(MAX_ATTACHMENTS);
    out
}

/// Copies `bytes` into `media_dir` under their SHA-256 (once; the same file again is a no-op)
/// and returns the name and size. Written to a temp name and renamed, so a crash never leaves
/// a half file under a real name.
#[cfg(feature = "store")]
pub fn store_file(media_dir: &std::path::Path, bytes: &[u8]) -> std::io::Result<(String, u64)> {
    use sha2::Digest;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(std::io::Error::other("Attachments can be up to 50 MB."));
    }
    let sha: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let path = media_dir.join(&sha);
    if !path.exists() {
        std::fs::create_dir_all(media_dir)?;
        let tmp = media_dir.join(format!(".{sha}.{}", Uuid::now_v7().simple()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok((sha, bytes.len() as u64))
}

/// A new attachment for a file already in `media_dir` (see [`store_file`]).
#[allow(clippy::too_many_arguments)]
pub fn new_attachment(
    sha256: String,
    bytes: u64,
    mime: &str,
    name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<u64>,
    now: i64,
) -> Option<Attachment> {
    let mime = if mime.is_empty() {
        "application/octet-stream"
    } else {
        mime
    };
    let kind = mime
        .split('/')
        .next()
        .filter(|k| matches!(*k, "image" | "audio"))
        .unwrap_or("file");
    Attachment {
        id: Uuid::now_v7(),
        kind: kind.to_owned(),
        mime: mime.to_owned(),
        bytes,
        sha256,
        name,
        width,
        height,
        duration_ms,
        created_at: now,
        deleted_at: None,
    }
    .sanitize()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn att(id: u128, at: i64) -> Attachment {
        Attachment {
            id: Uuid::from_u128(id),
            kind: "image".into(),
            mime: "image/jpeg".into(),
            bytes: 10,
            sha256: "a".repeat(64),
            name: None,
            width: None,
            height: None,
            duration_ms: None,
            created_at: at,
            deleted_at: None,
        }
    }

    #[test]
    fn merge_keeps_both_sides_and_removals_win() {
        let (a, b, c) = (att(1, 10), att(2, 20), att(3, 5));
        let mut gone = a.clone();
        gone.deleted_at = Some(99);
        let left = merge(&[a.clone(), b.clone()], &[gone.clone(), c.clone()]);
        let right = merge(&[gone.clone(), c.clone()], &[a.clone(), b.clone()]);
        assert_eq!(left, right, "same result whichever side merges");
        assert_eq!(
            left.iter().map(|x| x.id.as_u128()).collect::<Vec<_>>(),
            [3, 1, 2]
        );
        assert_eq!(left[1].deleted_at, Some(99));
    }

    #[cfg(feature = "store")]
    #[test]
    fn a_file_is_stored_once_under_its_hash() {
        let dir = std::env::temp_dir().join(format!("edge-media-test-{}", Uuid::now_v7()));
        let (a, n) = store_file(&dir, b"hello").unwrap();
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(n, 5);
        assert_eq!(store_file(&dir, b"hello").unwrap().0, a);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no temp files left"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sanitize_drops_bad_ones() {
        assert!(att(1, 1).sanitize().is_some());
        let bad = |f: fn(&mut Attachment)| {
            let mut x = att(1, 1);
            f(&mut x);
            x.sanitize().is_none()
        };
        assert!(bad(|x| x.kind = "video".into()));
        assert!(bad(|x| x.mime = "application".into()));
        let mut pdf = att(1, 1);
        (pdf.kind, pdf.mime) = ("file".into(), "application/pdf".into());
        assert!(pdf.sanitize().is_some(), "any file type is fine as a file");
        let kind = |m: &str| {
            new_attachment("a".repeat(64), 1, m, None, None, None, None, 1)
                .unwrap()
                .kind
        };
        assert_eq!(
            [
                kind("image/png"),
                kind("audio/mpeg"),
                kind("application/zip"),
                kind("")
            ],
            ["image", "audio", "file", "file"]
        );
        assert!(bad(|x| x.sha256 = "../etc/passwd".into()));
        assert!(bad(|x| x.sha256 = "A".repeat(64)));
        assert!(bad(|x| x.mime = "audio/mp4".into())); // does not match kind
        assert!(bad(|x| x.mime = "image/<script>".into()));
        assert!(bad(|x| x.bytes = MAX_ATTACHMENT_BYTES + 1));
    }
}
