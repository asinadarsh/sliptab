use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::note::{derive_title, Note};

/// Schema migrations, applied in order. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[r#"
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
"#];

const NOTE_COLUMNS: &str = "id, title, body, pinned, archived, deleted_at, created_at, updated_at, \
                            device_id, version, conflict_of";

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
        };
        let tx = self.conn.transaction()?;
        tx.execute(
            &format!("INSERT INTO notes ({NOTE_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"),
            params![
                note.id.to_string(),
                note.title,
                note.body,
                note.pinned,
                note.archived,
                note.deleted_at,
                note.created_at,
                note.updated_at,
                note.device_id,
                note.version,
                note.conflict_of.map(|u| u.to_string()),
            ],
        )?;
        index_and_queue(&tx, &note, now)?;
        tx.commit()?;
        Ok(note)
    }

    /// Replaces the body. Errors with `QueryReturnedNoRows` if the note does not exist.
    pub fn update_body(&mut self, id: Uuid, body: &str) -> rusqlite::Result<Note> {
        self.edit(id, |n| {
            n.body = body.to_owned();
            n.title = derive_title(body);
        })
    }

    pub fn set_pinned(&mut self, id: Uuid, pinned: bool) -> rusqlite::Result<Note> {
        self.edit(id, |n| n.pinned = pinned)
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
             updated_at=?7, device_id=?8, version=?9 WHERE id=?1",
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
            ],
        )?;
        index_and_queue(&tx, &note, now)?;
        tx.commit()?;
        Ok(note)
    }

    pub fn get(&self, id: Uuid) -> rusqlite::Result<Option<Note>> {
        get(&self.conn, id)
    }

    /// Visible notes: not archived, not deleted. Pinned first, then most recently edited.
    pub fn list(&self) -> rusqlite::Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE archived = 0 AND deleted_at IS NULL \
             ORDER BY pinned DESC, updated_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], note_from_row)?;
        rows.collect()
    }

    /// Prefix search over title and body. Any user input is safe: words are quoted before MATCH.
    pub fn search(&self, query: &str) -> rusqlite::Result<Vec<Note>> {
        let fts_query = query
            .split_whitespace()
            .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ");
        if fts_query.is_empty() {
            return self.list();
        }
        let cols = NOTE_COLUMNS.split(", ").map(|c| format!("n.{c}")).collect::<Vec<_>>().join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {cols} FROM notes_fts f JOIN notes n ON n.id = f.note_id \
             WHERE notes_fts MATCH ?1 AND n.archived = 0 AND n.deleted_at IS NULL \
             ORDER BY n.pinned DESC, bm25(notes_fts), n.updated_at DESC"
        ))?;
        let rows = stmt.query_map([fts_query], note_from_row)?;
        rows.collect()
    }

    /// IDs of notes changed since the last successful sync, oldest change first.
    pub fn pending_sync(&self) -> rusqlite::Result<Vec<Uuid>> {
        let mut stmt = self.conn.prepare("SELECT note_id FROM sync_queue ORDER BY queued_at, note_id")?;
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

fn index_and_queue(conn: &Connection, note: &Note, now: i64) -> rusqlite::Result<()> {
    let id = note.id.to_string();
    conn.execute("DELETE FROM notes_fts WHERE note_id = ?1", [&id])?;
    conn.execute(
        "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
        params![id, note.title, note.body],
    )?;
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
        conflict_of: r.get::<_, Option<String>>(10)?.map(|s| parse_uuid(10, &s)).transpose()?,
    })
}

fn parse_uuid(col: usize, s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, e.into()))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory("test-device").unwrap()
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
    fn editing_a_missing_note_errors() {
        let mut s = store();
        let err = s.update_body(Uuid::now_v7(), "x").unwrap_err();
        assert!(matches!(err, rusqlite::Error::QueryReturnedNoRows));
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
            s.search(q).unwrap_or_else(|e| panic!("query {q:?} failed: {e}"));
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
        let id = Store::open(&path, "d").unwrap().create("persist me").unwrap().id;
        let body = Store::open(&path, "d").unwrap().get(id).unwrap().unwrap().body;
        assert_eq!(body, "persist me");
        // Store is dropped by now; Windows refuses to delete an open database file.
        std::fs::remove_dir_all(dir).unwrap();
    }
}
