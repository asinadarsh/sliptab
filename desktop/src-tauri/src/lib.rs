mod edge;
mod hotkey;
mod settings;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use notes_core::{Note, Store, Uuid};
use tauri::{Manager, State, WebviewWindow};

pub use edge::prefer_xwayland_on_gnome;

struct Notes(Mutex<Store>);

/// Settings plus the directory they are saved in.
struct AppSettings {
    dir: PathBuf,
    current: Mutex<settings::Settings>,
}

/// Whether the panel is currently expanded, so a settings change can re-place the window correctly.
#[derive(Default)]
struct Expanded(AtomicBool);

type CmdResult<T> = Result<T, String>;

fn current_settings(s: &State<AppSettings>) -> CmdResult<settings::Settings> {
    s.current.lock().map(|c| c.clone()).map_err(|e| e.to_string())
}

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
fn set_expanded(
    window: WebviewWindow,
    s: State<AppSettings>,
    state: State<Expanded>,
    expanded: bool,
    focus: bool,
) -> CmdResult<()> {
    state.0.store(expanded, Ordering::Relaxed);
    edge::place(&window, expanded, &current_settings(&s)?).map_err(|e| e.to_string())?;
    if expanded && focus {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// How the edge window behaves on this platform.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    /// GNOME/XWayland loses the pointer once it moves onto a Wayland window, so "mouse left" can be
    /// missed. There the panel takes focus when it opens and also closes on blur.
    focus_on_hover: bool,
    /// X11: the window never resizes (only its input shape changes), so the panel can start sliding
    /// before Rust has answered. Elsewhere the window must grow first.
    fixed_window: bool,
    os: &'static str,
}

#[tauri::command]
fn platform() -> Platform {
    let x11 = cfg!(target_os = "linux") && edge::backend() == "x11";
    Platform {
        focus_on_hover: x11 && std::env::var_os("WAYLAND_DISPLAY").is_some(),
        fixed_window: x11,
        os: if cfg!(target_os = "macos") { "macos" } else if cfg!(windows) { "windows" } else { "linux" },
    }
}

#[tauri::command]
fn get_hotkey(app: tauri::AppHandle) -> hotkey::Status {
    hotkey::status(&app)
}

/// Bind a new hotkey and save it. On failure the old hotkey stays bound and saved.
#[tauri::command]
fn set_hotkey(app: tauri::AppHandle, s: State<AppSettings>, accelerator: String) -> CmdResult<hotkey::Status> {
    let previous = s.current.lock().map_err(|e| e.to_string())?.hotkey.clone();
    let status = hotkey::apply(&app, &accelerator, Some(&previous))?;
    let mut current = s.current.lock().map_err(|e| e.to_string())?;
    current.hotkey = accelerator;
    settings::save(&s.dir, &current).map_err(|e| format!("Hotkey set, but saving settings failed: {e}"))?;
    Ok(status)
}

#[tauri::command]
fn get_settings(s: State<AppSettings>) -> CmdResult<settings::Settings> {
    current_settings(&s)
}

/// Save everything except the hotkey (set_hotkey owns that) and move the window to match.
#[tauri::command]
fn save_settings(
    window: WebviewWindow,
    s: State<AppSettings>,
    state: State<Expanded>,
    new: settings::Settings,
) -> CmdResult<settings::Settings> {
    let saved = {
        let mut current = s.current.lock().map_err(|e| e.to_string())?;
        *current = settings::Settings { hotkey: current.hotkey.clone(), ..new }.sanitized();
        settings::save(&s.dir, &current).map_err(|e| e.to_string())?;
        current.clone()
    };
    edge::place(&window, state.0.load(Ordering::Relaxed), &saved).map_err(|e| e.to_string())?;
    Ok(saved)
}

/// Live resize while the user drags the panel edge; `save` on release only, to spare the disk.
#[tauri::command]
fn set_panel_width(window: WebviewWindow, s: State<AppSettings>, width: f64, save: bool) -> CmdResult<()> {
    let snapshot = {
        let mut current = s.current.lock().map_err(|e| e.to_string())?;
        current.panel_width = Some(width.max(edge::PANEL_MIN_WIDTH));
        if save {
            settings::save(&s.dir, &current).map_err(|e| e.to_string())?;
        }
        current.clone()
    };
    edge::place(&window, true, &snapshot).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> CmdResult<bool> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())?;
    al.is_enabled().map_err(|e| e.to_string())
}

/// Lets the webview write to the dev console (debug builds only; a no-op in release).
#[tauri::command]
fn debug_log(msg: String) {
    if cfg!(debug_assertions) {
        eprintln!("[web] {msg}");
    }
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
    let builder = tauri::Builder::default().manage(hotkey::Hotkey::default()).manage(Expanded::default());
    #[cfg(desktop)]
    let builder = builder
        .plugin(hotkey::plugin())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None));
    builder
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(dir.join("notes.db"), device_id(&dir)?)?;
            app.manage(Notes(Mutex::new(store)));
            let loaded = settings::load(&dir);
            if let Err(e) = hotkey::apply(app.handle(), &loaded.hotkey, None) {
                eprintln!("hotkey: {e}"); // shown in settings via get_hotkey's error
            }
            let window = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
            let placed = edge::place(&window, false, &loaded)?;
            let probe_settings = loaded.clone();
            app.manage(AppSettings { dir: dir.clone(), current: Mutex::new(loaded) });
            edge::forward_pointer_crossings(&window)?;
            if probe_settings.show_bar || edge::backend() == "x11" {
                window.show()?; // X11 hides the bar with an empty input shape instead of unmapping
            }
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
                    // Expand then collapse, reporting where the window really ends up each time.
                    for expanded in [true, false] {
                        // GTK must be called on the main thread.
                        let (tx, rx) = std::sync::mpsc::channel();
                        let w2 = w.clone();
                        let ps = probe_settings.clone();
                        let _ = w.run_on_main_thread(move || {
                            let _ = tx.send(edge::place(&w2, expanded, &ps).ok().flatten());
                        });
                        let r = rx.recv().ok().flatten();
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        eprintln!(
                            "EDGE_PROBE expanded={expanded} requested={r:?} actual_pos={:?} actual_size={:?}",
                            w.outer_position().ok(),
                            w.outer_size().ok()
                        );
                    }
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
            set_expanded,
            debug_log,
            platform,
            get_hotkey,
            set_hotkey,
            get_settings,
            save_settings,
            set_panel_width,
            get_autostart,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
