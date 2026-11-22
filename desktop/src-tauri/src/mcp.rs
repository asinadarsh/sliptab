//! Sliptab for AI agents: an MCP server on stdin/stdout, started by the agent as
//! `<sliptab> mcp` (Claude Code, Codex, Cursor, Kiro, Claude Desktop...). It opens the same
//! notes database as the app, so its edits sync like any edit made here; the running app
//! notices them within seconds (see `watch_other_writers` in lib.rs).
//!
//! What an agent may do is the user's choice in the app (Agents tab), re-read on every call:
//! Off, Read, Read & write, or Full access (also delete). Nothing works until it is turned on.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use notes_core::{Note, Store, Uuid};
use serde_json::{json, Value};

use crate::settings::{self, McpAccess};

const PROTOCOL: &str = "2025-06-18";

/// The app's data folder, found the way Tauri's app_data_dir does, without starting Tauri.
fn data_dir() -> Result<PathBuf, String> {
    const ID: &str = "com.asinadarsh.edge"; // tauri.conf.json identifier
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/share")))
    };
    base.map(|b| b.join(ID))
        .ok_or_else(|| "cannot find the home folder".into())
}

/// Runs the server until the agent closes stdin.
pub fn serve() {
    let dir = match data_dir() {
        Ok(d) => d,
        Err(e) => return eprintln!("sliptab mcp: {e}"),
    };
    let _ = std::fs::create_dir_all(&dir);
    let mut store = match crate::device_id(&dir)
        .map_err(|e| e.to_string())
        .and_then(|id| Store::open(dir.join("notes.db"), id).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => return eprintln!("sliptab mcp: cannot open the notes: {e}"),
    };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut client = String::from("An AI agent");
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                note_use(&dir, &msg, &mut client);
                handle(&msg, &mut store, settings::load(&dir).mcp_access)
            }
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            ),
        };
        if let Some(r) = reply {
            if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

/// Which agent connected and what it last did, for the Agents tab ("Kiro, 2 min ago").
pub const LAST_USE: &str = "mcp-last.json";
fn note_use(dir: &std::path::Path, msg: &Value, client: &mut String) {
    match msg["method"].as_str() {
        Some("initialize") => {
            if let Some(n) = msg["params"]["clientInfo"]["name"].as_str() {
                *client = n.chars().take(60).collect();
            }
        }
        Some("tools/call") => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let v = json!({"client": client, "tool": msg["params"]["name"], "at": now as u64});
            let _ = std::fs::write(dir.join(LAST_USE), v.to_string());
        }
        _ => {}
    }
}

/// One JSON-RPC message in, at most one reply out (notifications get none).
fn handle(msg: &Value, store: &mut Store, access: McpAccess) -> Option<Value> {
    let id = msg.get("id")?.clone(); // no id: a notification
    let ok = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => {
            let version = params["protocolVersion"].as_str().unwrap_or(PROTOCOL);
            ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "sliptab", "title": "Sliptab notes", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": tools(access)})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (text, is_error) = match call(name, &args, store, access) {
                Ok(v) => (v, false),
                Err(e) => (e, true),
            };
            ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
        }
        other => Some(json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": -32601, "message": format!("unknown method {other}")}})),
    }
}

const INSTRUCTIONS: &str = "Sliptab is the user's notes app (a bar on the laptop's screen edge, synced with \
their phone). Notes are plain text: the first line is the title, `- [ ] item` / `- [x] item` lines are a \
checklist, and **bold**, *italic*, __underline__ and ~~strike~~ are formatting. Pinned notes show as tabs on \
the screen edge, so pin only what the user should keep in view. What you may do depends on the access the \
user chose in Sliptab (Agents tab).";

fn level(name: &str) -> McpAccess {
    match name {
        "delete_note" => McpAccess::Full,
        "create_note" | "update_note" | "append_to_note" | "set_pinned" | "set_checklist_item" => {
            McpAccess::Write
        }
        _ => McpAccess::Read,
    }
}

fn tools(access: McpAccess) -> Vec<Value> {
    let id = json!({"type": "string", "description": "The note's id (from list_notes or search_notes)."});
    let all = vec![
        json!({"name": "search_notes", "title": "Search notes",
            "description": "Find notes containing words (prefix match on each word). Returns the best matches first.",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}},
                "required": ["query"]},
            "annotations": {"readOnlyHint": true}}),
        json!({"name": "list_notes", "title": "List notes",
            "description": "The user's notes: pinned first, then most recently edited.",
            "inputSchema": {"type": "object", "properties": {
                "pinned_only": {"type": "boolean", "default": false},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}}},
            "annotations": {"readOnlyHint": true}}),
        json!({"name": "get_note", "title": "Read a note", "description": "One note's full text.",
            "inputSchema": {"type": "object", "properties": {"id": id}, "required": ["id"]},
            "annotations": {"readOnlyHint": true}}),
        json!({"name": "create_note", "title": "Create a note",
            "description": "A new note. The first line becomes its title. Set pin to show it as a tab on the screen edge.",
            "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string"}, "pin": {"type": "boolean", "default": false}}, "required": ["text"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false}}),
        json!({"name": "update_note", "title": "Replace a note's text",
            "description": "Replaces the whole text of a note. Read it first and keep what should stay.",
            "inputSchema": {"type": "object", "properties": {"id": id, "text": {"type": "string"}}, "required": ["id", "text"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": true}}),
        json!({"name": "append_to_note", "title": "Add to a note",
            "description": "Adds text on new lines at the end of a note.",
            "inputSchema": {"type": "object", "properties": {"id": id, "text": {"type": "string"}}, "required": ["id", "text"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false}}),
        json!({"name": "set_pinned", "title": "Pin or unpin",
            "description": "Pinned notes show as tabs on the screen edge.",
            "inputSchema": {"type": "object", "properties": {"id": id, "pinned": {"type": "boolean"}}, "required": ["id", "pinned"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false}}),
        json!({"name": "set_checklist_item", "title": "Tick or untick a checklist item",
            "description": "item: the item's number (1 = first checklist item) or its text.",
            "inputSchema": {"type": "object", "properties": {"id": id,
                "item": {"type": ["integer", "string"]}, "done": {"type": "boolean"}}, "required": ["id", "item", "done"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false}}),
        json!({"name": "delete_note", "title": "Delete a note",
            "description": "Deletes a note on every device. Ask the user first.",
            "inputSchema": {"type": "object", "properties": {"id": id}, "required": ["id"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": true}}),
    ];
    // Off still lists the read tools, so the agent can tell the user how to turn access on.
    let allowed = access.max(McpAccess::Read);
    all.into_iter()
        .filter(|t| level(t["name"].as_str().unwrap_or("")) <= allowed)
        .collect()
}

fn call(name: &str, a: &Value, store: &mut Store, access: McpAccess) -> Result<String, String> {
    let need = level(name);
    if access == McpAccess::Off {
        return Err(
            "Sliptab's AI access is off. Ask the user to turn it on in Sliptab → AI agents.".into(),
        );
    }
    if need > access {
        let what = if need == McpAccess::Full {
            "Full access"
        } else {
            "Read & write"
        };
        return Err(format!(
            "Not allowed: this needs \"{what}\" in Sliptab → AI agents (the user chose less)."
        ));
    }
    fn db(e: impl std::fmt::Display) -> String {
        format!("notes database: {e}")
    }
    let text = |k: &str| {
        a[k].as_str()
            .map(str::to_owned)
            .ok_or(format!("missing \"{k}\""))
    };
    let limit = |d: u64, max: u64| a["limit"].as_u64().unwrap_or(d).clamp(1, max) as usize;
    let out = |v: Value| Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    match name {
        "search_notes" => {
            let found = store.search(&text("query")?).map_err(db)?;
            out(json!(found
                .iter()
                .take(limit(10, 50))
                .map(summary)
                .collect::<Vec<_>>()))
        }
        "list_notes" => {
            let pinned = a["pinned_only"].as_bool().unwrap_or(false);
            let notes = store.list().map_err(db)?;
            out(json!(notes
                .iter()
                .filter(|n| !pinned || n.pinned)
                .take(limit(20, 100))
                .map(summary)
                .collect::<Vec<_>>()))
        }
        "get_note" => out(full(&find(store, a)?)),
        "create_note" => {
            let body = text("text")?;
            if body.trim().is_empty() {
                return Err("text is empty".into());
            }
            let mut n = store.create(&body).map_err(db)?;
            if a["pin"].as_bool().unwrap_or(false) {
                n = store.set_pinned(n.id, true).map_err(db)?;
            }
            out(json!({"created": full(&n)}))
        }
        "update_note" => {
            let n = find(store, a)?;
            let body = text("text")?;
            if body.trim().is_empty() {
                return Err("text is empty; use delete_note to remove a note".into());
            }
            out(json!({"updated": full(&store.update_body(n.id, &body).map_err(db)?)}))
        }
        "append_to_note" => {
            let n = find(store, a)?;
            let body = format!("{}\n{}", n.body.trim_end(), text("text")?.trim_end());
            out(json!({"updated": full(&store.update_body(n.id, &body).map_err(db)?)}))
        }
        "set_pinned" => {
            let n = find(store, a)?;
            let pinned = a["pinned"].as_bool().ok_or("missing \"pinned\"")?;
            out(json!({"updated": summary(&store.set_pinned(n.id, pinned).map_err(db)?)}))
        }
        "set_checklist_item" => {
            let n = find(store, a)?;
            let done = a["done"].as_bool().ok_or("missing \"done\"")?;
            let body = set_item(&n.body, &a["item"], done)?;
            out(json!({"updated": full(&store.update_body(n.id, &body).map_err(db)?)}))
        }
        "delete_note" => {
            let n = find(store, a)?;
            store.delete(n.id).map_err(db)?;
            out(json!({"deleted": n.id, "title": n.title}))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

/// The note named by `id`: a full id, or a unique start of one (agents sometimes shorten it).
fn find(store: &Store, a: &Value) -> Result<Note, String> {
    let id = a["id"]
        .as_str()
        .ok_or("missing \"id\"")?
        .trim()
        .to_lowercase();
    if let Ok(u) = id.parse::<Uuid>() {
        if let Some(n) = store
            .get(u)
            .map_err(|e| e.to_string())?
            .filter(|n| n.deleted_at.is_none())
        {
            return Ok(n);
        }
    }
    let matches: Vec<Note> = if id.len() >= 6 {
        store
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|n| n.id.to_string().starts_with(&id))
            .collect()
    } else {
        Vec::new()
    };
    match matches.len() {
        1 => Ok(matches.into_iter().next().expect("one")),
        0 => Err(format!("no note with id {id}")),
        _ => Err(format!("{id} matches several notes; use the full id")),
    }
}

fn items(body: &str) -> Vec<(usize, bool, &str)> {
    body.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim_start();
            let rest = t.strip_prefix("- [ ] ").map(|r| (false, r)).or_else(|| {
                t.strip_prefix("- [x] ")
                    .or_else(|| t.strip_prefix("- [X] "))
                    .map(|r| (true, r))
            });
            rest.map(|(done, text)| (i, done, text))
        })
        .collect()
}

/// Ticks or unticks checklist item `item` (1-based number, or its text) in `body`.
fn set_item(body: &str, item: &Value, done: bool) -> Result<String, String> {
    let list = items(body);
    let line = match item {
        Value::Number(n) => list
            .get(n.as_u64().unwrap_or(0).wrapping_sub(1) as usize)
            .map(|x| x.0),
        Value::String(s) => {
            let s = s.trim().to_lowercase();
            list.iter()
                .find(|x| x.2.trim().to_lowercase() == s)
                .or_else(|| list.iter().find(|x| x.2.to_lowercase().contains(&s)))
                .map(|x| x.0)
        }
        _ => None,
    }
    .ok_or("no such checklist item")?;
    let lines: Vec<String> = body
        .split('\n')
        .enumerate()
        .map(|(i, l)| {
            if i != line {
                return l.to_owned();
            }
            let at = l.find("- [").expect("an item line") + 3;
            format!(
                "{}{}{}",
                &l[..at],
                if done { 'x' } else { ' ' },
                &l[at + 1..]
            )
        })
        .collect();
    Ok(lines.join("\n"))
}

fn summary(n: &Note) -> Value {
    let rest: String = n
        .body
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let preview: String = rest
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect();
    json!({"id": n.id, "title": if n.title.is_empty() { "Untitled" } else { &n.title }, "preview": preview,
        "pinned": n.pinned, "updated": iso(n.updated_at)})
}

fn full(n: &Note) -> Value {
    let list: Vec<Value> = items(&n.body)
        .iter()
        .map(|(_, done, t)| json!({"item": t, "done": done}))
        .collect();
    let files: Vec<Value> = n
        .attachments
        .iter()
        .filter(|a| a.is_live())
        .map(|a| json!({"kind": a.kind, "name": a.name, "bytes": a.bytes}))
        .collect();
    let mut v = json!({"id": n.id, "title": n.title, "text": n.body, "pinned": n.pinned,
        "created": iso(n.created_at), "updated": iso(n.updated_at)});
    if !list.is_empty() {
        v["checklist"] = json!(list);
    }
    if !files.is_empty() {
        v["attachments"] = json!(files);
    }
    v
}

/// Milliseconds since 1970 as "2026-11-23T01:16:29Z".
fn iso(ms: i64) -> String {
    let (secs, days) = (ms.div_euclid(1000), ms.div_euclid(86_400_000));
    let tod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        tod / 60 % 60,
        tod % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rpc(store: &mut Store, access: McpAccess, method: &str, params: Value) -> Value {
        handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            store,
            access,
        )
        .unwrap()
    }
    fn tool(store: &mut Store, access: McpAccess, name: &str, args: Value) -> (bool, String) {
        let r = rpc(
            store,
            access,
            "tools/call",
            json!({"name": name, "arguments": args}),
        );
        (
            r["result"]["isError"].as_bool().unwrap(),
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }
    fn names(store: &mut Store, access: McpAccess) -> Vec<String> {
        rpc(store, access, "tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn each_access_level_allows_only_its_tools() {
        let mut s = Store::open_in_memory("laptop").unwrap();
        let n = s.create("Groceries\n- [ ] milk\n- [ ] atta").unwrap();
        let id = json!({"id": n.id.to_string()});
        assert_eq!(
            names(&mut s, McpAccess::Read),
            ["search_notes", "list_notes", "get_note"]
        );
        assert!(names(&mut s, McpAccess::Write).contains(&"create_note".to_owned()));
        assert!(!names(&mut s, McpAccess::Write).contains(&"delete_note".to_owned()));
        assert!(names(&mut s, McpAccess::Full).contains(&"delete_note".to_owned()));

        let (err, msg) = tool(&mut s, McpAccess::Off, "list_notes", json!({}));
        assert!(err && msg.contains("off"));
        let (err, msg) = tool(
            &mut s,
            McpAccess::Read,
            "search_notes",
            json!({"query": "groc"}),
        );
        assert!(!err && msg.contains("Groceries"), "{msg}");
        assert!(
            tool(&mut s, McpAccess::Read, "create_note", json!({"text": "x"})).0,
            "read cannot write"
        );
        assert!(
            tool(&mut s, McpAccess::Write, "delete_note", id.clone()).0,
            "write cannot delete"
        );

        let (err, msg) = tool(
            &mut s,
            McpAccess::Write,
            "set_checklist_item",
            json!({"id": &n.id.to_string()[..8], "item": "atta", "done": true}),
        );
        assert!(!err, "{msg}");
        assert_eq!(
            s.get(n.id).unwrap().unwrap().body,
            "Groceries\n- [ ] milk\n- [x] atta"
        );
        let (err, _) = tool(
            &mut s,
            McpAccess::Write,
            "create_note",
            json!({"text": "Call the bank", "pin": true}),
        );
        assert!(!err);
        assert!(s
            .list()
            .unwrap()
            .iter()
            .any(|n| n.pinned && n.body == "Call the bank"));
        assert!(!tool(&mut s, McpAccess::Full, "delete_note", id).0);
        assert!(s.list().unwrap().iter().all(|x| x.id != n.id), "deleted");
    }

    #[test]
    fn notifications_get_no_reply_and_dates_are_iso() {
        let mut s = Store::open_in_memory("laptop").unwrap();
        assert!(handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &mut s,
            McpAccess::Read
        )
        .is_none());
        let init = rpc(
            &mut s,
            McpAccess::Read,
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_795_381_589_000), "2026-11-22T21:06:29Z");
    }
}
