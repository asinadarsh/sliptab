# Sliptab

Notes on the edge of your screen. Pinned notes sit as coloured tabs on the edge of your laptop
screen: hover a tab to read it, over any app. Sync to your other computers and to the Android app
is optional.

Website: [sliptab.com](https://sliptab.com) · Windows and Linux · Android coming soon · Mac later

| Path | What |
|---|---|
| `core/` | `notes-core`: the note model, SQLite + FTS5 store and sync queue |
| `desktop/` | The Tauri app: the edge bar, the panel and the app window |

The sync server and the Android app are separate, closed-source projects.

## Build

Needs Rust (stable), Node 20+ and the Tauri 2 system packages for your OS
([prerequisites](https://v2.tauri.app/start/prerequisites/)).

```bash
cargo test --workspace
cd desktop && npm ci && npm run tauri build
```

Google sign-in is only built in when `scripts/google-env.sh` finds an OAuth client file; without it,
email sign-in still works.

## Licence

GPL-3.0-only, see `LICENSE`. The Sliptab name and logo are not covered by the licence.
