//! The global hotkey that opens the panel from any app.
//!
//! Two backends:
//! - GNOME/KDE on Wayland: the XDG GlobalShortcuts portal. The desktop owns the binding: the first time,
//!   it shows a dialog where the user confirms or changes our suggested keys.
//! - Windows, macOS, X11: tauri-plugin-global-shortcut, which grabs the keys directly.
//!
//! Accelerators are stored as "Super+Shift+N" style strings.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

/// Default chosen to avoid common collisions:
/// - Ctrl+Shift+N is taken by browsers (incognito), VS Code (new window) and Nautilus (new folder).
/// - Super+N is GNOME's "focus notification"; Super+Shift+N is unbound in GNOME and not a Windows shortcut.
/// - Ctrl+Alt+<letter> is avoided on Windows/Linux because Ctrl+Alt is AltGr on many keyboard layouts.
/// - macOS: Cmd+Shift+N is "new folder" in Finder; Control+Option+N is unused by the system.
pub const DEFAULT: &str = if cfg!(target_os = "macos") { "Control+Alt+N" } else { "Super+Shift+N" };

/// Emitted to the webview when the hotkey is pressed.
pub const PRESSED_EVENT: &str = "edge-hotkey";
/// Emitted whenever the binding changes (portal binds asynchronously).
pub const STATUS_EVENT: &str = "edge-hotkey-status";

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// What the user asked for, e.g. "Super+Shift+N".
    pub accelerator: String,
    /// What the desktop actually bound, as it describes it. Portal only; None while waiting.
    pub bound: Option<String>,
    /// The desktop decides the keys (portal); our accelerator is only a suggestion.
    pub portal: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Hotkey {
    status: Mutex<Status>,
    #[cfg(target_os = "linux")]
    portal_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

pub fn use_portal() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn status(app: &AppHandle) -> Status {
    app.state::<Hotkey>().status.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_status(app: &AppHandle, status: Status) {
    if cfg!(debug_assertions) {
        eprintln!("hotkey status: {status:?}");
    }
    if let Ok(mut s) = app.state::<Hotkey>().status.lock() {
        *s = status.clone();
    }
    let _ = app.emit(STATUS_EVENT, status);
}

/// Bind `accelerator`. With the plugin, failure (invalid keys, or taken by another app) leaves the
/// previous binding in place and returns the error. With the portal, binding finishes in the background.
pub fn apply(app: &AppHandle, accelerator: &str, previous: Option<&str>) -> Result<Status, String> {
    if use_portal() {
        #[cfg(target_os = "linux")]
        {
            let xdg = to_xdg(accelerator).ok_or_else(|| format!("{accelerator} cannot be used as a desktop shortcut"))?;
            portal::start(app, accelerator.to_owned(), xdg);
        }
    } else {
        #[cfg(desktop)]
        {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let gs = app.global_shortcut();
            let _ = gs.unregister_all();
            if let Err(e) = gs.register(accelerator) {
                let msg = format!("Could not use {accelerator}: {e}. It may be taken by another app.");
                // Keep the old hotkey working; at startup there is none, so record the error for settings.
                let kept = previous.filter(|p| gs.register(*p).is_ok());
                if kept.is_none() {
                    set_status(app, Status { accelerator: accelerator.to_owned(), bound: None, portal: false, error: Some(msg.clone()) });
                }
                return Err(msg);
            }
        }
    }
    let status = Status { accelerator: accelerator.to_owned(), bound: None, portal: use_portal(), error: None };
    set_status(app, status.clone());
    Ok(status)
}

/// Plugin with the press handler, for non-portal platforms.
#[cfg(desktop)]
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_global_shortcut::{Builder, ShortcutState};
    Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                let _ = app.emit(PRESSED_EVENT, ());
            }
        })
        .build()
}

/// "Super+Shift+N" → "LOGO+SHIFT+n", the XDG shortcut format the GlobalShortcuts portal takes.
/// None if there is no modifier, no key, two keys, or a key we do not map.
pub fn to_xdg(accelerator: &str) -> Option<String> {
    let mut mods = Vec::new();
    let mut key = None;
    for part in accelerator.split('+').map(str::trim) {
        let lower = part.to_ascii_lowercase();
        let m = match lower.as_str() {
            "ctrl" | "control" => "CTRL",
            "alt" | "option" => "ALT",
            "shift" => "SHIFT",
            "super" | "meta" | "cmd" | "command" | "win" | "logo" => "LOGO",
            _ => {
                if key.replace(xdg_key(&lower)?).is_some() {
                    return None;
                }
                continue;
            }
        };
        if !mods.contains(&m) {
            mods.push(m);
        }
    }
    let key = key?;
    (!mods.is_empty()).then(|| format!("{}+{key}", mods.join("+")))
}

fn xdg_key(k: &str) -> Option<String> {
    let named = match k {
        "space" => "space",
        "period" => "period",
        "comma" => "comma",
        "slash" => "slash",
        "semicolon" => "semicolon",
        "backquote" => "grave",
        "minus" => "minus",
        "equal" => "equal",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_owned());
    }
    if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(k.to_owned());
    }
    k.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=24).contains(n))
        .map(|n| format!("F{n}"))
}

#[cfg(target_os = "linux")]
mod portal {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use tauri::{AppHandle, Emitter, Manager};

    use super::{set_status, Hotkey, Status, PRESSED_EVENT};

    /// GNOME remembers a binding per shortcut ID and ignores a new preferred trigger for a known ID
    /// (portal v1 has no ConfigureShortcuts). So each key combination gets its own ID.
    fn shortcut_id(xdg: &str) -> String {
        format!("open-panel-{}", xdg.to_ascii_lowercase().replace('+', "-"))
    }

    /// Replace any running portal session with one for `xdg`. The session lives as long as the task.
    pub fn start(app: &AppHandle, accelerator: String, xdg: String) {
        let state = app.state::<Hotkey>();
        let mut task = state.portal_task.lock().expect("hotkey task lock");
        if let Some(old) = task.take() {
            old.abort();
        }
        let app = app.clone();
        *task = Some(tauri::async_runtime::spawn(async move {
            if let Err(e) = run(&app, &accelerator, &xdg).await {
                set_status(
                    &app,
                    Status { accelerator, bound: None, portal: true, error: Some(format!("Desktop shortcut failed: {e}")) },
                );
            }
        }));
    }

    async fn run(app: &AppHandle, accelerator: &str, xdg: &str) -> ashpd::Result<()> {
        // Host (non-Flatpak) apps must tell the portal who they are, or GNOME 50 rejects the binding.
        // GNOME looks the ID up as a .desktop file, which packages install but dev builds/AppImages lack.
        let id = app.config().identifier.clone();
        if let Err(e) = ensure_desktop_entry(&id) {
            eprintln!("hotkey: could not write desktop entry: {e}");
        }
        let app_id = id.as_str().try_into()?;
        ashpd::register_host_app(app_id).await?;

        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session(Default::default()).await?;
        let id = shortcut_id(xdg);
        let shortcut = NewShortcut::new(id.as_str(), "Open the Edge notes panel").preferred_trigger(xdg);
        let bound = portal.bind_shortcuts(&session, &[shortcut], None, Default::default()).await?.response()?;
        let trigger = bound
            .shortcuts()
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.trigger_description().to_owned());
        set_status(app, Status { accelerator: accelerator.to_owned(), bound: trigger, portal: true, error: None });

        let mut activated = portal.receive_activated().await?;
        while let Some(a) = activated.next().await {
            if cfg!(debug_assertions) {
                eprintln!("hotkey activated: {}", a.shortcut_id());
            }
            if a.shortcut_id() == id {
                let _ = app.emit(PRESSED_EVENT, ());
            }
        }
        drop(session);
        Ok(())
    }

    /// Write ~/.local/share/applications/<id>.desktop unless some XDG data dir already has one.
    fn ensure_desktop_entry(id: &str) -> std::io::Result<()> {
        let name = format!("applications/{id}.desktop");
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
            .ok_or_else(|| std::io::Error::other("no HOME"))?;
        let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        if std::iter::once(home.clone())
            .chain(system.split(':').map(std::path::PathBuf::from))
            .any(|d| d.join(&name).exists())
        {
            return Ok(());
        }
        // AppImages run from a temp mount; $APPIMAGE is the real file to launch.
        let exe = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).unwrap_or(std::env::current_exe()?);
        let path = home.join(&name);
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(
            path,
            format!(
                "[Desktop Entry]\nType=Application\nName=Edge\nComment=Notes on the edge of your screen\n\
                 Exec=\"{}\"\nIcon={id}\nTerminal=false\nCategories=Utility;\n",
                exe.display()
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::to_xdg;

    #[test]
    fn default_style_accelerators_convert() {
        assert_eq!(to_xdg("Super+Shift+N").as_deref(), Some("LOGO+SHIFT+n"));
        assert_eq!(to_xdg("Ctrl+Alt+Space").as_deref(), Some("CTRL+ALT+space"));
        assert_eq!(to_xdg("Control+F5").as_deref(), Some("CTRL+F5"));
        assert_eq!(to_xdg("Super+Backquote").as_deref(), Some("LOGO+grave"));
    }

    #[test]
    fn rejects_bare_keys_and_nonsense() {
        assert_eq!(to_xdg("N"), None);
        assert_eq!(to_xdg("Super+Shift"), None);
        assert_eq!(to_xdg("Super+N+M"), None);
        assert_eq!(to_xdg("Super+F25"), None);
        assert_eq!(to_xdg("Super+ArrowUp"), None);
    }

    #[test]
    fn duplicate_modifiers_collapse() {
        assert_eq!(to_xdg("Cmd+Super+K").as_deref(), Some("LOGO+k"));
    }
}
