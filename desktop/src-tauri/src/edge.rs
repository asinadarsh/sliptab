//! Placing the window on the screen edge: a thin bar when collapsed, a tall panel when expanded.
//! All platform differences in edge behaviour live in this file.

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::settings::{Settings, Side};

/// Collapsed edge, in logical pixels: pinned-note tabs stacked above the main bar.
/// The webview draws the same strip; keep these in step with STRIP in +page.svelte.
const BAR_WIDTH: f64 = 8.0;
const BAR_HEIGHT: f64 = 120.0;
const TAB_HEIGHT: f64 = 36.0;
const TAB_GAP: f64 = 6.0;
/// Six note tabs, one "+N" tab and the saves-for-today tab at most.
pub const MAX_TABS: usize = 8;

fn strip_height(tabs: usize) -> f64 {
    BAR_HEIGHT + tabs.min(MAX_TABS) as f64 * (TAB_HEIGHT + TAB_GAP)
}
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
pub fn edge_rect(work: Rect, scale: f64, expanded: bool, s: &Settings, tabs: usize) -> Rect {
    let (w, h) = if expanded {
        let min = ((PANEL_MIN_WIDTH * scale) as u32).min(work.w);
        let max = ((work.w as f64 * PANEL_MAX_FRACTION) as u32).max(min);
        let want = s.panel_width.map_or(work.w / 3, |w| (w * scale) as u32);
        (want.clamp(min, max), work.h)
    } else {
        (
            (BAR_WIDTH * scale).round() as u32,
            ((strip_height(tabs) * scale).round() as u32).min(work.h),
        )
    };
    let x = match s.side {
        Side::Left => work.x,
        Side::Right => work.x + (work.w - w) as i32,
    };
    let y = if expanded {
        work.y
    } else {
        work.y + ((work.h - h) as f64 * s.bar_position).round() as i32
    };
    Rect { x, y, w, h }
}

/// A rectangle in window-local logical pixels (CSS px == GDK units), e.g. the tab preview card.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct Local {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// `peek`: while collapsed, the hovered tab's preview card also takes the pointer so it can be clicked.
pub fn place(
    window: &WebviewWindow,
    expanded: bool,
    s: &Settings,
    tabs: usize,
    peek: Option<Local>,
) -> tauri::Result<Option<Rect>> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(None);
    };
    let wa = monitor.work_area();
    let work = Rect {
        x: wa.position.x,
        y: wa.position.y,
        w: wa.size.width,
        h: wa.size.height,
    };
    let scale = monitor.scale_factor();
    let r = edge_rect(work, scale, expanded, s, tabs);
    #[cfg(target_os = "linux")]
    {
        let panel = edge_rect(work, scale, true, s, tabs);
        // Collapsed: only the bar takes the pointer, or nothing at all when the bar is hidden.
        let input = (!expanded).then(|| s.show_bar.then(|| edge_rect(work, scale, false, s, tabs)));
        if place_x11(window, panel, input, peek)? {
            return Ok(Some(r));
        }
    }
    #[cfg(windows)]
    {
        let panel = edge_rect(work, scale, true, s, tabs);
        let input = (!expanded).then(|| s.show_bar.then(|| edge_rect(work, scale, false, s, tabs)));
        place_windows(window, panel, input, peek, scale)?;
        Ok(Some(r))
    }
    #[cfg(not(windows))]
    {
        // ponytail: macOS (and Wayland without XWayland) still resize the window to the bar, so
        // the preview card has no room to be clickable; macOS gets its own edge window later.
        let _ = peek;
        place_by_resizing(window, r, expanded || s.show_bar)?;
        Ok(Some(r))
    }
}

#[cfg(not(windows))]
fn place_by_resizing(window: &WebviewWindow, r: Rect, visible: bool) -> tauri::Result<()> {
    window.set_size(PhysicalSize::new(r.w, r.h))?;
    window.set_position(PhysicalPosition::new(r.x, r.y))?;
    if visible {
        window.show()?;
    } else {
        window.hide()?; // hotkey-only mode
    }
    Ok(())
}

/// X11 (incl. GNOME's XWayland): the window always stays panel-sized and transparent, and only its
/// input shape changes. When collapsed, only the bar takes the pointer; clicks elsewhere go through
/// to the windows underneath. Resizing on every open made GNOME show the newly exposed area black
/// for a frame before WebKit painted it, which looked like a flicker. Returns false when not on X11.
#[cfg(target_os = "linux")]
/// `input`: None = expanded, the whole window takes input; Some(None) = collapsed with no bar;
/// Some(Some(bar)) = collapsed, only the bar takes input.
fn place_x11(
    window: &WebviewWindow,
    panel: Rect,
    input: Option<Option<Rect>>,
    peek: Option<Local>,
) -> tauri::Result<bool> {
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::*;
    let gtk_window = window.gtk_window()?;
    if gtk_window.window().is_none() {
        // A dock, not a normal window: GNOME's "show desktop" (Super+D) hides every normal window,
        // and docks can't be minimised. Must be set before the window is mapped.
        gtk_window.set_type_hint(gtk::gdk::WindowTypeHint::Dock);
        gtk_window.realize(); // create the X11 window now, before it is first shown
    }
    let Some(gdk_window) = gtk_window.window() else {
        return Ok(false);
    };
    if !gdk_window.display().type_().name().contains("X11") {
        return Ok(false);
    }
    // Tao starts a window-manager resize when the left button goes down within 5px of the edge
    // of an undecorated, resizable window, and our bar sits on that edge: press-and-drag resized
    // the window and moved the bar. Tao skips that for "decorated" windows, so mark the GTK
    // window decorated and tell the window manager directly to draw no frame. (It must stay
    // resizable: otherwise GTK pins its size.)
    if !gtk_window.is_decorated() {
        gtk_window.set_decorated(true);
    }
    gdk_window.set_decorations(gtk::gdk::WMDecoration::empty());
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
            let region =
                Region::create_rectangle(&RectangleInt::new(bx, (b.y - panel.y) / s, bw, bh));
            if let Some(p) = peek {
                let r = RectangleInt::new(
                    p.x.floor() as i32,
                    p.y.floor() as i32,
                    p.w.ceil() as i32,
                    p.h.ceil() as i32,
                );
                let _ = region.union_rectangle(&r);
            }
            region
        }
    });
    gtk_window.input_shape_combine_region(region.as_ref());
    Ok(true)
}

/// Windows: the same idea as X11. The window always stays panel-sized and transparent; its window
/// region is just the bar when collapsed (clicks outside it reach the windows underneath, as the
/// region is all of the window that exists for hit-testing) and the whole panel when expanded.
/// Never resizing means opening never shows an unpainted frame. Also makes it a tool window, so it
/// stays out of Alt+Tab and the taskbar.
/// `input`: None = expanded; Some(None) = collapsed, bar hidden; Some(Some(bar)) = collapsed.
#[cfg(windows)]
fn place_windows(
    window: &WebviewWindow,
    panel: Rect,
    input: Option<Option<Rect>>,
    peek: Option<Local>,
    scale: f64,
) -> tauri::Result<()> {
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    // A resizable borderless window gets Tauri's invisible drag-to-resize strip along its edges
    // (a child window, TAURI_DRAG_RESIZE_BORDERS). Our bar sits on that edge, so the strip took
    // every hover and click. The panel is not user-resizable, and set_size still works.
    if window.is_resizable()? {
        window.set_resizable(false)?;
    }
    let hwnd = window.hwnd()?.0 as windows_sys::Win32::Foundation::HWND;
    let size = window.outer_size()?;
    let pos = window.outer_position()?;
    if (size.width, size.height) != (panel.w, panel.h) {
        window.set_size(PhysicalSize::new(panel.w, panel.h))?;
    }
    if (pos.x, pos.y) != (panel.x, panel.y) {
        window.set_position(PhysicalPosition::new(panel.x, panel.y))?;
    }
    // SAFETY: hwnd is this app's live top-level window, on the main thread (Tauri commands for
    // the window run there). Regions passed to SetWindowRgn belong to the system afterwards;
    // the temporary peek region is deleted here.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
        if ex != want {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
        }
        let region = match input {
            None => std::ptr::null_mut(), // the whole window
            Some(None) => CreateRectRgn(0, 0, 0, 0),
            Some(Some(b)) => {
                let bx = if b.x == panel.x {
                    0
                } else {
                    panel.w as i32 - b.w as i32
                };
                let by = b.y - panel.y;
                let r = CreateRectRgn(bx, by, bx + b.w as i32, by + b.h as i32);
                if let Some(p) = peek {
                    let px = |v: f64| (v * scale).round() as i32;
                    // On Windows the region clips drawing too, not only the pointer: a region the
                    // card's exact size cut off its shadow and squared its rounded corners. Leave
                    // room for the shadow (.peek: 0 10px 30px), inside the window.
                    let pad = px(PEEK_SHADOW);
                    let card = CreateRectRgn(
                        (px(p.x) - pad).max(0),
                        (px(p.y) - pad).max(0),
                        (px(p.x + p.w) + pad).min(panel.w as i32),
                        (px(p.y + p.h) + pad + px(10.0)).min(panel.h as i32),
                    );
                    CombineRgn(r, r, card, RGN_OR);
                    DeleteObject(card);
                }
                r
            }
        };
        SetWindowRgn(hwnd, region, 1);
    }
    window.show()?;
    keep_on_top(window)?;
    Ok(())
}

/// Windows: put the bar back above every normal window if anything has pushed it down. The flag
/// set at start-up is not permanent: Windows drops it, or another window takes the top spot, and
/// the bar then hides behind whatever is maximized (seen with Chrome and a terminal). Cheap when
/// the bar is already on top, so it runs after every placement and once a second.
#[cfg(windows)]
pub fn keep_on_top(window: &WebviewWindow) -> tauri::Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowLongPtrW, IsWindowVisible, SetWindowPos, GWL_EXSTYLE, GW_HWNDPREV,
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
    };
    let hwnd = window.hwnd()?.0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: hwnd is this app's live top-level window; these calls only read its state and ask
    // for a z-order change without moving, resizing or activating it.
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return Ok(());
        }
        let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
        let nothing_above = GetWindow(hwnd, GW_HWNDPREV).is_null();
        if !(topmost && nothing_above) {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    Ok(())
}

/// Room around the peek card for its shadow, in CSS px (the shadow's blur, .peek in +page.svelte).
#[cfg(windows)]
const PEEK_SHADOW: f64 = 30.0;

/// The window that had focus before the panel took it, to give focus back on collapse.
#[cfg(windows)]
static PREVIOUS_FOREGROUND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Takes keyboard focus for the panel, remembering on Windows which window had it.
pub fn take_focus(window: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let ours = window.hwnd()?.0 as isize;
        // SAFETY: reads which window is in front; no pointer is dereferenced.
        let front = unsafe { GetForegroundWindow() } as isize;
        if front != 0 && front != ours {
            PREVIOUS_FOREGROUND.store(front, std::sync::atomic::Ordering::Relaxed);
        }
    }
    window.set_focus()
}

/// Forward the toplevel window's own pointer crossings to the webview as `edge-pointer` events
/// ({kind: "enter" | "leave", x, y} in window coordinates, so the page can tell a tab from the bar). WebKitGTK under XWayland drops many of its DOM mouseleave events, but GTK
/// still receives the X11 crossing. Crossings into child widgets (the webview itself) are ignored.
#[cfg(target_os = "linux")]
pub fn forward_pointer_crossings(window: &WebviewWindow) -> tauri::Result<()> {
    use gtk::gdk::{CrossingMode, NotifyType};
    use gtk::prelude::*;
    use tauri::Emitter;

    use std::{cell::Cell, rc::Rc};

    let gtk_window = window.gtk_window()?;
    gtk_window.add_events(
        gtk::gdk::EventMask::ENTER_NOTIFY_MASK | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    // While the pointer is inside, poll its position: WebKitGTK under XWayland delivers few DOM
    // mousemove events, so moving from one tab to the next inside the strip went unnoticed.
    // Inside our own X11 window the position is accurate (it only goes stale over Wayland windows).
    let inside = Rc::new(Cell::new(false));
    for (kind, leave) in [("enter", false), ("leave", true)] {
        let w = window.clone();
        let inside = inside.clone();
        let handler = move |win: &gtk::ApplicationWindow, e: &gtk::gdk::EventCrossing| {
            // Grab/ungrab crossings come from focus and pointer grabs (GNOME focusing the panel,
            // a click), not from the pointer moving; treating them as a leave made the panel
            // open, close and open again.
            if e.detail() != NotifyType::Inferior && e.mode() == CrossingMode::Normal {
                let (x, y) = e.position();
                let _ = w.emit(
                    "edge-pointer",
                    serde_json::json!({ "kind": kind, "x": x, "y": y }),
                );
                if leave {
                    inside.set(false);
                } else if !inside.replace(true) {
                    start_polling(win, w.clone(), inside.clone());
                }
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

#[cfg(target_os = "linux")]
fn start_polling(
    win: &gtk::ApplicationWindow,
    w: WebviewWindow,
    inside: std::rc::Rc<std::cell::Cell<bool>>,
) {
    use gtk::prelude::*;
    use tauri::Emitter;
    let Some(gdk_window) = win.window() else {
        return;
    };
    let Some(pointer) = gdk_window
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())
    else {
        return;
    };
    let mut last = (f64::NAN, f64::NAN);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
        if !inside.get() {
            return gtk::glib::ControlFlow::Break;
        }
        let (_, x, y, _) = gdk_window.device_position_double(&pointer);
        if (x, y) != last {
            last = (x, y);
            let _ = w.emit(
                "edge-pointer",
                serde_json::json!({ "kind": "move", "x": x, "y": y }),
            );
        }
        gtk::glib::ControlFlow::Continue
    });
}

#[cfg(not(target_os = "linux"))]
pub fn forward_pointer_crossings(_window: &WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

/// Give keyboard focus back after the panel collapses. On GNOME the panel is an XWayland window;
/// once it had focus, typing kept going into the hidden search box. Setting the X focus to
/// PointerRoot hands it back to the window manager, which refocuses the previous window.
/// Returns false where this is not needed or not possible (non-X11).
#[cfg(target_os = "linux")]
pub fn release_focus(window: &WebviewWindow) -> tauri::Result<bool> {
    use gtk::prelude::*;
    let gtk_window = window.gtk_window()?;
    let Some(gdk_window) = gtk_window.window() else {
        return Ok(false);
    };
    let Ok(display) = gdk_window.display().downcast::<gdkx11::X11Display>() else {
        return Ok(false);
    };
    // SAFETY: the Display pointer belongs to GDK and stays valid for the life of the app;
    // XSetInputFocus and XFlush only send requests on it, on the GTK main thread.
    unsafe {
        use gtk::glib::translate::ToGlibPtr;
        let xd = gdk_x11_sys::gdk_x11_display_get_xdisplay(display.to_glib_none().0)
            as *mut x11::xlib::Display;
        x11::xlib::XSetInputFocus(
            xd,
            x11::xlib::PointerRoot as u64,
            x11::xlib::RevertToPointerRoot,
            x11::xlib::CurrentTime,
        );
        x11::xlib::XFlush(xd);
    }
    Ok(true)
}

/// Windows: hand focus back to the window the user was in before the panel took it, so typing
/// after the panel closes goes there instead of into the hidden search box.
#[cfg(windows)]
pub fn release_focus(_window: &WebviewWindow) -> tauri::Result<bool> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    let prev = PREVIOUS_FOREGROUND.swap(0, std::sync::atomic::Ordering::Relaxed)
        as windows_sys::Win32::Foundation::HWND;
    // SAFETY: IsWindow checks the handle is still a live window before it is used; we are the
    // foreground process, which Windows requires for SetForegroundWindow to succeed.
    Ok(!prev.is_null() && unsafe { IsWindow(prev) != 0 && SetForegroundWindow(prev) != 0 })
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn release_focus(_window: &WebviewWindow) -> tauri::Result<bool> {
    Ok(false)
}

/// Which windowing backend we ended up on. GNOME on Wayland ignores window positioning
/// for native Wayland clients, so `main` switches GNOME to XWayland (see PLATFORMS.md).
pub fn backend() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if std::env::var("GDK_BACKEND").as_deref() == Ok("x11")
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        "x11"
    } else {
        "wayland"
    }
}

/// Run before GTK starts. Override with EDGE_NATIVE_WAYLAND=1 to test native Wayland.
pub fn prefer_xwayland_on_gnome() {
    let gnome =
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"));
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let xwayland = std::env::var_os("DISPLAY").is_some();
    let overridden = std::env::var_os("EDGE_NATIVE_WAYLAND").is_some()
        || std::env::var_os("GDK_BACKEND").is_some();
    if gnome && wayland && xwayland && !overridden {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920x1080 screen, GNOME top bar 32px tall.
    const WORK: Rect = Rect {
        x: 0,
        y: 32,
        w: 1920,
        h: 1048,
    };

    fn edge_rect(work: Rect, scale: f64, expanded: bool) -> Rect {
        super::edge_rect(work, scale, expanded, &Settings::default(), 0)
    }

    #[test]
    fn tabs_grow_the_strip_and_stay_centred() {
        let r = super::edge_rect(WORK, 1.0, false, &Settings::default(), 3);
        assert_eq!(r.h, 120 + 3 * 42);
        assert_eq!(r.y, 32 + (1048 - r.h as i32) / 2);
        let capped = super::edge_rect(WORK, 1.0, false, &Settings::default(), 50);
        assert_eq!(capped.h, 120 + super::MAX_TABS as u32 * 42);
        let tiny = Rect {
            x: 0,
            y: 0,
            w: 800,
            h: 200,
        };
        assert_eq!(
            super::edge_rect(tiny, 1.0, false, &Settings::default(), 7).h,
            200
        );
    }

    #[test]
    fn left_side_and_bar_position() {
        let s = Settings {
            side: Side::Left,
            bar_position: 0.0,
            ..Default::default()
        };
        assert_eq!(
            super::edge_rect(WORK, 1.0, false, &s, 0),
            Rect {
                x: 0,
                y: 32,
                w: 8,
                h: 120
            }
        );
        assert_eq!(super::edge_rect(WORK, 1.0, true, &s, 0).x, 0);
        let bottom = Settings {
            bar_position: 1.0,
            ..Default::default()
        };
        let r = super::edge_rect(WORK, 1.0, false, &bottom, 0);
        assert_eq!(r.y + r.h as i32, 32 + 1048);
    }

    #[test]
    fn dragged_panel_width_is_clamped() {
        let w = |pw: f64| {
            super::edge_rect(
                WORK,
                1.0,
                true,
                &Settings {
                    panel_width: Some(pw),
                    ..Default::default()
                },
                0,
            )
            .w
        };
        assert_eq!(w(500.0), 500);
        assert_eq!(w(50.0), 320);
        assert_eq!(w(5000.0), 1536);
        let hidpi = Settings {
            panel_width: Some(500.0),
            ..Default::default()
        };
        assert_eq!(super::edge_rect(WORK, 2.0, true, &hidpi, 0).w, 1000);
    }

    #[test]
    fn collapsed_bar_hugs_right_edge_vertically_centred() {
        let r = edge_rect(WORK, 1.0, false);
        assert_eq!(
            r,
            Rect {
                x: 1912,
                y: 32 + (1048 - 120) / 2,
                w: 8,
                h: 120
            }
        );
    }

    #[test]
    fn expanded_panel_is_a_third_and_full_height() {
        let r = edge_rect(WORK, 1.0, true);
        assert_eq!(
            r,
            Rect {
                x: 1280,
                y: 32,
                w: 640,
                h: 1048
            }
        );
    }

    #[test]
    fn panel_has_a_minimum_width_scaled_for_hidpi() {
        let small = Rect {
            x: 0,
            y: 0,
            w: 1200,
            h: 800,
        };
        assert_eq!(edge_rect(small, 2.0, true).w, 640);
        assert_eq!(edge_rect(small, 2.0, false).w, 16);
    }

    #[test]
    fn second_monitor_offset_is_respected() {
        let right_monitor = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let r = edge_rect(right_monitor, 1.0, false);
        assert_eq!(r.x, 1920 + 2560 - 8);
    }

    #[test]
    fn never_wider_than_a_tiny_screen() {
        let tiny = Rect {
            x: 0,
            y: 0,
            w: 300,
            h: 100,
        };
        let r = edge_rect(tiny, 1.0, true);
        assert_eq!((r.x, r.w), (0, 300));
        assert_eq!(edge_rect(tiny, 1.0, false).h, 100);
    }
}
