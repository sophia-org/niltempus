/* Shared whole-pixel fill for every external-bar reference client.
 *
 * Byte-for-byte the same work as the Sophia timing peer's `fill_pixels`
 * (comparator crates/sophia-runtime/examples/shell_timing_peer/canvas.rs):
 * every 4-byte pixel becomes [shade, shade, shade, 255] with
 * shade = generation % 128; a partial trailing pixel gets `shade` bytes.
 * The shade is equal on all colour channels, so BGRX, BGRA and RGBA
 * framebuffers receive identical bytes and the fill cost does not depend
 * on the platform's pixel order.
 */
#ifndef EXTERNAL_BAR_FILL_H
#define EXTERNAL_BAR_FILL_H
#include <stddef.h>
#include <stdint.h>

void fill_pixels(uint8_t *bytes, size_t len, uint64_t generation);

#endif
