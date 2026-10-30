/* Untimed niri screenshot normalization. No gamma, profile or channel transform. */
#define _GNU_SOURCE
#include <png.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>
#define WIDTH 1920
#define HEIGHT 1080
static unsigned char pixels[WIDTH * HEIGHT * 4];
static png_bytep rows[HEIGHT];
static void refuse(const char *message) { fprintf(stderr, "png capture refused: %s\n", message); exit(2); }
static int unknown_chunk(png_structp png, png_unknown_chunkp chunk) {
    (void)png; (void)chunk;
    return -1; /* Includes APNG: a capture must be one unambiguous still image. */
}
int main(int argc, char **argv) {
    if (argc != 2) refuse("one PNG path required");
    int fd = open(argv[1], O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK);
    struct stat st;
    if (fd < 0 || fstat(fd, &st) || !S_ISREG(st.st_mode) || st.st_size <= 0 || st.st_size > 20*1024*1024)
        refuse("bounded regular PNG required");
    FILE *file = fdopen(fd, "rb");
    if (!file) refuse("fdopen");
    png_structp png = png_create_read_struct(PNG_LIBPNG_VER_STRING, NULL, NULL, NULL);
    if (!png) refuse("decoder allocation");
    png_infop info = png_create_info_struct(png);
    if (!info || setjmp(png_jmpbuf(png))) refuse("invalid PNG");
    png_set_user_limits(png, WIDTH, HEIGHT);
    png_set_chunk_malloc_max(png, 1024*1024);
    png_set_chunk_cache_max(png, 32);
    png_set_crc_action(png, PNG_CRC_ERROR_QUIT, PNG_CRC_ERROR_QUIT);
    png_set_keep_unknown_chunks(png, PNG_HANDLE_CHUNK_ALWAYS, NULL, 0);
    png_set_read_user_chunk_fn(png, NULL, unknown_chunk);
    png_init_io(png, file);
    png_read_info(png, info);
    int type = png_get_color_type(png, info);
    if (png_get_image_width(png, info) != WIDTH || png_get_image_height(png, info) != HEIGHT ||
        png_get_bit_depth(png, info) != 8 ||
        (type != PNG_COLOR_TYPE_RGB && type != PNG_COLOR_TYPE_RGBA) ||
        png_get_interlace_type(png, info) != PNG_INTERLACE_NONE || png_get_valid(png, info, PNG_INFO_tRNS))
        refuse("requires 1920x1080 noninterlaced RGB8/RGBA8 without tRNS");
    if (type == PNG_COLOR_TYPE_RGB) png_set_add_alpha(png, 255, PNG_FILLER_AFTER);
    png_read_update_info(png, info);
    if (png_get_rowbytes(png, info) != WIDTH*4) refuse("unexpected row layout");
    for (unsigned y = 0; y < HEIGHT; y++) rows[y] = pixels + y*WIDTH*4;
    png_read_image(png, rows);
    png_read_end(png, info);
    if (fgetc(file) != EOF || ferror(file)) refuse("trailing bytes or read error");
    png_destroy_read_struct(&png, &info, NULL);
    if (fclose(file)) refuse("close");
    if (fputs("P7\nWIDTH 1920\nHEIGHT 1080\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n", stdout) < 0 ||
        fwrite(pixels, 1, sizeof pixels, stdout) != sizeof pixels || fflush(stdout)) refuse("PAM write");
    return 0;
}
