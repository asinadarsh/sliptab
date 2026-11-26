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

## Code signing policy

Windows releases will be signed: free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org) (application in progress; until then the
Windows installer on sliptab.com is unsigned, and the Microsoft Store build is signed by the Store).

- Committers and reviewers: [@asinadarsh](https://github.com/asinadarsh)
- Approvers: [@asinadarsh](https://github.com/asinadarsh)

Only builds made by this repository's GitHub Actions from its own source are submitted for signing,
and each release is approved by hand.

Privacy: notes stay on your device unless you sign in to sync. The app asks sliptab.com for updates
every 6 hours (it sends only the app version and platform). Link previews are fetched from the
linked site itself. Full policy: [sliptab.com/privacy](https://sliptab.com/privacy).

## Licence

GPL-3.0-only, see `LICENSE`. The Sliptab name and logo are not covered by the licence.
