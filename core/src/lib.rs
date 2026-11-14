//! Shared core for Edge: the note model and the local SQLite store.
//! Used by the desktop app, the mobile app (via flutter_rust_bridge) and the server.

mod link;
mod note;
#[cfg(feature = "store")]
mod store;
pub mod sync;
#[cfg(feature = "client")]
pub mod client;

pub use link::{clean_url, platform_of, Link};
pub use note::{derive_title, valid_color, Note};
#[cfg(feature = "store")]
pub use store::Store;
pub use uuid::Uuid;
