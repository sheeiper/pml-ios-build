use std::future::Future;

/// 与 Tauri 的 `async_runtime::spawn` 对齐（泛型输出 + JoinHandle）。
#[cfg(feature = "tauri")]
pub fn spawn<F>(future: F) -> tauri::async_runtime::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tauri::async_runtime::spawn(future)
}

#[cfg(not(feature = "tauri"))]
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(future)
}

#[cfg(not(feature = "tauri"))]
pub fn runtime() -> &'static tokio::runtime::Runtime {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("pml-core")
            .build()
            .expect("无法创建 Rust 内核 tokio 运行时")
    })
}
