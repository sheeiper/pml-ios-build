#ifndef PML_CORE_H
#define PML_CORE_H

#include <stdint.h>
#include <stddef.h>

long pml_inflate_raw(const uint8_t *src, size_t src_len, uint8_t *dst, size_t dst_len);

#ifdef __cplusplus
extern "C" {
#endif

/// 启动纯 Rust 内核（无 Tauri、无窗口），返回本地 API 端口；<0 表示失败。
/// @param data_dir  App 的 Application Support 目录（SQLite 所在）
/// @param cache_dir App 的 Caches 目录（封面/漫画缓存）
int32_t pml_core_start(const char *data_dir, const char *cache_dir);

/// 最近一次启动失败的原因（UTF-8），没有错误时返回 NULL。
const char *pml_core_last_error(void);
int32_t pml_core_restart(void);

#ifdef __cplusplus
}
#endif

#endif /* PML_CORE_H */
