# Edge (codename)

Capture a note on your phone in two seconds. It is already on the edge of your laptop screen.

| Path | What |
|---|---|
| `core/` | `notes-core`: note model, SQLite + FTS5 store, sync queue. Shared by every app. |
| `desktop/` | Tauri edge bar (Phase 1, not started) |
| `mobile/` | Flutter Android app (Phase 2, not started) |

The hosted sync server is a separate, private repository.

```bash
cargo test --workspace
cargo run --example notes -- add "buy milk"
cargo run --example notes -- search milk
```

## Licence

GPL-3.0-only, see `LICENSE`. Contributions need the CLA (bot set up on GitHub). The product name is a trademark and is not covered by the licence.
