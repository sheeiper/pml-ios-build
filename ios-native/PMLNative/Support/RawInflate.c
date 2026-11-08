#include "RawInflate.h"
#include <string.h>
#include <zlib.h>

long pml_inflate_raw(const uint8_t *src, size_t src_len, uint8_t *dst, size_t dst_len) {
    z_stream stream;
    memset(&stream, 0, sizeof(stream));
    if (inflateInit2(&stream, -15) != Z_OK) {
        return -1;
    }
    stream.next_in = (Bytef *)src;
    stream.avail_in = (uInt)src_len;
    stream.next_out = dst;
    stream.avail_out = (uInt)dst_len;
    int result = inflate(&stream, Z_FINISH);
    long written = (long)stream.total_out;
    inflateEnd(&stream);
    if (result != Z_STREAM_END) {
        return -2;
    }
    return written;
}
