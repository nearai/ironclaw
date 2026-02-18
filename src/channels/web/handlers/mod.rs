//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.

pub mod chat;
pub mod extensions;
pub mod jobs;
pub mod memory;
pub mod routines;
pub mod settings;
pub mod skills;
pub mod static_files;

// Re-export all handler functions so `server.rs` can reference them
// as `handlers::chat_send_handler`, etc.
pub use chat::*;
pub use extensions::*;
pub use jobs::*;
pub use memory::*;
pub use routines::*;
pub use settings::*;
pub use skills::*;
pub use static_files::*;
