mod edge;

use std::path::Path;
use std::sync::Mutex;

use notes_core::{Note, Store, Uuid};
use tauri::{Manager, State, WebviewWindow};

pub use edge::prefer_xwayland_on_gnome;

struct Notes(Mutex<Store>);

type CmdResult<T> = Result<T, String>;

fn with_store<T, E: std::fmt::Display>(s: &State<Notes>, f: impl FnOnce(&mut Store) -> Result<T, E>) -> CmdResult<T> {
    let mut store = s.0.lock().map_err(|e| e.to_string())?;
    f(&mut store).map_err(|e| e.to_string())
}

#[tauri::command]
fn notes_list(s: State<Notes>) -> CmdResult<Vec<Note>> {
    with_store(&s, |st| st.list())
}

#[tauri::command]
fn notes_search(s: State<Notes>, query: String) -> CmdResult<Vec<Note>> {
    with_store(&s, |st| st.search(&query))
}

#[tauri::command]
fn note_create(s: State<Notes>, body: String) -> CmdResult<Note> {
    with_store(&s, |st| st.create(&body))
}

#[tauri::command]
fn note_update(s: State<Notes>, id: Uuid, body: String) -> CmdResult<Note> {
    with_store(&s, |st| st.update_body(id, &body))
}

#[tauri::command]
fn note_set_pinned(s: State<Notes>, id: Uuid, pinned: bool) -> CmdResult<Note> {
    with_store(&s, |st| st.set_pinned(id, pinned))
}

#[tauri::command]
fn set_expanded(window: WebviewWindow, expanded: bool, focus: bool) -> CmdResult<()> {
    edge::place(&window, expanded).map_err(|e| e.to_string())?;
    if expanded && focus {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A per-install random ID, so sync can tell which device made an edit.
fn device_id(dir: &Path) -> std::io::Result<String> {
    let path = dir.join("device_id");
    match std::fs::read_to_string(&path) {
        Ok(id) if !id.trim().is_empty() => Ok(id.trim().to_owned()),
        _ => {
            let id = Uuid::now_v7().to_string();
            std::fs::write(&path, &id)?;
            Ok(id)
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(dir.join("notes.db"), device_id(&dir)?)?;
            app.manage(Notes(Mutex::new(store)));

            let window = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
            let placed = edge::place(&window, false)?;
            window.show()?;
            if std::env::var_os("EDGE_PROBE").is_some() {
                // Task 3 probe: did the window manager honour our position?
                let w = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    eprintln!(
                        "EDGE_PROBE backend={} requested={:?} actual_pos={:?} actual_size={:?} on_top_requested=true",
                        edge::backend(),
                        placed,
                        w.outer_position().ok(),
                        w.outer_size().ok()
                    );
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            notes_list,
            notes_search,
            note_create,
            note_update,
            note_set_pinned,
            set_expanded
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
