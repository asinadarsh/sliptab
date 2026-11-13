//! Placing the window on the screen edge: a thin bar when collapsed, a tall panel when expanded.
//! All platform differences in edge behaviour live in this file.

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

/// Collapsed bar, in logical pixels.
const BAR_WIDTH: f64 = 8.0;
const BAR_HEIGHT: f64 = 120.0;
/// The expanded panel is a third of the work area, but never narrower than this.
const PANEL_MIN_WIDTH: f64 = 320.0;

/// A rectangle in physical pixels: x, y, width, height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Where the window goes, given the monitor's work area (which excludes GNOME's top bar,
/// the Windows taskbar and the macOS menu bar). Right edge only for now; edge side is a setting in Task 6.
pub fn edge_rect(work: Rect, scale: f64, expanded: bool) -> Rect {
    let (w, h) = if expanded {
        let w = ((PANEL_MIN_WIDTH * scale) as u32).max(work.w / 3).min(work.w);
        (w, work.h)
    } else {
        ((BAR_WIDTH * scale).round() as u32, ((BAR_HEIGHT * scale).round() as u32).min(work.h))
    };
    let x = work.x + (work.w - w) as i32;
    let y = if expanded { work.y } else { work.y + ((work.h - h) / 2) as i32 };
    Rect { x, y, w, h }
}

pub fn place(window: &WebviewWindow, expanded: bool) -> tauri::Result<Option<Rect>> {
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        return Ok(None);
    };
    let wa = monitor.work_area();
    let work = Rect { x: wa.position.x, y: wa.position.y, w: wa.size.width, h: wa.size.height };
    let r = edge_rect(work, monitor.scale_factor(), expanded);
    window.set_size(PhysicalSize::new(r.w, r.h))?;
    window.set_position(PhysicalPosition::new(r.x, r.y))?;
    Ok(Some(r))
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
