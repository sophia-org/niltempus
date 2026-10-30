#ifndef GPU_X_PIXELS_H
#define GPU_X_PIXELS_H
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <stdbool.h>
#include <stdio.h>
/* XGetPixel handles byte order and scanline padding; masks fix channel meaning. */
static bool x_pixels(XImage *image, FILE *out) {
    if (!image || !image->data || image->width != 1920 || image->height != 1080 ||
        image->format != ZPixmap || image->depth != 24 || image->bits_per_pixel != 32 ||
        image->xoffset || image->bytes_per_line < 1920*4 ||
        (image->byte_order != LSBFirst && image->byte_order != MSBFirst) ||
        image->red_mask != 0xff0000 || image->green_mask != 0xff00 || image->blue_mask != 0xff)
        return false;
    if (fputs("P7\nWIDTH 1920\nHEIGHT 1080\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n", out) < 0) return false;
    for (int y = 0; y < 1080; y++) {
        unsigned char row[1920*4];
        for (int x = 0; x < 1920; x++) {
            unsigned long p = XGetPixel(image, x, y);
            row[4*x] = (unsigned char)(p >> 16);
            row[4*x+1] = (unsigned char)(p >> 8);
            row[4*x+2] = (unsigned char)p;
            row[4*x+3] = 255;
        }
        if (fwrite(row, 1, sizeof row, out) != sizeof row) return false;
    }
    return fflush(out) == 0;
}
#endif
