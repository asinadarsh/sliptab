// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `sliptab mcp`: the server AI agents start (Claude Code, Codex, Cursor, Kiro...). It talks
    // over stdin/stdout and never opens a window, so it must not reach the single-instance check.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return desktop_lib::mcp::serve();
    }
    #[cfg(target_os = "linux")]
    desktop_lib::prefer_xwayland_on_gnome();
    desktop_lib::run()
}
