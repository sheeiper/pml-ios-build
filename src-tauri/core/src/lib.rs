pub mod media_library;
pub mod rt;
pub mod server;

#[cfg(not(feature = "tauri"))]
pub mod compat;

#[cfg(not(feature = "tauri"))]
pub use compat::{AppHandle, State};

#[cfg(feature = "tauri")]
pub use tauri::{AppHandle, State};

#[cfg(all(feature = "ios-ffi", not(feature = "tauri")))]
pub mod ios_ffi;
