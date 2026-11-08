//! iOS 原生壳的 C ABI：在 App 内启动纯 Rust 内核（不依赖 Tauri）。

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::compat::AppHandle;
use crate::media_library::MediaLibraryState;

static CORE_PORT: Mutex<Option<u16>> = Mutex::new(None);
static CORE_APP: OnceLock<AppHandle> = OnceLock::new();
static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);

fn read_c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned())
}

fn remember_error(message: &str) {
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = CString::new(message.replace('\0', " ")).ok();
    }
}

#[no_mangle]
pub extern "C" fn pml_core_last_error() -> *const c_char {
    if let Ok(slot) = LAST_ERROR.lock() {
        if let Some(value) = slot.as_ref() {
            return value.as_ptr();
        }
    }
    std::ptr::null()
}

/// 启动内核并返回本地 API 端口；< 0 表示失败。
#[no_mangle]
pub extern "C" fn pml_core_start(data_dir: *const c_char, cache_dir: *const c_char) -> i32 {
    if let Some(port) = CORE_PORT.lock().ok().and_then(|slot| *slot) {
        return i32::from(port);
    }
    let Some(data_dir) = read_c_string(data_dir) else {
        remember_error("数据目录为空");
        return -2;
    };
    let cache_dir = read_c_string(cache_dir).unwrap_or_else(|| data_dir.clone());
    let data_path = PathBuf::from(data_dir);
    let cache_path = PathBuf::from(cache_dir);
    let _ = std::fs::create_dir_all(&data_path);
    let _ = std::fs::create_dir_all(&cache_path);

    // 提前创建 tokio 运行时，后续所有 spawn 都复用它。
    let _ = crate::rt::runtime();

    let app = AppHandle::new(data_path, cache_path, MediaLibraryState::new());
    let _ = CORE_APP.set(app.clone());
    crate::media_library::init_app_handle(app.clone());
    match crate::server::start_on_loopback(app.clone(), 0) {
        Ok(port) => {
            // 就绪探测：确认 axum 真的在监听，否则把服务端错误暴露给 Swift。
            let address = format!("127.0.0.1:{port}");
            let mut ready = false;
            if let Ok(socket) = address.parse::<std::net::SocketAddr>() {
                for _ in 0..50 {
                    if std::net::TcpStream::connect_timeout(&socket, std::time::Duration::from_millis(100)).is_ok() {
                        ready = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
            }
            if !ready {
                let reason = crate::server::last_server_error()
                    .unwrap_or_else(|| "本地 API 未监听".to_owned());
                remember_error(&format!("{reason}（端口 {port}）"));
                return -5;
            }
            if let Ok(mut slot) = CORE_PORT.lock() { *slot = Some(port); }
            crate::media_library::start_scheduled_scanner(app.clone());
            i32::from(port)
        }
        Err(message) => {
            remember_error(&message);
            -4
        }
    }
}

/// 后台恢复时如果旧监听已失效，重新绑定一个本地端口并返回新端口（0/-1/-4）。
#[no_mangle]
pub extern "C" fn pml_core_restart() -> i32 {
    let Some(app) = CORE_APP.get().cloned() else {
        remember_error("内核尚未初始化");
        return -1;
    };
    match crate::server::start_on_loopback(app, 0) {
        Ok(port) => {
            if let Ok(mut slot) = CORE_PORT.lock() {
                *slot = Some(port);
            }
            i32::from(port)
        }
        Err(message) => {
            remember_error(&message);
            -4
        }
    }
}
