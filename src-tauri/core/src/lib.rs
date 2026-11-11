pub mod four_domains;
pub mod media_library;
pub mod telegram_public;
pub mod v2_catalog;
pub mod v2_state;
mod v2_config;
mod v2_import;
mod v2_moves;
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
