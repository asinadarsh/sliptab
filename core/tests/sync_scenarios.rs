//! Two devices syncing through a minimal in-memory server. The `Server` here is the reference
//! for the real one (edge-sync-server): store each change under a new sequence number, merging
//! with the stored copy, and answer with everything after the device's cursor.

use std::collections::HashMap;

use notes_core::sync::{merge, SyncRequest, SyncResponse};
use notes_core::{Note, Store, Uuid};

#[derive(Default)]
struct Server {
    notes: HashMap<Uuid, (Note, i64)>,
    seq: i64,
}

impl Server {
    fn sync(&mut self, req: &SyncRequest) -> SyncResponse {
        for c in &req.changes {
            let stored = match self.notes.get(&c.note.id) {
                None => vec![c.note.clone()],
                Some((theirs, _)) => {
                    let m = merge(c.base, &c.note, theirs);
                    std::iter::once(m.note).chain(m.copy).collect()
                }
            };
            for n in stored {
                self.seq += 1;
                self.notes.insert(n.id, (n, self.seq));
            }
        }
        let mut notes: Vec<_> = self
            .notes
            .values()
            .filter(|(_, s)| *s > req.cursor)
            .cloned()
            .collect();
        notes.sort_by_key(|(_, s)| *s);
        SyncResponse {
            cursor: self.seq,
            notes: notes.into_iter().map(|(n, _)| n).collect(),
            more: false,
        }
    }
}

fn sync(dev: &mut Store, server: &mut Server) {
    let req = dev.sync_request().unwrap();
    let resp = server.sync(&req);
    dev.apply_sync(&req, &resp).unwrap();
}

/// Sync both until nothing is left to send, then compare what each shows.
fn settle(a: &mut Store, b: &mut Store, server: &mut Server) {
    for _ in 0..3 {
        sync(a, server);
        sync(b, server);
    }
    assert!(a.pending_sync().unwrap().is_empty() && b.pending_sync().unwrap().is_empty());
    assert_eq!(view(a), view(b), "devices disagree");
}

fn view(s: &Store) -> Vec<(String, bool, Option<Uuid>)> {
    let mut v: Vec<_> = s
        .list()
        .unwrap()
        .into_iter()
        .map(|n| (n.body, n.pinned, n.conflict_of))
        .collect();
    v.sort();
    v
}

fn devices() -> (Store, Store, Server) {
    (
        Store::open_in_memory("phone").unwrap(),
        Store::open_in_memory("laptop").unwrap(),
        Server::default(),
    )
}

#[test]
fn a_phone_note_reaches_the_laptop() {
    let (mut phone, mut laptop, mut server) = devices();
    phone.create("Buy milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(laptop.list().unwrap()[0].body, "Buy milk");
}

#[test]
fn offline_edits_of_the_same_text_keep_both() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    phone.update_body(n.id, "milk and eggs").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2)); // laptop edits later
    laptop.update_body(n.id, "oat milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    let notes = phone.list().unwrap();
    assert_eq!(notes.len(), 2);
    let main = notes.iter().find(|x| x.id == n.id).unwrap();
    let copy = notes.iter().find(|x| x.id != n.id).unwrap();
    assert_eq!(main.body, "oat milk");
    assert_eq!(
        (copy.body.as_str(), copy.conflict_of),
        ("milk and eggs", Some(n.id))
    );
}

#[test]
fn pin_on_laptop_and_edit_on_phone_merge_without_a_copy() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    laptop.set_pinned(n.id, true).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2)); // the text edit is the later one
    phone.update_body(n.id, "milk and eggs").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(
        view(&phone),
        vec![("milk and eggs".to_string(), true, None)]
    );
}

#[test]
fn an_edit_on_one_device_beats_a_delete_on_the_other() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    phone.delete(n.id).unwrap();
    laptop.update_body(n.id, "milk and eggs").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(
        view(&phone),
        vec![("milk and eggs".to_string(), false, None)]
    );
}

#[test]
fn a_delete_reaches_the_other_device() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    laptop.delete(n.id).unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert!(phone.list().unwrap().is_empty());
}

#[test]
fn a_retried_request_does_not_duplicate_notes() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    phone.update_body(n.id, "milk and eggs").unwrap();
    let req = phone.sync_request().unwrap();
    server.sync(&req); // the answer is lost
    let resp = server.sync(&req); // the phone retries
    phone.apply_sync(&req, &resp).unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(
        view(&laptop),
        vec![("milk and eggs".to_string(), false, None)]
    );
}

#[test]
fn typing_during_a_sync_is_not_lost() {
    let (mut phone, mut laptop, mut server) = devices();
    let n = phone.create("milk").unwrap();
    let req = phone.sync_request().unwrap();
    let resp = server.sync(&req);
    phone.update_body(n.id, "milk and eggs").unwrap(); // while the request was in flight
    phone.apply_sync(&req, &resp).unwrap();
    assert_eq!(phone.get(n.id).unwrap().unwrap().body, "milk and eggs");
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(
        view(&laptop),
        vec![("milk and eggs".to_string(), false, None)]
    );
}

#[test]
fn notes_made_offline_on_both_devices_all_arrive() {
    let (mut phone, mut laptop, mut server) = devices();
    phone.create("from phone").unwrap();
    laptop.create("from laptop").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    assert_eq!(phone.list().unwrap().len(), 2);
}

#[test]
fn a_save_from_the_phone_reaches_the_laptop_with_its_folder() {
    let (mut phone, mut laptop, mut server) = devices();
    let (s, new) = phone
        .create_save("https://www.instagram.com/reel/DftSYUwtKNT/?vrfl=abc")
        .unwrap();
    assert!(new);
    phone.set_save_folder(s.id, Some("Learn")).unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    let got = laptop.saves().unwrap();
    assert_eq!(got.len(), 1);
    let link = got[0].link.clone().unwrap();
    assert_eq!(
        (link.url.as_str(), link.platform.as_str()),
        ("https://www.instagram.com/reel/DftSYUwtKNT", "instagram")
    );
    assert_eq!(link.folder.as_deref(), Some("Learn"));
    assert!(
        laptop.list().unwrap().is_empty(),
        "a save is not listed among notes"
    );
}

#[test]
fn folder_on_one_device_and_done_after_sync_on_the_other_both_survive() {
    let (mut phone, mut laptop, mut server) = devices();
    let (s, _) = phone.create_save("https://x.com/jack/status/20").unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    phone.set_save_folder(s.id, Some("Ideas")).unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    laptop.set_save_done(s.id, true).unwrap();
    laptop.mark_opened(s.id).unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    let l = phone.get(s.id).unwrap().unwrap().link.unwrap();
    assert_eq!(
        (l.folder.as_deref(), l.done, l.opened_at.is_some()),
        (Some("Ideas"), true, true)
    );
}

#[test]
fn a_folder_change_and_a_text_edit_offline_merge_without_a_copy() {
    let (mut phone, mut laptop, mut server) = devices();
    let (s, _) = phone
        .create_save("look https://youtu.be/hdMszZ2ah5Q")
        .unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    phone.set_save_folder(s.id, Some("Watch Later")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    laptop
        .update_body(s.id, "look https://youtu.be/hdMszZ2ah5Q  for the talk")
        .unwrap();
    settle(&mut phone, &mut laptop, &mut server);
    let saves = phone.saves().unwrap();
    assert_eq!(saves.len(), 1, "no conflict copy");
    assert_eq!(
        saves[0].link.as_ref().unwrap().folder.as_deref(),
        Some("Watch Later")
    );
    assert!(saves[0].body.ends_with("for the talk"));
}

#[test]
fn fields_from_a_newer_version_survive_an_older_device_editing_the_note() {
    // A newer app wrote `reminder_at` on the note and `canonical` on its link. This version
    // doesn't know either; it must keep them through storing, editing and syncing back.
    let (mut phone, _, mut server) = devices();
    let newer: Note = serde_json::from_value(serde_json::json!({
        "id": Uuid::now_v7(), "title": "", "body": "https://www.threads.com/share/_oANR7c7l",
        "pinned": false, "archived": false, "deleted_at": null, "created_at": 1, "updated_at": 1,
        "device_id": "newer-laptop", "version": 1, "conflict_of": null, "color": null,
        "reminder_at": 1794700000000_i64,
        "link": { "url": "https://www.threads.com/share/_oANR7c7l", "platform": "threads",
                  "canonical": "https://www.threads.com/@nick_saraev/post/Dfui2UBE4Sv" }
    }))
    .unwrap();
    server.sync(&SyncRequest {
        cursor: 0,
        changes: vec![notes_core::sync::Change {
            note: newer.clone(),
            base: None,
        }],
    });
    sync(&mut phone, &mut server);
    phone
        .update_body(
            newer.id,
            "https://www.threads.com/share/_oANR7c7l  read later",
        )
        .unwrap();
    sync(&mut phone, &mut server);
    let stored = serde_json::to_value(&server.notes[&newer.id].0).unwrap();
    assert_eq!(stored["reminder_at"], 1794700000000_i64);
    assert_eq!(
        stored["link"]["canonical"],
        "https://www.threads.com/@nick_saraev/post/Dfui2UBE4Sv"
    );
    assert!(stored["body"].as_str().unwrap().ends_with("read later"));
}

#[test]
fn oversized_unknown_fields_from_a_device_are_dropped() {
    let mut n: Note = serde_json::from_value(serde_json::json!({
        "id": Uuid::now_v7(), "title": "", "body": "x", "pinned": false, "archived": false,
        "deleted_at": null, "created_at": 1, "updated_at": 1, "device_id": "d", "version": 1,
        "conflict_of": null, "color": null, "junk": "y".repeat(notes_core::MAX_EXTRA_BYTES), "small": 1
    }))
    .unwrap();
    n.cap_extra();
    assert!(n.extra.is_empty());
}
