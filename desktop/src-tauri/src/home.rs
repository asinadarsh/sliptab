//! The app window: a normal, resizable window with the same notes, saves and settings as the
//! edge panel, so Edge is never only reachable from the screen edge. It opens on the first run
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
    if let Some(n) = std::env::var("EDGE_TOUR_STEP").ok().filter(|_| cfg!(debug_assertions)) {
        url += &format!("&step={n}");
    }
    // Dev aid: EDGE_OPEN_NOTE=1 opens the first note in the window, for screenshots.
    if cfg!(debug_assertions) && std::env::var_os("EDGE_OPEN_NOTE").is_some() {
        url += "&open=first";
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url.into()))
        .title("Edge")
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
        let open_item = MenuItem::with_id(app, "open", "Open Edge", true, None::<&str>)?;
        let quit_item = MenuItem::with_id(app, "quit", "Quit Edge", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_item, &quit_item])?;
        let mut tray = TrayIconBuilder::with_id("edge").tooltip("Edge").menu(&menu).show_menu_on_left_click(false);
        if let Some(icon) = app.default_window_icon() {
            tray = tray.icon(icon.clone());
        }
        tray.on_menu_event(|app, e| match e.id.as_ref() {
            "open" => {
                let _ = open(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|t, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
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

#[tauri::command]
pub fn open_home(app: AppHandle) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}
