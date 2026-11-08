#ifndef PML_RAW_INFLATE_H
#define PML_RAW_INFLATE_H

#include <stddef.h>
#include <stdint.h>

/// 解压 ZIP 的 raw DEFLATE 数据（无 zlib 头）。返回解压字节数，失败返回负数。
long pml_inflate_raw(const uint8_t *src, size_t src_len, uint8_t *dst, size_t dst_len);

#endif
