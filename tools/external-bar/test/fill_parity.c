/* Compare fill_pixels against a byte-loop reimplementation of the peer's
 * canvas.rs fill (pixels [s,s,s,255], remainder bytes s), odd lengths too. */
#include "../common/fill.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    size_t lens[] = {184320, 4194304, 1, 3, 4, 5, 7, 4099};
    for (size_t li = 0; li < sizeof lens / sizeof *lens; li++) {
        size_t n = lens[li];
        uint8_t *a = malloc(n + 8), *b = malloc(n + 8);
        for (uint64_t g = 0; g < 300; g += 37) {
            memset(a, 0xEE, n + 8); memset(b, 0xEE, n + 8);
            fill_pixels(a, n, g);
            uint8_t s = g % 128;
            size_t px = n / 4;
            for (size_t i = 0; i < px; i++) { b[4*i]=s; b[4*i+1]=s; b[4*i+2]=s; b[4*i+3]=255; }
            for (size_t i = px * 4; i < n; i++) b[i] = s;
            if (memcmp(a, b, n + 8)) { printf("MISMATCH len=%zu g=%llu\n", n, (unsigned long long)g); return 1; }
        }
        free(a); free(b);
    }
    puts("fill parity OK (8 lengths x 9 generations, guard bytes untouched)");
    size_t n = 1920u * 24u * 4u;
    uint8_t *pixels = malloc(n + 8);
    for (uint64_t g = 0; g <= 256; g++) {
        memset(pixels, 0xEE, n + 8);
        fill_bursty_panel(pixels, g);
        for (unsigned y = 0; y < 24; y++) {
            for (unsigned x = 0; x < 1920; x++) {
                uint8_t shade = x < 1824 ? 1 : g % 128;
                size_t off = (y * 1920 + x) * 4;
                if (pixels[off] != shade || pixels[off+1] != shade ||
                    pixels[off+2] != shade || pixels[off+3] != 255) return 1;
            }
        }
        for (size_t i = n; i < n + 8; i++) if (pixels[i] != 0xEE) return 1;
    }
    free(pixels);
    puts("bursty pixel bounds OK (constant background, changing rectangle, guards)");
    return 0;
}
