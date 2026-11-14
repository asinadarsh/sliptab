//! Placing the window on the screen edge: a thin bar when collapsed, a tall panel when expanded.
//! All platform differences in edge behaviour live in this file.

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::settings::{Settings, Side};

/// Collapsed bar, in logical pixels.
const BAR_WIDTH: f64 = 8.0;
const BAR_HEIGHT: f64 = 120.0;
/// The expanded panel defaults to a third of the work area; the user can drag it between these.
pub const PANEL_MIN_WIDTH: f64 = 320.0;
const PANEL_MAX_FRACTION: f64 = 0.8;

/// A rectangle in physical pixels: x, y, width, height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Where the window goes, given the monitor's work area (which excludes GNOME's top bar,
/// the Windows taskbar and the macOS menu bar).
pub fn edge_rect(work: Rect, scale: f64, expanded: bool, s: &Settings) -> Rect {
    let (w, h) = if expanded {
        let min = ((PANEL_MIN_WIDTH * scale) as u32).min(work.w);
        let max = ((work.w as f64 * PANEL_MAX_FRACTION) as u32).max(min);
        let want = s.panel_width.map_or(work.w / 3, |w| (w * scale) as u32);
        (want.clamp(min, max), work.h)
    } else {
        ((BAR_WIDTH * scale).round() as u32, ((BAR_HEIGHT * scale).round() as u32).min(work.h))
    };
    let x = match s.side {
        Side::Left => work.x,
        Side::Right => work.x + (work.w - w) as i32,
    };
    let y = if expanded { work.y } else { work.y + ((work.h - h) as f64 * s.bar_position).round() as i32 };
    Rect { x, y, w, h }
}

pub fn place(window: &WebviewWindow, expanded: bool, s: &Settings) -> tauri::Result<Option<Rect>> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(None);
    };
    let wa = monitor.work_area();
    let work = Rect { x: wa.position.x, y: wa.position.y, w: wa.size.width, h: wa.size.height };
    let scale = monitor.scale_factor();
    let r = edge_rect(work, scale, expanded, s);
    #[cfg(target_os = "linux")]
    {
        let panel = edge_rect(work, scale, true, s);
        // Collapsed: only the bar takes the pointer, or nothing at all when the bar is hidden.
        let input = (!expanded).then(|| s.show_bar.then(|| edge_rect(work, scale, false, s)));
        if place_x11(window, panel, input)? {
            return Ok(Some(r));
        }
    }
    window.set_size(PhysicalSize::new(r.w, r.h))?;
    window.set_position(PhysicalPosition::new(r.x, r.y))?;
    if expanded || s.show_bar {
        window.show()?;
    } else {
        window.hide()?; // hotkey-only mode
    }
    Ok(Some(r))
}

/// X11 (incl. GNOME's XWayland): the window always stays panel-sized and transparent, and only its
/// input shape changes. When collapsed, only the bar takes the pointer; clicks elsewhere go through
/// to the windows underneath. Resizing on every open made GNOME show the newly exposed area black
/// for a frame before WebKit painted it, which looked like a flicker. Returns false when not on X11.
#[cfg(target_os = "linux")]
/// `input`: None = expanded, the whole window takes input; Some(None) = collapsed with no bar;
/// Some(Some(bar)) = collapsed, only the bar takes input.
fn place_x11(window: &WebviewWindow, panel: Rect, input: Option<Option<Rect>>) -> tauri::Result<bool> {
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::*;
    let gtk_window = window.gtk_window()?;
    if gtk_window.window().is_none() {
        gtk_window.realize(); // create the X11 window now, before it is first shown
    }
    let Some(gdk_window) = gtk_window.window() else { return Ok(false) };
    if !gdk_window.display().type_().name().contains("X11") {
        return Ok(false);
    }
    // GDK works in its own (integer-scaled) units, Tauri's Rect is in physical pixels.
    let s = gtk_window.scale_factor().max(1);
    let (w, h) = (panel.w as i32 / s, panel.h as i32 / s);
    let (x, y) = ((panel.x + panel.w as i32) / s - w, panel.y / s); // flush with the right edge
    if gtk_window.size() != (w, h) || gdk_window.position() != (x, y) {
        // One configure request for both, so GNOME never sees a half-applied geometry.
        gdk_window.move_resize(x, y, w, h);
        gtk_window.resize(w, h);
        gtk_window.move_(x, y);
    }
    let region = input.map(|bar| match bar {
        None => Region::create(),
        Some(b) => {
            let bw = (b.w as i32 / s).max(1);
            let bh = b.h as i32 / s;
            let bx = if b.x == panel.x { 0 } else { w - bw };
            Region::create_rectangle(&RectangleInt::new(bx, (b.y - panel.y) / s, bw, bh))
        }
    });
    gtk_window.input_shape_combine_region(region.as_ref());
    Ok(true)
}

/// Forward the toplevel window's own pointer crossings to the webview as `edge-pointer` events
/// ("enter" / "leave"). WebKitGTK under XWayland drops many of its DOM mouseleave events, but GTK
/// still receives the X11 crossing. Crossings into child widgets (the webview itself) are ignored.
#[cfg(target_os = "linux")]
pub fn forward_pointer_crossings(window: &WebviewWindow) -> tauri::Result<()> {
    use gtk::gdk::NotifyType;
    use gtk::prelude::*;
    use tauri::Emitter;

    let gtk_window = window.gtk_window()?;
    gtk_window.add_events(gtk::gdk::EventMask::ENTER_NOTIFY_MASK | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK);
    for (kind, leave) in [("enter", false), ("leave", true)] {
        let w = window.clone();
        let handler = move |_: &gtk::ApplicationWindow, e: &gtk::gdk::EventCrossing| {
            if e.detail() != NotifyType::Inferior {
                let _ = w.emit("edge-pointer", kind);
            }
            gtk::glib::Propagation::Proceed
        };
        if leave {
            gtk_window.connect_leave_notify_event(handler);
        } else {
            gtk_window.connect_enter_notify_event(handler);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn forward_pointer_crossings(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

/// Which windowing backend we ended up on. GNOME on Wayland ignores window positioning
/// for native Wayland clients, so `main` switches GNOME to XWayland (see PLATFORMS.md).
pub fn backend() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if std::env::var("GDK_BACKEND").as_deref() == Ok("x11") || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        "x11"
    } else {
        "wayland"
    }
}

/// Run before GTK starts. Override with EDGE_NATIVE_WAYLAND=1 to test native Wayland.
pub fn prefer_xwayland_on_gnome() {
    let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"));
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let xwayland = std::env::var_os("DISPLAY").is_some();
    let overridden = std::env::var_os("EDGE_NATIVE_WAYLAND").is_some() || std::env::var_os("GDK_BACKEND").is_some();
    if gnome && wayland && xwayland && !overridden {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920x1080 screen, GNOME top bar 32px tall.
    const WORK: Rect = Rect { x: 0, y: 32, w: 1920, h: 1048 };

    fn edge_rect(work: Rect, scale: f64, expanded: bool) -> Rect {
        super::edge_rect(work, scale, expanded, &Settings::default())
    }

    #[test]
    fn left_side_and_bar_position() {
        let s = Settings { side: Side::Left, bar_position: 0.0, ..Default::default() };
        assert_eq!(super::edge_rect(WORK, 1.0, false, &s), Rect { x: 0, y: 32, w: 8, h: 120 });
        assert_eq!(super::edge_rect(WORK, 1.0, true, &s).x, 0);
        let bottom = Settings { bar_position: 1.0, ..Default::default() };
        let r = super::edge_rect(WORK, 1.0, false, &bottom);
        assert_eq!(r.y + r.h as i32, 32 + 1048);
    }

    #[test]
    fn dragged_panel_width_is_clamped() {
        let w = |pw: f64| super::edge_rect(WORK, 1.0, true, &Settings { panel_width: Some(pw), ..Default::default() }).w;
        assert_eq!(w(500.0), 500);
        assert_eq!(w(50.0), 320);
        assert_eq!(w(5000.0), 1536);
        let hidpi = Settings { panel_width: Some(500.0), ..Default::default() };
        assert_eq!(super::edge_rect(WORK, 2.0, true, &hidpi).w, 1000);
    }

    #[test]
    fn collapsed_bar_hugs_right_edge_vertically_centred() {
        let r = edge_rect(WORK, 1.0, false);
        assert_eq!(r, Rect { x: 1912, y: 32 + (1048 - 120) / 2, w: 8, h: 120 });
    }

    #[test]
    fn expanded_panel_is_a_third_and_full_height() {
        let r = edge_rect(WORK, 1.0, true);
        assert_eq!(r, Rect { x: 1280, y: 32, w: 640, h: 1048 });
    }

    #[test]
    fn panel_has_a_minimum_width_scaled_for_hidpi() {
        let small = Rect { x: 0, y: 0, w: 1200, h: 800 };
        assert_eq!(edge_rect(small, 2.0, true).w, 640);
        assert_eq!(edge_rect(small, 2.0, false).w, 16);
    }

    #[test]
    fn second_monitor_offset_is_respected() {
        let right_monitor = Rect { x: 1920, y: 0, w: 2560, h: 1440 };
        let r = edge_rect(right_monitor, 1.0, false);
        assert_eq!(r.x, 1920 + 2560 - 8);
    }

    #[test]
    fn never_wider_than_a_tiny_screen() {
        let tiny = Rect { x: 0, y: 0, w: 300, h: 100 };
        let r = edge_rect(tiny, 1.0, true);
        assert_eq!((r.x, r.w), (0, 300));
        assert_eq!(edge_rect(tiny, 1.0, false).h, 100);
    }
}
