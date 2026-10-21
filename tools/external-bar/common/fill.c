#include "fill.h"
#include <string.h>

void fill_pixels(uint8_t *bytes, size_t len, uint64_t generation) {
    uint8_t shade = (uint8_t)(generation % 128);
    size_t pixels = len / 4;
    uint32_t px;
    uint8_t p[4] = {shade, shade, shade, 255};
    memcpy(&px, p, 4);
    uint32_t *out = (uint32_t *)bytes;
    for (size_t i = 0; i < pixels; i++) {
        out[i] = px;
    }
    memset(bytes + pixels * 4, shade, len - pixels * 4);
}
