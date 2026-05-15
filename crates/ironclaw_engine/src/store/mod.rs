//! Filesystem-backed implementation of the engine [`crate::Store`] trait.
//!
//! See [`filesystem::FilesystemStore`] for the implementation.

pub mod filesystem;
mod paths;

pub use filesystem::FilesystemStore;
