//! Sync rules shared by the apps and the server: what a device sends, what it gets back, and
//! how two edits of the same note are merged. The server is the authority on order: it gives
//! every stored change a sequence number, and a device asks for "everything after my cursor",
//! so a phone with a wrong clock can never skip notes.
//!
//! Merging works per field. A change carries the state it started from (`Base`): its version
//! and a hash of its body. So "pinned on the laptop, edited on the phone" merges cleanly, and a
//! conflict copy is made only when both sides changed the body to different text.

use uuid::Uuid;

use crate::link::Link;
use crate::note::{derive_title, Note};

/// The server state a device last saw for a note. None for a note made on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Base {
    pub version: i64,
    pub body_hash: u64,
    /// Hash of pinned, archived and colour. None from an older client: the later edit wins.
    #[cfg_attr(feature = "serde", serde(default))]
    pub meta_hash: Option<u64>,
}

impl Base {
    pub fn of(n: &Note) -> Self {
        Base { version: n.version, body_hash: body_hash(&n.body), meta_hash: Some(meta_hash(n)) }
    }
}

/// One note a device changed, with the server state it started from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Change {
    pub note: Note,
    pub base: Option<Base>,
}

/// `POST /v1/sync` body: my changes, and the last server sequence number I have seen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyncRequest {
    pub cursor: i64,
    pub changes: Vec<Change>,
}

/// Every note stored after `cursor` (including the ones just sent, as the server merged them,
/// and tombstones of deleted notes) and the new cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SyncResponse {
    pub cursor: i64,
    pub notes: Vec<Note>,
    /// The server sent a page, not everything: sync again straight away.
    #[cfg_attr(feature = "serde", serde(default))]
    pub more: bool,
}

/// Result of merging two versions of one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub note: Note,
    /// The losing body, kept as a new note when both sides rewrote the text.
    pub copy: Option<Note>,
}

/// FNV-1a: stable across Rust versions and platforms, unlike `std`'s hasher, so a hash made on a
/// phone matches one made on the server.
pub fn body_hash(body: &str) -> u64 {
    body.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The fields besides the text that a user sets: pin, archive and tab colour, and for a save its
/// folder and done mark. A note without a link hashes exactly as before saves existed.
pub fn meta_hash(n: &Note) -> u64 {
    let mut s = format!("{}|{}|{}", n.pinned, n.archived, n.color.as_deref().unwrap_or(""));
    if let Some(l) = &n.link {
        s += &format!("|{}|{}", l.folder.as_deref().unwrap_or(""), l.done);
    }
    body_hash(&s)
}

/// Merges `mine` and `theirs`, two versions of the same note that both started from `base`.
/// The result is the same whichever side calls it (argument order only matters for which
/// state is "mine" relative to `base`), so device and server agree.
pub fn merge(base: Option<Base>, mine: &Note, theirs: &Note) -> Merged {
    debug_assert_eq!(mine.id, theirs.id);
    if let Some(b) = base {
        if theirs.version == b.version && body_hash(&theirs.body) == b.body_hash {
            return Merged { note: mine.clone(), copy: None }; // only I changed it
        }
        if mine.version == b.version && body_hash(&mine.body) == b.body_hash {
            return Merged { note: theirs.clone(), copy: None }; // only they changed it
        }
    }
    if same_content(mine, theirs) {
        return Merged { note: later(mine, theirs).clone(), copy: None };
    }

    // Both changed. Pin, archive and colour come from the side that changed them; if both did,
    // the later edit wins.
    let newer = later(mine, theirs);
    let older = if std::ptr::eq(newer, mine) { theirs } else { mine };
    let mut note = newer.clone();
    note.version = mine.version.max(theirs.version) + 1;
    let base_meta = base.and_then(|b| b.meta_hash);
    let meta_from = match (base_meta != Some(meta_hash(mine)), base_meta != Some(meta_hash(theirs))) {
        (true, false) => mine,
        (false, true) => theirs,
        _ => newer,
    };
    (note.pinned, note.archived, note.color) = (meta_from.pinned, meta_from.archived, meta_from.color.clone());
    note.link = merge_link(mine.link.as_ref(), theirs.link.as_ref(), meta_from.link.as_ref());

    let base_hash = base.map(|b| b.body_hash);
    let mine_changed = base_hash != Some(body_hash(&mine.body));
    let theirs_changed = base_hash != Some(body_hash(&theirs.body));
    let mut copy = None;
    if mine.body != theirs.body {
        if !theirs_changed {
            note.body = mine.body.clone();
        } else if !mine_changed {
            note.body = theirs.body.clone();
        } else {
            // Both rewrote the text: the newer text stays, the older becomes a conflict copy.
            note.body = newer.body.clone();
            copy = Some(conflict_copy(older, note.id));
        }
    }
    note.title = derive_title(&note.body);

    // An edit beats a delete, so nobody loses text they just wrote.
    let edited_body = |n: &Note| base_hash != Some(body_hash(&n.body)) && n.deleted_at.is_none();
    if note.deleted_at.is_some() && (edited_body(mine) || edited_body(theirs)) {
        note.deleted_at = None;
    }
    Merged { note, copy }
}

/// A save's link: the fetched details from the later fetch, folder and done from `meta_from`
/// (the side that changed them), and the latest open. A link only one side has is kept.
fn merge_link(a: Option<&Link>, b: Option<&Link>, meta_from: Option<&Link>) -> Option<Link> {
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        (one, other) => return one.or(other).cloned(),
    };
    let mut l = if b.fetched_at > a.fetched_at { b.clone() } else { a.clone() };
    if let Some(m) = meta_from {
        (l.folder, l.done) = (m.folder.clone(), m.done);
    }
    l.opened_at = a.opened_at.max(b.opened_at);
    Some(l)
}

/// Later `updated_at` wins; on a tie the larger device id, so both sides pick the same one.
fn later<'a>(a: &'a Note, b: &'a Note) -> &'a Note {
    if (b.updated_at, &b.device_id) > (a.updated_at, &a.device_id) {
        b
    } else {
        a
    }
}

fn same_content(a: &Note, b: &Note) -> bool {
    (&a.body, a.pinned, a.archived, a.deleted_at.is_some(), &a.color, &a.link)
        == (&b.body, b.pinned, b.archived, b.deleted_at.is_some(), &b.color, &b.link)
}

fn conflict_copy(loser: &Note, of: Uuid) -> Note {
    Note {
        id: Uuid::now_v7(),
        title: derive_title(&loser.body),
        pinned: false,
        deleted_at: None,
        version: 1,
        conflict_of: Some(of),
        ..loser.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(body: &str, version: i64, updated_at: i64, device: &str) -> Note {
        Note {
            id: Uuid::nil(),
            title: derive_title(body),
            body: body.into(),
            pinned: false,
            archived: false,
            deleted_at: None,
            created_at: 0,
            updated_at,
            device_id: device.into(),
            version,
            conflict_of: None,
            color: None,
            link: None,
        }
    }

    #[test]
    fn hash_is_stable() {
        // Fixed value: if this changes, devices and server disagree about every note.
        assert_eq!(body_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(body_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn one_sided_changes_win_without_a_copy() {
        let base = note("milk", 3, 10, "phone");
        let mine = note("milk and eggs", 4, 20, "phone");
        let m = merge(Some(Base::of(&base)), &mine, &base);
        assert_eq!((m.note, m.copy), (mine.clone(), None));
        let m = merge(Some(Base::of(&base)), &base, &mine);
        assert_eq!((m.note, m.copy), (mine, None));
    }

    #[test]
    fn pin_on_one_device_and_edit_on_the_other_both_survive() {
        let base = note("milk", 3, 10, "phone");
        let mut pinned = base.clone();
        pinned.pinned = true;
        pinned.color = Some("#3b82f6".into());
        pinned.version = 4;
        pinned.updated_at = 30;
        pinned.device_id = "laptop".into();
        let edited = note("milk and eggs", 4, 20, "phone");
        for (a, b) in [(&pinned, &edited), (&edited, &pinned)] {
            let m = merge(Some(Base::of(&base)), a, b);
            assert!(m.copy.is_none());
            assert_eq!(m.note.body, "milk and eggs");
            assert!(m.note.pinned);
            assert_eq!(m.note.version, 5);
        }
    }

    #[test]
    fn an_earlier_pin_survives_a_later_text_edit() {
        let base = note("milk", 3, 10, "phone");
        let mut pinned = base.clone();
        pinned.pinned = true;
        pinned.color = Some("#3b82f6".into());
        pinned.version = 4;
        pinned.updated_at = 15; // pinned first...
        pinned.device_id = "laptop".into();
        let edited = note("milk and eggs", 4, 20, "phone"); // ...text edited later
        for (a, b) in [(&pinned, &edited), (&edited, &pinned)] {
            let m = merge(Some(Base::of(&base)), a, b);
            assert_eq!((m.note.body.as_str(), m.note.pinned), ("milk and eggs", true));
            assert_eq!(m.note.color.as_deref(), Some("#3b82f6"));
        }
    }

    #[test]
    fn both_rewrote_the_text_newer_stays_older_is_copied() {
        let base = note("milk", 3, 10, "phone");
        let phone = note("milk and eggs", 4, 20, "phone");
        let laptop = note("oat milk", 4, 25, "laptop");
        let m = merge(Some(Base::of(&base)), &phone, &laptop);
        assert_eq!(m.note.body, "oat milk");
        let copy = m.copy.unwrap();
        assert_eq!(copy.body, "milk and eggs");
        assert_eq!(copy.conflict_of, Some(m.note.id));
        assert_ne!(copy.id, m.note.id);
        // Same winner from the other side.
        assert_eq!(merge(Some(Base::of(&base)), &laptop, &phone).note.body, "oat milk");
    }

    #[test]
    fn an_edit_beats_a_delete() {
        let base = note("milk", 3, 10, "phone");
        let mut deleted = base.clone();
        deleted.deleted_at = Some(30);
        deleted.version = 4;
        deleted.updated_at = 30;
        let edited = note("milk and eggs", 4, 20, "laptop");
        let m = merge(Some(Base::of(&base)), &deleted, &edited);
        assert_eq!(m.note.deleted_at, None);
        assert_eq!(m.note.body, "milk and eggs");
        assert!(m.copy.is_none());
        // A delete with no competing text edit stays deleted.
        let mut pinned = base.clone();
        pinned.pinned = true;
        pinned.version = 4;
        pinned.updated_at = 20;
        assert!(merge(Some(Base::of(&base)), &deleted, &pinned).note.deleted_at.is_some());
    }

    #[test]
    fn identical_edits_merge_quietly() {
        let base = note("milk", 3, 10, "phone");
        let a = note("eggs", 4, 20, "phone");
        let b = note("eggs", 4, 25, "laptop");
        let m = merge(Some(Base::of(&base)), &a, &b);
        assert_eq!((m.note.device_id.as_str(), m.copy), ("laptop", None));
    }
}
