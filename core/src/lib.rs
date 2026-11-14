//! Shared core for Edge: the note model and the local SQLite store.
//! Used by the desktop app, the mobile app (via flutter_rust_bridge) and the server.

mod note;
mod store;

pub use note::{derive_title, valid_color, Note};
pub use store::Store;
pub use uuid::Uuid;
