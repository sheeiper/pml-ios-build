pub mod four_domains;
pub mod library_modules;
pub mod comic_library;
pub mod comic_scraper;
pub mod online_comics;
pub mod magnet_search;
pub mod offline_download;
pub mod media_library;
pub mod telegram_public;
pub mod v2_catalog;
pub mod v2_state;
pub mod jav_scraper;
pub mod movie_extras;
pub mod performers;
pub mod album_bundler;
pub mod cleaner;
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
