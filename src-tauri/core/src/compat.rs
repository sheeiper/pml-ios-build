//! Tauri 兼容层：在没有 Tauri 的场景（iOS 纯 Rust 静态库）提供同样形状的
//! `AppHandle` / `State`，使 `media_library.rs` 与 `server.rs` 不必改动签名。

use std::any::Any;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use crate::media_library::MediaLibraryState;

struct Inner {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    state: Box<dyn Any + Send + Sync>,
}

#[derive(Clone)]
pub struct AppHandle {
    inner: Arc<Inner>,
}

pub struct PathResolver<'a> {
    inner: &'a Inner,
}

impl<'a> PathResolver<'a> {
    pub fn app_data_dir(&self) -> Result<PathBuf, String> {
        Ok(self.inner.data_dir.clone())
    }

    pub fn app_cache_dir(&self) -> Result<PathBuf, String> {
        Ok(self.inner.cache_dir.clone())
    }
}

pub struct State<'a, T> {
    value: &'a T,
}

impl<'a, T> Clone for State<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T> Copy for State<'a, T> {}

impl<'a, T> Deref for State<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T> State<'a, T> {
    pub fn inner(&self) -> &'a T {
        self.value
    }
}

impl AppHandle {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf, state: MediaLibraryState) -> Self {
        Self {
            inner: Arc::new(Inner {
                data_dir,
                cache_dir,
                state: Box::new(state),
            }),
        }
    }

    pub fn path(&self) -> PathResolver<'_> {
        PathResolver { inner: &self.inner }
    }

    pub fn state<T>(&self) -> State<'_, T>
    where
        T: Send + Sync + 'static,
    {
        State {
            value: self
                .inner
                .state
                .downcast_ref::<T>()
                .expect("请求的状态未被托管"),
        }
    }

    pub fn try_state<T>(&self) -> Option<State<'_, T>>
    where
        T: Send + Sync + 'static,
    {
        self.inner
            .state
            .downcast_ref::<T>()
            .map(|value| State { value })
    }
}
