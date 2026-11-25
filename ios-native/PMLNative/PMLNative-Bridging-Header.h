#ifndef PMLNative_Bridging_Header_h
#define PMLNative_Bridging_Header_h

#include <stddef.h>
#include <stdint.h>

long pml_inflate_raw(const uint8_t *src, size_t src_len, uint8_t *dst, size_t dst_len);

#endif
