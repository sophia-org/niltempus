/* Included only by x-panel-gpu. Plain PresentPixmap, never syncobj Present. */
#include <poll.h>
#include <sys/stat.h>
#include <xcb/dri3.h>
#include "producer.h"

static void gpu_x_open(struct ctx *c, FILE *journal) {
    Visual *visual = DefaultVisual(c->dpy, DefaultScreen(c->dpy));
    if (c->depth != 24 || visual->red_mask != 0xff0000 ||
        visual->green_mask != 0xff00 || visual->blue_mask != 0xff)
        die("DRI3 requires the XR24 visual");
    const xcb_query_extension_reply_t *ext = xcb_get_extension_data(c->conn, &xcb_dri3_id);
    if (!ext || !ext->present) die("DRI3 unavailable");
    xcb_dri3_query_version_reply_t *version = xcb_dri3_query_version_reply(
        c->conn, xcb_dri3_query_version(c->conn, 1, 2), NULL);
    if (!version || version->major_version != 1 || version->minor_version < 2)
        die("DRI3 1.2 required");
    free(version);
    xcb_dri3_open_reply_t *opened = xcb_dri3_open_reply(
        c->conn, xcb_dri3_open(c->conn, c->win, 0), NULL);
    if (!opened || opened->nfd != 1) die("DRI3 did not identify one device");
    int fd = xcb_dri3_open_reply_fds(c->conn, opened)[0];
    struct stat st;
    if (fstat(fd, &st) || !S_ISCHR(st.st_mode) || !gpu_device_matches_at("/sys", st.st_rdev))
        die("X server uses a different physical GPU");
    close(fd);
    free(opened);
    xcb_dri3_get_supported_modifiers_reply_t *mods = xcb_dri3_get_supported_modifiers_reply(
        c->conn, xcb_dri3_get_supported_modifiers(c->conn, c->win, 24, 32), NULL);
    if (!mods) die("DRI3 modifier query failed");
    bool linear = false;
    uint64_t *values = xcb_dri3_get_supported_modifiers_window_modifiers(mods);
    for (int i = 0; i < xcb_dri3_get_supported_modifiers_window_modifiers_length(mods); i++)
        linear |= values[i] == 0;
    values = xcb_dri3_get_supported_modifiers_screen_modifiers(mods);
    for (int i = 0; i < xcb_dri3_get_supported_modifiers_screen_modifiers_length(mods); i++)
        linear |= values[i] == 0;
    free(mods);
    if (!linear) die("DRI3 does not advertise LINEAR for depth24/bpp32");
    gpu_open(journal, "dri3_1_2");
    fprintf(journal, "# gpu_import schema=1 protocol=dri3 version=1.2 server_pci=" GPU_PCI
            " format=XR24 modifier=0 present=plain release=IdleNotify sync=implicit\n");
}

static void gpu_x_paint(struct ctx *c, struct back *b, uint64_t generation) {
    if (b->busy || b->token || b->pixmap) die("DRI3 slot still held by consumer");
    GpuBarFrame frame = gpu_render(generation);
    b->pixmap = xcb_generate_id(c->conn);
    b->token = frame.token;
    /* xcb owns/closes the passed FD. Asynchronous errors are fatal in handle(). */
    xcb_dri3_pixmap_from_buffers(c->conn, b->pixmap, c->win, 1, frame.width, frame.height,
        frame.stride, (uint32_t)frame.offset, 0, 0, 0, 0, 0, 0, 24, 32, frame.modifier, &frame.fd);
}

static void gpu_x_idle(struct ctx *c, xcb_present_idle_notify_event_t *event) {
    if (event->window != c->win || event->event != c->present_eid || event->idle_fence)
        die("misattributed or fenced GPU IdleNotify");
    for (unsigned i = 0; i < c->nbacks; i++) {
        struct back *b = &c->backs[i];
        if (b->pixmap != event->pixmap || b->serial != event->serial) continue;
        if (!b->busy || !b->token) die("duplicate GPU IdleNotify");
        xcb_free_pixmap(c->conn, b->pixmap);
        gpu_release(b->token);
        b->pixmap = b->token = 0;
        b->busy = 0;
        return;
    }
    die("stale or foreign GPU IdleNotify");
}
