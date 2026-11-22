use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::folders::suggest_folder;
use crate::link::{clean_url, Link};
use crate::note::{derive_title, valid_color, Note};
use crate::preview::Preview;
use crate::sync::{body_hash, merge, meta_hash, Base, Change, SyncRequest, SyncResponse};

/// Schema migrations, applied in order. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE notes (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    pinned      INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    deleted_at  INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   TEXT NOT NULL,
    version     INTEGER NOT NULL,
    conflict_of TEXT
);
CREATE INDEX notes_list ON notes (pinned DESC, updated_at DESC);
-- Kept in step with `notes` by the store, inside the same transaction as each write.
CREATE VIRTUAL TABLE notes_fts USING fts5 (note_id UNINDEXED, title, body);
-- One row per note changed since the last successful sync.
CREATE TABLE sync_queue (
    note_id   TEXT PRIMARY KEY,
    queued_at INTEGER NOT NULL
);
"#,
    r#"
ALTER TABLE notes ADD COLUMN color TEXT;
"#,
    r#"
-- The server state this device last saw for each note (NULL: never synced), for merging.
ALTER TABLE notes ADD COLUMN synced_version INTEGER;
ALTER TABLE notes ADD COLUMN synced_hash INTEGER;
CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
"#,
    r#"
ALTER TABLE notes ADD COLUMN synced_meta INTEGER;
"#,
    r#"
-- Saved posts: JSON of `Link`, NULL for an ordinary note.
ALTER TABLE notes ADD COLUMN link TEXT;
"#,
    r#"
-- Fields from newer versions of Edge, as a JSON object, kept so this version never strips them.
ALTER TABLE notes ADD COLUMN extra TEXT;
"#,
    r#"
-- Failed details fetches for saves, this device only (not synced): when to try again.
CREATE TABLE fetch_attempts (note_id TEXT PRIMARY KEY, tries INTEGER NOT NULL, next_at INTEGER NOT NULL);
"#,
    r#"
-- Images and voice recordings: JSON array of `Attachment`, NULL when there are none.
ALTER TABLE notes ADD COLUMN attachments TEXT;
-- Attachment files this device has sent to the server (this device only, not synced).
CREATE TABLE media_uploaded (sha256 TEXT PRIMARY KEY);
"#,
    r#"
-- Order of pinned notes' tabs on the edge (see Note::pin_order).
ALTER TABLE notes ADD COLUMN pin_order INTEGER;
"#,
    r#"
-- Pinned notes this device has shown the user (this device only, not synced). A note pinned on
-- another device and not in here is new to this one. Notes pinned before this existed count as seen.
CREATE TABLE pin_seen (note_id TEXT PRIMARY KEY);
INSERT INTO pin_seen SELECT id FROM notes WHERE pinned = 1;
"#,
];

const NOTE_COLUMNS: &str =
    "id, title, body, pinned, archived, deleted_at, created_at, updated_at, \
                            device_id, version, conflict_of, color, link, extra, attachments, pin_order";

/// The local, offline-first note store. Every write lands here before any sync happens.
pub struct Store {
    conn: Connection,
    device_id: String,
}

impl Store {
    pub fn open(path: impl AsRef<Path>, device_id: impl Into<String>) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?, device_id.into())
    }

    pub fn open_in_memory(device_id: impl Into<String>) -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?, device_id.into())
    }

    fn init(conn: Connection, device_id: String) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let done: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(done) {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(Self { conn, device_id })
    }

    pub fn create(&mut self, body: &str) -> rusqlite::Result<Note> {
        let now = now_ms();
        let note = Note {
            id: Uuid::now_v7(),
            title: derive_title(body),
            body: body.to_owned(),
            pinned: false,
            archived: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
            device_id: self.device_id.clone(),
            version: 1,
            conflict_of: None,
            color: None,
            link: None,
            extra: Default::default(),
            attachments: Vec::new(),
            pin_order: None,
        };
        let tx = self.conn.transaction()?;
        write(&tx, &note)?;
        queue(&tx, note.id, now)?;
        tx.commit()?;
        Ok(note)
    }

    /// Saves the first link in shared or pasted `text`. The same post saved again returns the
    /// existing save (`false`) instead of a duplicate. Errors with `InvalidQuery` if there is no link.
    pub fn create_save(&mut self, text: &str) -> rusqlite::Result<(Note, bool)> {
        let link = Link::from_text(text).ok_or(rusqlite::Error::InvalidQuery)?;
        if let Some(id) = self
            .conn
            .query_row(
                "SELECT id FROM notes WHERE deleted_at IS NULL AND json_extract(link, '$.url') = ?1",
                [&link.url],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            let existing = self.get(parse_uuid(0, &id)?)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            return Ok((existing, false));
        }
        let mut note = self.create(text.trim())?;
        note = self.edit(note.id, |n| n.link = Some(link))?;
        Ok((note, true))
    }

    /// Moves a save to a folder (None: unsorted). No-op error if the note is not a save.
    pub fn set_save_folder(&mut self, id: Uuid, folder: Option<&str>) -> rusqlite::Result<Note> {
        let folder = folder
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(str::to_owned);
        self.edit_link(id, |l| l.folder = folder)
    }

    pub fn set_save_done(&mut self, id: Uuid, done: bool) -> rusqlite::Result<Note> {
        self.edit_link(id, |l| l.done = done)
    }

    /// Records that the original post was opened from Edge.
    pub fn mark_opened(&mut self, id: Uuid) -> rusqlite::Result<Note> {
        let now = now_ms();
        self.edit_link(id, |l| l.opened_at = Some(now))
    }

    /// Which saves to bring back on each of the next `days` days, `per_day` a day. `today_start`
    /// is local midnight in ms (the apps know the time zone). A save that is 7 days old and was
    /// never opened leads its day; the rest are the least looked-at saves, oldest first, with no
    /// save repeated in the plan until all have had a turn. Done saves and saves less than a day
    /// old are left out.
    pub fn digest_plan(
        &self,
        today_start: i64,
        days: usize,
        per_day: usize,
    ) -> rusqlite::Result<Vec<Vec<Note>>> {
        const DAY: i64 = 86_400_000;
        let mut pool: Vec<Note> = self
            .saves()?
            .into_iter()
            .filter(|n| n.link.as_ref().is_some_and(|l| !l.done))
            .collect();
        pool.sort_by_key(|n| {
            (
                n.link.as_ref().and_then(|l| l.opened_at).unwrap_or(0),
                n.created_at,
            )
        });
        let mut used = std::collections::HashSet::new();
        let mut plan = Vec::with_capacity(days);
        for d in 0..days as i64 {
            let start = today_start + d * DAY;
            let ready: Vec<&Note> = pool.iter().filter(|n| n.created_at < start - DAY).collect();
            let mut day: Vec<Note> = Vec::new();
            if let Some(n) = ready.iter().find(|n| {
                n.created_at >= start - 7 * DAY && n.created_at < start - 6 * DAY
                // saved on this weekday last week && n.link.as_ref().is_some_and(|l| l.opened_at.is_none())
            }) {
                used.insert(n.id);
                day.push((*n).clone());
            }
            if ready.iter().all(|n| used.contains(&n.id)) {
                used.retain(|id| day.iter().any(|n| n.id == *id)); // everyone had a turn: start over
            }
            for n in &ready {
                if day.len() >= per_day.min(ready.len()) {
                    break;
                }
                if used.insert(n.id) {
                    day.push((*n).clone());
                }
            }
            plan.push(day);
        }
        Ok(plan)
    }

    /// For the success test: of the saves made since `since` (ms), how many there are, how many
    /// were opened again from Edge, and how many are done.
    pub fn save_stats(&self, since: i64) -> rusqlite::Result<(i64, i64, i64)> {
        self.conn.query_row(
            "SELECT COUNT(*), \
                    COUNT(json_extract(link, '$.opened_at')), \
                    COALESCE(SUM(CASE WHEN json_extract(link, '$.done') THEN 1 ELSE 0 END), 0) \
             FROM notes WHERE deleted_at IS NULL AND link IS NOT NULL AND created_at >= ?1",
            [since],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    /// A random save that is not done, for "surprise me".
    pub fn random_save(&self) -> rusqlite::Result<Option<Note>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {NOTE_COLUMNS} FROM notes WHERE archived = 0 AND deleted_at IS NULL AND link IS NOT NULL \
                     AND NOT COALESCE(json_extract(link, '$.done'), 0) ORDER BY random() LIMIT 1"
                ),
                [],
                note_from_row,
            )
            .optional()
    }

    /// Saves whose details have not been fetched yet and are due (not waiting after a failure),
    /// oldest first.
    pub fn saves_needing_details(&self, limit: usize) -> rusqlite::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE deleted_at IS NULL AND link IS NOT NULL \
             AND json_extract(link, '$.fetched_at') IS NULL \
             AND id NOT IN (SELECT note_id FROM fetch_attempts WHERE next_at > ?1) \
             ORDER BY created_at LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![now_ms(), limit as i64], note_from_row)?;
        rows.collect()
    }

    /// Records what the post's page said and, unless the user already chose one, picks a folder.
    /// When the page names the post's own address (a Threads short link's target), the save takes
    /// it; if that post was already saved, this copy is deleted and the earlier save returned.
    pub fn set_save_details(&mut self, id: Uuid, p: &Preview) -> rusqlite::Result<Note> {
        let now = now_ms();
        let note = self.get(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let link = note
            .link
            .clone()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let canonical = p
            .canonical
            .as_deref()
            .and_then(clean_url)
            // Not for plain web pages: a page behind a login redirects to a sign-in URL
            // (with a fresh session id each time), which would replace the saved link.
            .filter(|c| {
                link.platform != "web"
                    && crate::link::platform_of(c) == link.platform
                    && *c != link.url
            });
        if let Some(c) = &canonical {
            if let Some(other) = self.live_save_with_url(c, id)? {
                self.delete(id)?;
                return Ok(other);
            }
        }
        let body = note.body.clone();
        let updated = self.edit_link(id, |l| {
            if let Some(c) = canonical {
                l.url = c;
            }
            (l.title, l.author, l.text, l.thumbnail) = (
                p.title.clone(),
                p.author.clone(),
                p.text.clone(),
                p.thumbnail.clone(),
            );
            l.fetched_at = Some(now);
            if l.folder.is_none() {
                let words = [l.title.as_deref(), l.text.as_deref(), Some(body.as_str())];
                l.folder = suggest_folder(
                    &l.platform,
                    &words.into_iter().flatten().collect::<Vec<_>>(),
                )
                .map(str::to_owned);
            }
        })?;
        self.conn.execute(
            "DELETE FROM fetch_attempts WHERE note_id = ?1",
            [id.to_string()],
        )?;
        Ok(updated)
    }

    /// A fetch failed (offline, server error): try again later, waiting longer each time. After
    /// the last try the save is marked fetched with what it has, so devices stop trying.
    pub fn record_fetch_failure(&mut self, id: Uuid) -> rusqlite::Result<()> {
        const WAIT_MS: [i64; 5] = [60_000, 600_000, 3_600_000, 21_600_000, 86_400_000];
        let tries: i64 = self
            .conn
            .query_row(
                "SELECT tries FROM fetch_attempts WHERE note_id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if tries as usize >= WAIT_MS.len() {
            self.set_save_details(id, &Preview::default())?;
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO fetch_attempts (note_id, tries, next_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(note_id) DO UPDATE SET tries = excluded.tries, next_at = excluded.next_at",
            params![
                id.to_string(),
                tries + 1,
                now_ms() + WAIT_MS[tries as usize]
            ],
        )?;
        Ok(())
    }

    /// Folders that hold at least one visible save, with counts: the default folders in their
    /// order first, then the user's own alphabetically. Unsorted saves are not counted.
    pub fn save_folders(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT json_extract(link, '$.folder') AS f, COUNT(*) FROM notes \
             WHERE archived = 0 AND deleted_at IS NULL AND link IS NOT NULL AND f IS NOT NULL GROUP BY f",
        )?;
        let mut v: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let rank = |f: &str| {
            crate::folders::FOLDERS
                .iter()
                .position(|d| *d == f)
                .unwrap_or(usize::MAX)
        };
        v.sort_by(|a, b| (rank(&a.0), &a.0).cmp(&(rank(&b.0), &b.0)));
        Ok(v)
    }

    fn live_save_with_url(&self, url: &str, except: Uuid) -> rusqlite::Result<Option<Note>> {
        let id = self
            .conn
            .query_row(
                "SELECT id FROM notes WHERE deleted_at IS NULL AND json_extract(link, '$.url') = ?1 AND id != ?2",
                params![url, except.to_string()],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        match id {
            Some(id) => self.get(parse_uuid(0, &id)?),
            None => Ok(None),
        }
    }

    fn edit_link(&mut self, id: Uuid, change: impl FnOnce(&mut Link)) -> rusqlite::Result<Note> {
        if self.get(id)?.and_then(|n| n.link).is_none() {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        self.edit(id, |n| change(n.link.as_mut().expect("checked above")))
    }

    /// Replaces the body. Errors with `QueryReturnedNoRows` if the note does not exist.
    pub fn update_body(&mut self, id: Uuid, body: &str) -> rusqlite::Result<Note> {
        self.edit(id, |n| {
            n.body = body.to_owned();
            n.title = derive_title(body);
        })
    }

    /// Pinned notes this device has not shown yet: pinned on another device since. Oldest first.
    pub fn unseen_pins(&self) -> rusqlite::Result<Vec<Uuid>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM notes WHERE pinned = 1 AND deleted_at IS NULL \
             AND id NOT IN (SELECT note_id FROM pin_seen) ORDER BY updated_at",
        )?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?;
        ids.map(|s| parse_uuid(0, &s?)).collect()
    }

    /// The user has seen pinned note `id` here (opened it, or looked at its tab).
    pub fn mark_pin_seen(&mut self, id: Uuid) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO pin_seen (note_id) VALUES (?1)",
            [id.to_string()],
        )?;
        Ok(())
    }

    /// A newly pinned note's tab goes first (top); the others move down. Pinning here counts as
    /// seen here; unpinning forgets it, so a later pin on another device is new again.
    pub fn set_pinned(&mut self, id: Uuid, pinned: bool) -> rusqlite::Result<Note> {
        if pinned {
            self.mark_pin_seen(id)?;
        } else {
            self.conn
                .execute("DELETE FROM pin_seen WHERE note_id = ?1", [id.to_string()])?;
        }
        let first: Option<i64> = self.conn.query_row(
            "SELECT min(COALESCE(pin_order, created_at)) FROM notes WHERE pinned = 1 AND deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let next = first.map_or(now_ms(), |f| f - 1000);
        self.edit(id, |n| {
            if pinned && !n.pinned {
                n.pin_order = Some(next);
            }
            n.pinned = pinned;
        })
    }

    /// Moves a pinned note's tab to just before `before` (None: to the end), as a drag does.
    /// Only the moved note changes, unless there is no room between its neighbours; then every
    /// pinned note is renumbered once.
    pub fn move_pin(&mut self, id: Uuid, before: Option<Uuid>) -> rusqlite::Result<Note> {
        let mut pins: Vec<Note> = self
            .list()?
            .into_iter()
            .filter(|n| n.pinned && n.id != id)
            .collect();
        pins.sort_by_key(|n| (n.pin_order.unwrap_or(n.created_at), n.created_at));
        let at = before
            .and_then(|b| pins.iter().position(|n| n.id == b))
            .unwrap_or(pins.len());
        let key = |n: &Note| n.pin_order.unwrap_or(n.created_at);
        let lo = at.checked_sub(1).map(|i| key(&pins[i]));
        let hi = pins.get(at).map(key);
        let order = match (lo, hi) {
            (Some(l), Some(h)) if h - l >= 2 => Some(l + (h - l) / 2),
            (Some(_), Some(_)) => None, // no room: renumber below
            (Some(l), None) => Some(l + 1000),
            (None, Some(h)) => Some(h - 1000),
            (None, None) => Some(now_ms()),
        };
        if let Some(o) = order {
            return self.edit(id, |n| n.pin_order = Some(o));
        }
        let me = self.get(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        pins.insert(at, me);
        let base = now_ms();
        for (i, n) in pins.iter().enumerate() {
            self.edit(n.id, |n| n.pin_order = Some(base + 1000 * i as i64))?;
        }
        self.get(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// Tab colour, "#rrggbb" or None. Anything else is rejected.
    pub fn set_color(&mut self, id: Uuid, color: Option<&str>) -> rusqlite::Result<Note> {
        if let Some(c) = color.filter(|c| !valid_color(c)) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "not a #rrggbb colour: {c}"
            )));
        }
        self.edit(id, |n| n.color = color.map(str::to_owned))
    }

    /// Soft delete: the note disappears from list and search but stays as a tombstone, so sync
    /// can tell other devices it was deleted.
    /// Attaches `a` (its file is already stored under its SHA-256) to note `id`.
    pub fn add_attachment(&mut self, id: Uuid, a: crate::Attachment) -> rusqlite::Result<Note> {
        let a = a.sanitize().ok_or(rusqlite::Error::InvalidQuery)?;
        if self
            .get(id)?
            .is_some_and(|n| n.attachments.len() >= crate::attachment::MAX_ATTACHMENTS)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.edit(id, |n| n.attachments.push(a))
    }

    /// Removes attachment `att` from note `id`, leaving a tombstone that syncs.
    pub fn remove_attachment(&mut self, id: Uuid, att: Uuid) -> rusqlite::Result<Note> {
        let now = now_ms();
        self.edit(id, |n| {
            if let Some(a) = n
                .attachments
                .iter_mut()
                .find(|a| a.id == att && a.is_live())
            {
                a.deleted_at = Some(now);
            }
        })
    }

    /// Files of attachments on live notes that this device has not uploaded yet.
    pub fn media_to_upload(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT json_extract(a.value, '$.sha256') AS sha FROM notes, json_each(notes.attachments) a \
             WHERE notes.deleted_at IS NULL AND json_extract(a.value, '$.deleted_at') IS NULL \
             AND sha NOT IN (SELECT sha256 FROM media_uploaded)",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn mark_uploaded(&mut self, sha256: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO media_uploaded (sha256) VALUES (?1)",
            [sha256],
        )?;
        Ok(())
    }

    /// Deletes notes left empty on this device (no text, no attachments, not pinned, not a
    /// save), except `keep`, the one open in the editor. Notes last edited on another device are
    /// left alone, so a note someone is typing into elsewhere is never removed under them.
    pub fn delete_empty_notes(&mut self, keep: Option<Uuid>) -> rusqlite::Result<usize> {
        let keep = keep.map(|k| k.to_string()).unwrap_or_default();
        let ids: Vec<String> = self
            .conn
            .prepare(
                "SELECT id FROM notes WHERE deleted_at IS NULL AND trim(body, ' ' || char(9, 10, 13)) = '' AND link IS NULL \
                 AND pinned = 0 AND device_id = ?1 AND id != ?2 \
                 AND NOT EXISTS (SELECT 1 FROM json_each(COALESCE(notes.attachments, '[]')) a \
                                 WHERE json_extract(a.value, '$.deleted_at') IS NULL)",
            )?
            .query_map(params![self.device_id, keep], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &ids {
            self.delete(parse_uuid(0, id)?)?;
        }
        Ok(ids.len())
    }

    pub fn delete(&mut self, id: Uuid) -> rusqlite::Result<Note> {
        self.edit(id, |n| n.deleted_at = Some(now_ms()))
    }

    /// Every edit bumps `version`, stamps time and device, reindexes and queues for sync.
    fn edit(&mut self, id: Uuid, change: impl FnOnce(&mut Note)) -> rusqlite::Result<Note> {
        let tx = self.conn.transaction()?;
        let mut note = get(&tx, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        change(&mut note);
        let now = now_ms();
        note.updated_at = now;
        note.device_id = self.device_id.clone();
        note.version += 1;
        tx.execute(
            "UPDATE notes SET title=?2, body=?3, pinned=?4, archived=?5, deleted_at=?6, \
             updated_at=?7, device_id=?8, version=?9, color=?10, link=?11, extra=?12, attachments=?13, pin_order=?14 WHERE id=?1",
            params![
                id.to_string(),
                note.title,
                note.body,
                note.pinned,
                note.archived,
                note.deleted_at,
                note.updated_at,
                note.device_id,
                note.version,
                note.color,
                link_json(&note),
                extra_json(&note),
                attachments_json(&note),
                note.pin_order,
            ],
        )?;
        index_and_queue(&tx, &note, now)?;
        tx.commit()?;
        Ok(note)
    }

    pub fn get(&self, id: Uuid) -> rusqlite::Result<Option<Note>> {
        get(&self.conn, id)
    }

    /// Visible notes (saves excluded): not archived, not deleted. Pinned first, then most recently edited.
    pub fn list(&self) -> rusqlite::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE archived = 0 AND deleted_at IS NULL AND link IS NULL \
             ORDER BY pinned DESC, updated_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], note_from_row)?;
        rows.collect()
    }

    /// Visible saves: pinned first, then newest saved first (a details fetch does not reorder them).
    pub fn saves(&self) -> rusqlite::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE archived = 0 AND deleted_at IS NULL AND link IS NOT NULL \
             ORDER BY pinned DESC, created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], note_from_row)?;
        rows.collect()
    }

    /// Prefix search over notes (saves excluded). Any user input is safe: words are quoted before MATCH.
    pub fn search(&self, query: &str) -> rusqlite::Result<Vec<Note>> {
        self.search_kind(query, false)
    }

    /// Prefix search over saves: shared text, URL and the fetched title, author and text.
    pub fn search_saves(&self, query: &str) -> rusqlite::Result<Vec<Note>> {
        self.search_kind(query, true)
    }

    fn search_kind(&self, query: &str, saves: bool) -> rusqlite::Result<Vec<Note>> {
        let fts_query = query
            .split_whitespace()
            .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ");
        if fts_query.is_empty() {
            return if saves { self.saves() } else { self.list() };
        }
        let kind = if saves {
            "n.link IS NOT NULL"
        } else {
            "n.link IS NULL"
        };
        let cols = NOTE_COLUMNS
            .split(", ")
            .map(|c| format!("n.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {cols} FROM notes_fts f JOIN notes n ON n.id = f.note_id \
             WHERE notes_fts MATCH ?1 AND n.archived = 0 AND n.deleted_at IS NULL AND {kind} \
             ORDER BY n.pinned DESC, bm25(notes_fts), n.updated_at DESC"
        ))?;
        let rows = stmt.query_map([fts_query], note_from_row)?;
        rows.collect()
    }

    /// What to send to `/v1/sync`: every queued note with the server state it started from.
    pub fn sync_request(&self) -> rusqlite::Result<SyncRequest> {
        let cols = NOTE_COLUMNS
            .split(", ")
            .map(|c| format!("n.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {cols}, n.synced_version, n.synced_hash, n.synced_meta FROM sync_queue q JOIN notes n ON n.id = q.note_id \
             ORDER BY q.queued_at, q.note_id"
        ))?;
        let changes = stmt
            .query_map([], |r| {
                // The synced_* columns come right after the note's own columns.
                let k = NOTE_COLUMNS.split(", ").count();
                let base = match (r.get::<_, Option<i64>>(k)?, r.get::<_, Option<i64>>(k + 1)?) {
                    (Some(version), Some(h)) => Some(Base {
                        version,
                        body_hash: h as u64,
                        meta_hash: r.get::<_, Option<i64>>(k + 2)?.map(|m| m as u64),
                    }),
                    _ => None,
                };
                Ok(Change {
                    note: note_from_row(r)?,
                    base,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(SyncRequest {
            cursor: self.cursor()?,
            changes,
        })
    }

    /// Applies the server's answer to `sent`, in one transaction. Notes edited while the request
    /// was in flight stay queued and are merged, so nothing typed during a sync is lost.
    pub fn apply_sync(&mut self, sent: &SyncRequest, resp: &SyncResponse) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for c in &sent.changes {
            // The server now holds what we sent: that is our new base, even if it merged it.
            set_synced(&tx, &c.note)?;
            tx.execute(
                "DELETE FROM sync_queue WHERE note_id = ?1 AND \
                 (SELECT version FROM notes WHERE id = ?1) = ?2",
                params![c.note.id.to_string(), c.note.version],
            )?;
        }
        for remote in &resp.notes {
            let remote = sanitize(remote.clone());
            let queued: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_queue WHERE note_id = ?1)",
                [remote.id.to_string()],
                |r| r.get(0),
            )?;
            let local = get(&tx, remote.id)?;
            match local {
                Some(local) if queued => {
                    let base = synced_base(&tx, remote.id)?;
                    let merged = merge(base, &local, &remote);
                    write(&tx, &merged.note)?;
                    if let Some(copy) = merged.copy {
                        write(&tx, &copy)?;
                        queue(&tx, copy.id, now_ms())?;
                    }
                }
                _ => write(&tx, &remote)?,
            }
            set_synced(&tx, &remote)?;
            if !remote.pinned || remote.deleted_at.is_some() {
                tx.execute(
                    "DELETE FROM pin_seen WHERE note_id = ?1",
                    [remote.id.to_string()],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('cursor', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [resp.cursor.max(sent.cursor)],
        )?;
        tx.commit()
    }

    /// The server sequence this device has synced up to.
    pub fn cursor(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'cursor'", [], |r| {
                r.get(0)
            })
            .optional()
            .map(|c| c.unwrap_or(0))
    }

    /// IDs of notes changed since the last successful sync, oldest change first.
    pub fn pending_sync(&self) -> rusqlite::Result<Vec<Uuid>> {
        let mut stmt = self
            .conn
            .prepare("SELECT note_id FROM sync_queue ORDER BY queued_at, note_id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| r.and_then(|s| parse_uuid(0, &s))).collect()
    }
}

fn get(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<Note>> {
    conn.query_row(
        &format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"),
        [id.to_string()],
        note_from_row,
    )
    .optional()
}

/// Inserts or replaces a note and reindexes it, without queueing it for sync.
fn write(conn: &Connection, n: &Note) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO notes ({NOTE_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) \
             ON CONFLICT(id) DO UPDATE SET title=?2, body=?3, pinned=?4, archived=?5, deleted_at=?6, \
             created_at=?7, updated_at=?8, device_id=?9, version=?10, conflict_of=?11, color=?12, link=?13, extra=?14, \
             attachments=?15, pin_order=?16"
        ),
        params![
            n.id.to_string(),
            n.title,
            n.body,
            n.pinned,
            n.archived,
            n.deleted_at,
            n.created_at,
            n.updated_at,
            n.device_id,
            n.version,
            n.conflict_of.map(|u| u.to_string()),
            n.color,
            link_json(n),
            extra_json(n),
            attachments_json(n),
            n.pin_order,
        ],
    )?;
    index(conn, n)
}

/// Records `n` as the server state this device has seen for that note.
fn set_synced(conn: &Connection, n: &Note) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE notes SET synced_version = ?2, synced_hash = ?3, synced_meta = ?4 WHERE id = ?1",
        params![
            n.id.to_string(),
            n.version,
            body_hash(&n.body) as i64,
            meta_hash(n) as i64
        ],
    )?;
    Ok(())
}

fn synced_base(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<Base>> {
    conn.query_row(
        "SELECT synced_version, synced_hash, synced_meta FROM notes WHERE id = ?1",
        [id.to_string()],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        },
    )
    .map(|(v, h, m)| {
        Some(Base {
            version: v?,
            body_hash: h? as u64,
            meta_hash: m.map(|m| m as u64),
        })
    })
}

/// Notes from the network are untrusted: the title is rederived and a bad colour dropped.
fn sanitize(mut n: Note) -> Note {
    n.title = derive_title(&n.body);
    if n.color.as_deref().is_some_and(|c| !valid_color(c)) {
        n.color = None;
    }
    n.link = n.link.and_then(Link::sanitize);
    n.attachments = std::mem::take(&mut n.attachments)
        .into_iter()
        .filter_map(crate::Attachment::sanitize)
        .take(crate::attachment::MAX_ATTACHMENTS)
        .collect();
    n.cap_extra();
    n
}

fn extra_json(n: &Note) -> Option<String> {
    (!n.extra.is_empty())
        .then(|| serde_json::to_string(&n.extra).expect("a JSON map always serialises"))
}

fn attachments_json(n: &Note) -> Option<String> {
    (!n.attachments.is_empty())
        .then(|| serde_json::to_string(&n.attachments).expect("attachments always serialise"))
}

fn link_json(n: &Note) -> Option<String> {
    n.link
        .as_ref()
        .map(|l| serde_json::to_string(l).expect("Link always serialises"))
}

fn index_and_queue(conn: &Connection, note: &Note, now: i64) -> rusqlite::Result<()> {
    index(conn, note)?;
    queue(conn, note.id, now)
}

fn index(conn: &Connection, note: &Note) -> rusqlite::Result<()> {
    let id = note.id.to_string();
    conn.execute("DELETE FROM notes_fts WHERE note_id = ?1", [&id])?;
    conn.execute(
        "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
        params![id, note.title, searchable(note)],
    )?;
    Ok(())
}

fn queue(conn: &Connection, id: Uuid, now: i64) -> rusqlite::Result<()> {
    let id = id.to_string();
    conn.execute(
        "INSERT INTO sync_queue (note_id, queued_at) VALUES (?1, ?2) \
         ON CONFLICT(note_id) DO UPDATE SET queued_at = excluded.queued_at",
        params![id, now],
    )?;
    Ok(())
}

fn note_from_row(r: &Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: parse_uuid(0, &r.get::<_, String>(0)?)?,
        title: r.get(1)?,
        body: r.get(2)?,
        pinned: r.get(3)?,
        archived: r.get(4)?,
        deleted_at: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        device_id: r.get(8)?,
        version: r.get(9)?,
        conflict_of: r
            .get::<_, Option<String>>(10)?
            .map(|s| parse_uuid(10, &s))
            .transpose()?,
        color: r.get(11)?,
        // A link that does not parse (hand-edited or from a future version) leaves an ordinary note.
        link: r
            .get::<_, Option<String>>(12)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        extra: r
            .get::<_, Option<String>>(13)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        attachments: r
            .get::<_, Option<String>>(14)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        pin_order: r.get(15)?,
    })
}

/// What search sees: the body, plus a save's URL words and fetched details.
fn searchable(n: &Note) -> String {
    match &n.link {
        None => n.body.clone(),
        Some(l) => {
            let url_words = l.url.replace(['/', '.', '?', '=', '&', '-', '_', '@'], " ");
            [
                Some(n.body.as_str()),
                Some(url_words.as_str()),
                l.title.as_deref(),
                l.author.as_deref(),
                l.text.as_deref(),
                l.folder.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
        }
    }
}

fn parse_uuid(col: usize, s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, e.into())
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory("test-device").unwrap()
    }

    #[test]
    fn new_pins_go_last_and_a_drag_reorders_editing_only_what_moved() {
        let mut s = store();
        let [a, b, c] = ["a", "b", "c"].map(|t| s.create(t).unwrap().id);
        for id in [b, a, c] {
            s.set_pinned(id, true).unwrap();
        }
        let order = |s: &Store| {
            let mut p: Vec<_> = s.list().unwrap().into_iter().filter(|n| n.pinned).collect();
            p.sort_by_key(|n| n.pin_order);
            p.into_iter().map(|n| n.body).collect::<Vec<_>>()
        };
        assert_eq!(order(&s), ["c", "a", "b"], "newest pin on top");
        let versions = |s: &Store| [a, b, c].map(|id| s.get(id).unwrap().unwrap().version);
        let before = versions(&s);
        s.move_pin(a, Some(c)).unwrap(); // drag "a" to the top
        assert_eq!(order(&s), ["a", "c", "b"]);
        s.move_pin(a, None).unwrap(); // and to the end
        assert_eq!(order(&s), ["c", "b", "a"]);
        s.move_pin(a, Some(b)).unwrap(); // between c and b
        assert_eq!(order(&s), ["c", "a", "b"]);
        let after = versions(&s);
        let edited: i64 = before.iter().zip(after).map(|(x, y)| y - x).sum();
        assert!(edited <= 3 + 3, "renumbering at most once");
    }

    #[test]
    fn empty_notes_are_deleted_except_the_open_one_and_ones_with_files() {
        let mut s = store();
        let open = s.create("").unwrap();
        let blank = s.create("  \n ").unwrap();
        let photo = s.create("").unwrap();
        let att = crate::attachment::new_attachment(
            "a".repeat(64),
            1,
            "image/png",
            None,
            None,
            None,
            None,
            1,
        )
        .unwrap();
        s.add_attachment(photo.id, att).unwrap();
        let text = s.create("milk").unwrap();
        let mut other = Store::open_in_memory("phone").unwrap();
        let theirs = other.create("").unwrap();
        s.apply_sync(
            &Default::default(),
            &crate::sync::SyncResponse {
                cursor: 1,
                notes: vec![theirs.clone()],
                more: false,
            },
        )
        .unwrap();
        assert_eq!(s.delete_empty_notes(Some(open.id)).unwrap(), 1);
        let left: Vec<_> = s.list().unwrap().into_iter().map(|n| n.id).collect();
        assert!(!left.contains(&blank.id));
        for kept in [open.id, photo.id, text.id, theirs.id] {
            assert!(left.contains(&kept));
        }
    }

    #[test]
    fn files_to_upload_are_live_attachments_not_sent_yet() {
        let mut s = store();
        let n = s.create("Trip").unwrap();
        let att = |c: char| {
            crate::attachment::new_attachment(
                c.to_string().repeat(64),
                1,
                "image/png",
                None,
                None,
                None,
                None,
                1,
            )
            .unwrap()
        };
        let (a, b, c) = (att('a'), att('b'), att('c'));
        s.add_attachment(n.id, a.clone()).unwrap();
        s.add_attachment(n.id, b.clone()).unwrap();
        s.add_attachment(n.id, c.clone()).unwrap();
        s.remove_attachment(n.id, c.id).unwrap();
        s.mark_uploaded(&a.sha256).unwrap();
        assert_eq!(
            s.media_to_upload().unwrap(),
            std::slice::from_ref(&b.sha256)
        );
        s.delete(n.id).unwrap();
        assert!(
            s.media_to_upload().unwrap().is_empty(),
            "a deleted note's files are not sent"
        );
    }

    #[test]
    fn saving_the_same_post_twice_returns_the_first_save() {
        let mut s = store();
        let (a, new_a) = s
            .create_save("https://x.com/b4dgerr/status/2121498943662023026?s=19")
            .unwrap();
        let (b, new_b) = s
            .create_save("look: https://twitter.com/b4dgerr/status/2121498943662023026?t=x")
            .unwrap();
        assert!(new_a && !new_b);
        assert_eq!(a.id, b.id);
        assert_eq!(s.saves().unwrap().len(), 1);
        // Once deleted, saving it again makes a new save.
        s.delete(a.id).unwrap();
        assert!(
            s.create_save("https://x.com/b4dgerr/status/2121498943662023026")
                .unwrap()
                .1
        );
    }

    #[test]
    fn saves_and_notes_are_listed_and_searched_apart() {
        let mut s = store();
        let n = s.create("chess openings to learn").unwrap();
        let (v, _) = s
            .create_save("https://www.instagram.com/reel/DftSYUwtKNT/")
            .unwrap();
        assert_eq!(
            s.list().unwrap().iter().map(|x| x.id).collect::<Vec<_>>(),
            vec![n.id]
        );
        assert_eq!(
            s.saves().unwrap().iter().map(|x| x.id).collect::<Vec<_>>(),
            vec![v.id]
        );
        assert_eq!(s.search("chess").unwrap().len(), 1);
        assert!(s.search("instagram").unwrap().is_empty());
        // URL words and the folder are searchable among saves.
        assert_eq!(s.search_saves("instagram").unwrap()[0].id, v.id);
        s.set_save_folder(v.id, Some("Learn")).unwrap();
        assert_eq!(s.search_saves("learn").unwrap()[0].id, v.id);
        assert!(s.search_saves("chess").unwrap().is_empty());
    }

    #[test]
    fn text_without_a_link_is_not_a_save_and_notes_have_no_save_actions() {
        let mut s = store();
        assert!(s.create_save("no link here").is_err());
        let n = s.create("plain").unwrap();
        assert!(s.set_save_done(n.id, true).is_err());
        assert_eq!(s.get(n.id).unwrap().unwrap().link, None);
    }

    #[test]
    fn a_database_from_before_saves_upgrades_and_keeps_its_notes() {
        let path = std::env::temp_dir().join(format!("edge-mig-{}.db", Uuid::now_v7()));
        {
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..4] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 4).unwrap();
            conn.execute(
                "INSERT INTO notes (id,title,body,created_at,updated_at,device_id,version) VALUES (?1,'old','old',1,1,'d',1)",
                [Uuid::now_v7().to_string()],
            )
            .unwrap();
        }
        let mut s = Store::open(&path, "d").unwrap();
        assert_eq!(s.list().unwrap()[0].body, "old");
        assert!(s.create_save("https://example.com/a").unwrap().1);
        std::fs::remove_file(&path).ok();
    }

    const DAY: i64 = 86_400_000;

    /// A save made `days_ago` days before `now`.
    fn aged_save(s: &mut Store, url: &str, now: i64, days_ago: i64) -> Uuid {
        let (n, _) = s.create_save(url).unwrap();
        s.conn
            .execute(
                "UPDATE notes SET created_at = ?2 WHERE id = ?1",
                params![n.id.to_string(), now - days_ago * DAY],
            )
            .unwrap();
        n.id
    }

    #[test]
    fn the_digest_brings_back_old_unopened_saves_and_leads_with_the_week_old_one() {
        let mut s = store();
        let today = 100 * DAY;
        let old = aged_save(&mut s, "https://x.com/a/status/1", today, 30);
        let opened = aged_save(&mut s, "https://x.com/a/status/2", today, 20);
        let week = aged_save(&mut s, "https://x.com/a/status/3", today, 7);
        let done = aged_save(&mut s, "https://x.com/a/status/4", today, 10);
        let fresh = aged_save(&mut s, "https://x.com/a/status/5", today, 0);
        let mid = aged_save(&mut s, "https://x.com/a/status/6", today, 3);
        s.mark_opened(opened).unwrap();
        s.set_save_done(done, true).unwrap();
        let plan = s.digest_plan(today, 3, 3).unwrap();
        let ids = |d: &Vec<Note>| d.iter().map(|n| n.id).collect::<Vec<_>>();
        assert_eq!(
            ids(&plan[0]),
            vec![week, old, mid],
            "week-old first, then never opened, oldest first"
        );
        assert!(
            !plan.iter().flatten().any(|n| n.id == done),
            "never a done save"
        );
        assert!(
            !plan[0].iter().chain(&plan[1]).any(|n| n.id == fresh),
            "a save waits a day before it comes back"
        );
        assert!(plan[2].iter().any(|n| n.id == fresh));
        // Day two: the opened one gets its turn before anyone repeats.
        assert_eq!(plan[1][0].id, opened);
        assert_eq!(s.digest_plan(today, 1, 3).unwrap()[0].len(), 3);
        assert!(s.random_save().unwrap().is_some_and(|n| n.id != done));
    }

    #[test]
    fn stats_count_saves_opens_and_dones_since_a_time() {
        let mut s = store();
        let now = 100 * DAY;
        let a = aged_save(&mut s, "https://x.com/a/status/1", now, 2);
        let b = aged_save(&mut s, "https://x.com/a/status/2", now, 3);
        aged_save(&mut s, "https://x.com/a/status/3", now, 10);
        s.create("a note").unwrap();
        s.mark_opened(a).unwrap();
        s.set_save_done(b, true).unwrap();
        assert_eq!(s.save_stats(now - 7 * DAY).unwrap(), (2, 1, 1));
    }

    #[test]
    fn an_empty_library_has_an_empty_digest() {
        let s = store();
        assert_eq!(s.digest_plan(0, 2, 3).unwrap(), vec![vec![], vec![]]);
        assert!(s.random_save().unwrap().is_none());
    }

    fn preview(title: &str, canonical: Option<&str>) -> Preview {
        Preview {
            title: Some(title.into()),
            text: Some(title.into()),
            canonical: canonical.map(Into::into),
            ..Preview::default()
        }
    }

    #[test]
    fn details_fill_the_save_pick_a_folder_and_make_it_searchable() {
        let mut s = store();
        let (v, _) = s
            .create_save("https://www.instagram.com/reel/DftSYUwtKNT/")
            .unwrap();
        assert_eq!(s.saves_needing_details(10).unwrap().len(), 1);
        let n = s
            .set_save_details(v.id, &preview("5 minute dal recipe", None))
            .unwrap();
        let l = n.link.unwrap();
        assert_eq!(
            (l.title.as_deref(), l.folder.as_deref()),
            (Some("5 minute dal recipe"), Some("Recipes"))
        );
        assert!(l.fetched_at.is_some());
        assert!(s.saves_needing_details(10).unwrap().is_empty());
        assert_eq!(s.search_saves("dal").unwrap()[0].id, v.id);
        assert_eq!(s.save_folders().unwrap(), vec![("Recipes".to_string(), 1)]);
    }

    #[test]
    fn a_folder_the_user_chose_is_kept() {
        let mut s = store();
        let (v, _) = s.create_save("https://x.com/a/status/1").unwrap();
        s.set_save_folder(v.id, Some("Gifts for Maa")).unwrap();
        let n = s
            .set_save_details(v.id, &preview("new workout plan", None))
            .unwrap();
        assert_eq!(n.link.unwrap().folder.as_deref(), Some("Gifts for Maa"));
    }

    #[test]
    fn a_web_page_behind_a_login_keeps_its_own_link() {
        let mut s = store();
        let url = "https://play.google.com/apps/testing/com.asin.cnglocator";
        let (v, _) = s.create_save(url).unwrap();
        let signin = format!("https://accounts.google.com/v3/signin/identifier?continue={url}");
        let n = s
            .set_save_details(v.id, &preview("Sign in - Google Accounts", Some(&signin)))
            .unwrap();
        assert_eq!(n.link.unwrap().url, url);
        // So saving it again finds the first save instead of making a copy.
        assert!(!s.create_save(url).unwrap().1);
    }

    #[test]
    fn a_threads_short_link_takes_the_post_address_and_duplicates_collapse() {
        let mut s = store();
        let post = "https://www.threads.com/@nick_saraev/post/Dfui2UBE4Sv";
        let (a, _) = s
            .create_save("https://www.threads.com/share/_7fwIRJAV")
            .unwrap();
        let a2 = s
            .set_save_details(
                a.id,
                &preview("Meta open sourced", Some(&format!("{post}?xmt=1"))),
            )
            .unwrap();
        assert_eq!(a2.link.unwrap().url, post);
        // Sharing the same post again gives a new short link; once fetched it folds into the first.
        let (b, new) = s
            .create_save("https://www.threads.com/share/_oANR7c7l")
            .unwrap();
        assert!(new);
        let kept = s
            .set_save_details(b.id, &preview("Meta open sourced", Some(post)))
            .unwrap();
        assert_eq!(kept.id, a.id);
        assert_eq!(s.saves().unwrap().len(), 1);
        // A canonical address on another site is not trusted.
        let (c, _) = s.create_save("https://x.com/a/status/2").unwrap();
        let c2 = s
            .set_save_details(c.id, &preview("t", Some("https://evil.example/x")))
            .unwrap();
        assert_eq!(c2.link.unwrap().url, "https://x.com/a/status/2");
    }

    #[test]
    fn failed_fetches_wait_then_give_up() {
        let mut s = store();
        let (v, _) = s
            .create_save("https://www.youtube.com/watch?v=hdMszZ2ah5Q")
            .unwrap();
        s.record_fetch_failure(v.id).unwrap();
        assert!(
            s.saves_needing_details(10).unwrap().is_empty(),
            "waits a minute after a failure"
        );
        s.conn
            .execute("UPDATE fetch_attempts SET next_at = 0", [])
            .unwrap();
        assert_eq!(s.saves_needing_details(10).unwrap().len(), 1, "due again");
        for _ in 0..5 {
            s.record_fetch_failure(v.id).unwrap();
        }
        let l = s.get(v.id).unwrap().unwrap().link.unwrap();
        assert!(l.fetched_at.is_some(), "stops trying after the last wait");
        assert_eq!(
            l.folder.as_deref(),
            Some("Watch Later"),
            "folder by platform when nothing was read"
        );
    }

    #[test]
    fn create_then_list_newest_first() {
        let mut s = store();
        let a = s.create("first").unwrap();
        let b = s.create("second").unwrap();
        let ids: Vec<_> = s.list().unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![b.id, a.id]);
    }

    #[test]
    fn pinned_notes_come_first() {
        let mut s = store();
        let a = s.create("old but pinned").unwrap();
        let b = s.create("newer").unwrap();
        s.set_pinned(a.id, true).unwrap();
        s.update_body(b.id, "newer, edited after pin").unwrap();
        let ids: Vec<_> = s.list().unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![a.id, b.id]);
    }

    #[test]
    fn edit_bumps_version_and_rederives_title() {
        let mut s = store();
        let n = s.create("Draft\nbody").unwrap();
        assert_eq!(n.version, 1);
        let n2 = s.update_body(n.id, "Final\nbody").unwrap();
        assert_eq!(n2.version, 2);
        assert_eq!(n2.title, "Final");
        assert_eq!(s.get(n.id).unwrap().unwrap(), n2);
    }

    #[test]
    fn color_is_stored_and_validated() {
        let mut s = store();
        let n = s.create("tab me").unwrap();
        assert_eq!(
            s.set_color(n.id, Some("#3b82f6")).unwrap().color.as_deref(),
            Some("#3b82f6")
        );
        assert!(s.set_color(n.id, Some("blue")).is_err());
        assert_eq!(
            s.get(n.id).unwrap().unwrap().color.as_deref(),
            Some("#3b82f6")
        );
        assert_eq!(s.set_color(n.id, None).unwrap().color, None);
    }

    #[test]
    fn version_1_database_gets_the_color_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO notes (id,title,body,created_at,updated_at,device_id,version) VALUES (?1,'old','old',1,1,'d',1)",
            [Uuid::now_v7().to_string()],
        )
        .unwrap();
        let s = Store::init(conn, "d".into()).unwrap();
        assert_eq!(s.list().unwrap()[0].color, None);
    }

    #[test]
    fn editing_a_missing_note_errors() {
        let mut s = store();
        let err = s.update_body(Uuid::now_v7(), "x").unwrap_err();
        assert!(matches!(err, rusqlite::Error::QueryReturnedNoRows));
    }

    #[test]
    fn deleted_notes_leave_list_and_search_but_stay_queued() {
        let mut s = store();
        let n = s.create("Old idea").unwrap();
        let gone = s.delete(n.id).unwrap();
        assert!(gone.deleted_at.is_some());
        assert_eq!(gone.version, n.version + 1);
        assert!(s.list().unwrap().is_empty());
        assert!(s.search("idea").unwrap().is_empty());
        assert_eq!(s.pending_sync().unwrap(), vec![n.id]);
    }

    #[test]
    fn search_finds_body_words_and_prefixes() {
        let mut s = store();
        let hit = s.create("Groceries\nbuy milk and eggs").unwrap();
        s.create("Call the plumber").unwrap();
        let found: Vec<_> = s.search("mil").unwrap().into_iter().map(|n| n.id).collect();
        assert_eq!(found, vec![hit.id]);
    }

    #[test]
    fn search_sees_edits_not_old_text() {
        let mut s = store();
        let n = s.create("apples").unwrap();
        s.update_body(n.id, "oranges").unwrap();
        assert!(s.search("apples").unwrap().is_empty());
        assert_eq!(s.search("oranges").unwrap().len(), 1);
    }

    #[test]
    fn search_accepts_any_user_input() {
        let mut s = store();
        s.create("he said \"hi\" (AND) NOT * ^").unwrap();
        for q in ["\"", "AND", "(", "NOT *", "^", "a\"b", "   "] {
            s.search(q)
                .unwrap_or_else(|e| panic!("query {q:?} failed: {e}"));
        }
    }

    #[test]
    fn every_write_is_queued_once_per_note() {
        let mut s = store();
        let a = s.create("a").unwrap();
        let b = s.create("b").unwrap();
        s.update_body(a.id, "a2").unwrap();
        let mut pending = s.pending_sync().unwrap();
        pending.sort();
        let mut expected = vec![a.id, b.id];
        expected.sort();
        assert_eq!(pending, expected);
    }

    #[test]
    fn reopening_a_file_keeps_notes_and_skips_migrations() {
        let dir = std::env::temp_dir().join(format!("edge-notes-test-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.db");
        let id = Store::open(&path, "d")
            .unwrap()
            .create("persist me")
            .unwrap()
            .id;
        let body = Store::open(&path, "d")
            .unwrap()
            .get(id)
            .unwrap()
            .unwrap()
            .body;
        assert_eq!(body, "persist me");
        // Store is dropped by now; Windows refuses to delete an open database file.
        std::fs::remove_dir_all(dir).unwrap();
    }
}
