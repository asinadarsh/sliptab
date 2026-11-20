//! The app window: a normal, resizable window with the same notes, saves and settings as the
//! edge panel, so Sliptab is never only reachable from the screen edge. It opens on the first run
//! (with onboarding), when the app is launched again while running (Start menu, desktop icon,
//! dock), from the tray icon and from the panel's "open window" button. Closing it hides it;
//! the edge keeps running until Quit in the tray menu.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const LABEL: &str = "home";
/// Passed by the login item, so a login start shows only the edge, not the window.
pub const AUTOSTART_ARG: &str = "--autostart";

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        w.unminimize()?;
        w.show()?;
        return w.set_focus();
    }
    // The root route with a flag; SvelteKit has no /index.html route (that showed a 404).
    let mut url = "?w=home".to_owned();
    // Dev aid: EDGE_TOUR_STEP=3 opens the tour on that step, for screenshots.
    if let Some(n) = std::env::var("EDGE_TOUR_STEP")
        .ok()
        .filter(|_| cfg!(debug_assertions))
    {
        url += &format!("&step={n}");
    }
    // Dev aid: EDGE_OPEN_NOTE=1 opens the first note in the window, for screenshots.
    if cfg!(debug_assertions) && std::env::var_os("EDGE_OPEN_NOTE").is_some() {
        url += "&open=first";
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title("Sliptab")
        .inner_size(980.0, 680.0)
        .min_inner_size(560.0, 480.0)
        .center()
        .focused(true)
        .build()?;
    let hide = w.clone();
    w.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close(); // keep the page (and any typing) alive; the edge keeps running
            let _ = hide.hide();
        }
    });
    Ok(())
}

/// The tray icon: click opens the window; the menu has Open and Quit. Some Linux desktops have
/// no tray, so a failure is logged and the app runs without one.
pub fn tray(app: &AppHandle) {
    let build = || -> tauri::Result<()> {
        let open_item = MenuItem::with_id(app, "open", "Open Sliptab", true, None::<&str>)?;
        let quit_item = MenuItem::with_id(app, "quit", "Quit Sliptab", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_item, &quit_item])?;
        let mut tray = TrayIconBuilder::with_id("edge")
            .tooltip("Sliptab")
            .menu(&menu)
            .show_menu_on_left_click(false);
        if let Ok(icon) = tray_icon() {
            tray = tray.icon(icon);
        }
        tray.on_menu_event(|app, e| match e.id.as_ref() {
            "open" => {
                let _ = open(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|t, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = open(t.app_handle());
            }
        })
        .build(app)?;
        Ok(())
    };
    if let Err(e) = build() {
        eprintln!("tray: {e}");
    }
}

/// The tray icon for the panel's colour: the light one on dark panels, the dark one on light.
fn tray_icon() -> tauri::Result<tauri::image::Image<'static>> {
    let png: &[u8] = if panel_is_light() {
        include_bytes!("../icons/tray-dark.png")
    } else {
        include_bytes!("../icons/tray-light.png")
    };
    tauri::image::Image::from_bytes(png)
}

/// Windows: the taskbar follows "SystemUsesLightTheme" (it can differ from the app theme).
#[cfg(windows)]
fn panel_is_light() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, name) = (
        w(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w("SystemUsesLightTheme"),
    );
    let (mut v, mut n) = (0u32, 4u32);
    // SAFETY: valid NUL-terminated strings and a 4-byte out buffer whose size is passed in n.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut v as *mut u32).cast(),
            &mut n,
        )
    };
    r == 0 && v == 1
}

/// Linux: GNOME's top bar is dark in light and dark mode alike; elsewhere ask the colour-scheme
/// setting. ponytail: KDE/others with a light panel but dark apps get the wrong one; a setting can fix it.
#[cfg(not(windows))]
fn panel_is_light() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    if desktop.contains("gnome") || desktop.contains("unity") || cfg!(target_os = "macos") {
        return false;
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).contains("dark"))
        .unwrap_or(false)
}

/// Re-pick the tray icon after the system theme changes (Windows switches the taskbar with it).
pub fn refresh_tray(app: &AppHandle) {
    if let (Some(t), Ok(icon)) = (app.tray_by_id("edge"), tray_icon()) {
        let _ = t.set_icon(Some(icon));
    }
}

#[tauri::command]
pub fn open_home(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}
