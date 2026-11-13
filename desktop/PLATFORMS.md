# Edge bar: platform behaviour

What actually happens on each desktop, measured rather than assumed. Update this when testing a new one.

## Ubuntu 26.04, GNOME Shell 50.1, Wayland (Asin's laptop), tested 2026-11-14

GNOME 50 has no X11 session, but XWayland is running (`DISPLAY=:0`).

| Check | Native Wayland (`EDGE_NATIVE_WAYLAND=1`) | XWayland (default on GNOME) |
|---|---|---|
| Window placed on the right edge | No. Position reads back 0,0; GNOME ignores it | **Yes**, exact (x=3056 on a 3072 px wide screen) |
| Size honoured | Yes | Yes, **only if the window is resizable**. With `resizable: false` GTK pins min=max=400x400 |
| Always on top | Not measurable | **Yes** (`_NET_WM_STATE_ABOVE`) |
| On all workspaces | Not measurable | **Yes** (`_NET_WM_STATE_STICKY`) |
| Hidden from taskbar/alt-tab | | **Yes** (`SKIP_TASKBAR`, `SKIP_PAGER`) |
| Real mouse hover / click opens the panel | | **Click: yes** (confirmed by hand). Hover: to confirm by hand |
| Global hotkey | Needs the GlobalShortcuts portal (v1 available) | XWayland key grabs only work while an X11 window has focus, so the portal is needed here too |

The GNOME rule is in `src-tauri/src/edge.rs::prefer_xwayland_on_gnome`: it sets `GDK_BACKEND=x11` unless
`EDGE_NATIVE_WAYLAND` or `GDK_BACKEND` is already set.

### Testing gotcha

`xdotool mousemove` does **not** produce enter/leave events for windows on GNOME Wayland (even an `xev`
window got none). Pointer behaviour must be checked by hand. Window geometry can still be read with
`xwininfo -id $(xdotool search --name '^Edge$')`, and `scripts/dev-probe.sh` prints both.

## Not yet tested

- KDE Plasma / Hyprland / Sway (plan: `gtk-layer-shell`)
- Windows 11
- macOS (Asin's Mac)
