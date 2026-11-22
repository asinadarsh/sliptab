//! Shared core for Edge: the note model and the local SQLite store.
//! Used by the desktop app, the mobile app (via flutter_rust_bridge) and the server.

pub mod attachment;
#[cfg(feature = "client")]
pub mod client;
pub mod folders;
pub mod format;
mod link;
mod note;
pub mod preview;
#[cfg(feature = "store")]
mod store;
pub mod sync;

pub use attachment::Attachment;
pub use link::{clean_url, platform_of, Link};
pub use note::{derive_title, valid_color, Note, MAX_EXTRA_BYTES};
#[cfg(feature = "store")]
pub use store::Store;
pub use uuid::Uuid;
