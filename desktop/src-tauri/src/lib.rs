mod edge;
mod hotkey;
mod settings;
mod sync;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use notes_core::{Note, Store, Uuid};
use tauri::{Manager, State, WebviewWindow};

pub use edge::prefer_xwayland_on_gnome;

struct Notes(Mutex<Store>);

use sync::SyncState;

/// Settings plus the directory they are saved in.
struct AppSettings {
    dir: PathBuf,
    current: Mutex<settings::Settings>,
}

/// Edge state a settings change needs to re-place the window: expanded or not, and how many
/// pinned-note tabs the strip shows.
#[derive(Default)]
struct EdgeState {
    expanded: AtomicBool,
    tabs: AtomicUsize,
    peek: Mutex<Option<edge::Local>>,
}

impl EdgeState {
    fn place(&self, window: &WebviewWindow, s: &settings::Settings) -> CmdResult<()> {
        let (expanded, tabs) = (self.expanded.load(Ordering::Relaxed), self.tabs.load(Ordering::Relaxed));
        let peek = *self.peek.lock().map_err(|e| e.to_string())?;
        edge::place(window, expanded, s, tabs, peek).map(|_| ()).map_err(|e| e.to_string())
    }
}

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
fn note_create(s: State<Notes>, body: String, sync: State<SyncState>) -> CmdResult<Note> {
    let n = with_store(&s, |st| st.create(&body))?;
    sync.kick(false);
    Ok(n)
}

#[tauri::command]
fn note_update(s: State<Notes>, id: Uuid, body: String, sync: State<SyncState>) -> CmdResult<Note> {
    let n = with_store(&s, |st| st.update_body(id, &body))?;
    sync.kick(false);
    Ok(n)
}

#[tauri::command]
fn note_set_pinned(s: State<Notes>, id: Uuid, pinned: bool, sync: State<SyncState>) -> CmdResult<Note> {
    let n = with_store(&s, |st| st.set_pinned(id, pinned))?;
    sync.kick(false);
    Ok(n)
}

#[tauri::command]
fn note_set_color(s: State<Notes>, id: Uuid, color: Option<String>, sync: State<SyncState>) -> CmdResult<Note> {
    let n = with_store(&s, |st| st.set_color(id, color.as_deref()))?;
    sync.kick(false);
    Ok(n)
}

#[tauri::command]
fn set_expanded(
    window: WebviewWindow,
    s: State<AppSettings>,
    state: State<EdgeState>,
    sync: State<SyncState>,
    expanded: bool,
    focus: bool,
) -> CmdResult<()> {
    if expanded {
        sync.kick(true); // fresh notes from the phone as the panel opens
    }
    state.expanded.store(expanded, Ordering::Relaxed);
    state.place(&window, &current_settings(&s)?)?;
    if expanded && focus {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    if !expanded && window.is_focused().unwrap_or(false) {
        edge::release_focus(&window).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// How the edge window behaves on this platform.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    /// Hover no longer takes keyboard focus anywhere: GTK's own crossings make mouse-out reliable,
    /// and focusing on hover sent typing into the panel after it had closed.
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
        focus_on_hover: false,
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

/// Clicking a dock window does not focus it on GNOME, so the panel asks when you click in it.
#[tauri::command]
fn focus_panel(window: WebviewWindow) -> CmdResult<()> {
    window.set_focus().map_err(|e| e.to_string())
}

/// The page reports how many tabs it draws, so the window/input area matches the strip.
#[tauri::command]
fn set_tab_count(window: WebviewWindow, s: State<AppSettings>, state: State<EdgeState>, count: usize) -> CmdResult<()> {
    if state.tabs.swap(count.min(edge::MAX_TABS), Ordering::Relaxed) != count.min(edge::MAX_TABS) {
        state.place(&window, &current_settings(&s)?)?;
    }
    Ok(())
}

/// The hovered tab's preview card (None when hidden), so it can take clicks while collapsed.
#[tauri::command]
fn set_peek(window: WebviewWindow, s: State<AppSettings>, state: State<EdgeState>, rect: Option<edge::Local>) -> CmdResult<()> {
    let mut peek = state.peek.lock().map_err(|e| e.to_string())?;
    if *peek == rect {
        return Ok(());
    }
    *peek = rect;
    drop(peek);
    state.place(&window, &current_settings(&s)?)
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
    state: State<EdgeState>,
    new: settings::Settings,
) -> CmdResult<settings::Settings> {
    let saved = {
        let mut current = s.current.lock().map_err(|e| e.to_string())?;
        *current = settings::Settings { hotkey: current.hotkey.clone(), ..new }.sanitized();
        settings::save(&s.dir, &current).map_err(|e| e.to_string())?;
        current.clone()
    };
    state.place(&window, &saved)?;
    Ok(saved)
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
    let builder = tauri::Builder::default().manage(hotkey::Hotkey::default()).manage(EdgeState::default());
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
            sync::start(app.handle(), &dir);
            let loaded = settings::load(&dir);
            if let Err(e) = hotkey::apply(app.handle(), &loaded.hotkey, None) {
                eprintln!("hotkey: {e}"); // shown in settings via get_hotkey's error
            }
            let window = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
            let placed = edge::place(&window, false, &loaded, 0, None)?;
            let probe_settings = loaded.clone();
            app.manage(AppSettings { dir: dir.clone(), current: Mutex::new(loaded) });
            edge::forward_pointer_crossings(&window)?;
            if probe_settings.show_bar || edge::backend() == "x11" {
                window.show()?; // X11 hides the bar with an empty input shape instead of unmapping
            }
            // Dev aid: open the panel as the hotkey would, so a screenshot can show it.
            if cfg!(debug_assertions) && std::env::var_os("EDGE_OPEN_ON_START").is_some() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    let _ = tauri::Emitter::emit(&handle, hotkey::PRESSED_EVENT, ());
                    if std::env::var("EDGE_OPEN_ON_START").as_deref() == Ok("toggle") {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        let _ = tauri::Emitter::emit(&handle, hotkey::PRESSED_EVENT, ());
                    }
                });
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
                            let _ = tx.send(edge::place(&w2, expanded, &ps, 0, None).ok().flatten());
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
            note_set_color,
            set_tab_count,
            focus_panel,
            set_peek,
            set_expanded,
            debug_log,
            platform,
            get_hotkey,
            set_hotkey,
            get_settings,
            save_settings,
            get_autostart,
            set_autostart,
            sync::sync_status,
            sync::sync_sign_in_start,
            sync::sync_sign_in_verify,
            sync::sync_sign_out,
            sync::sync_now
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
