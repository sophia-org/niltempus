/* x-panel: MIT-SHM + Present reference panel client for the XLibre arm of
 * the external bar (WORKLOADS.md, journal rev 5). Same visible work as the
 * Sophia timing peer: a 1920x24 strip at (0,0), filled with the shared
 * whole-pixel fill, presented through X Present, one frame in flight.
 *
 * Workloads (--workload):
 *   fresh     new full panel every vsync; optional --bulk adds a 1024x1024
 *             upload into an offscreen pixmap each frame. Worst-case bound.
 *   bursty    absolute timer schedule, one update every --interval-ms; each
 *             update fills a whole new buffer but uploads and presents only
 *             the 96x24 damage rectangle at x=1824 (XFixes update region).
 *   retained  8 tiles of 240x24, uploaded once; every vsync they are copied
 *             (GPU CopyArea under glamor) into the back pixmap at rotated
 *             slots and presented. No new pixels after setup.
 *   idle      present once, wait for it, then sleep --idle-sec doing nothing.
 *
 * Lifecycles (--lifecycle):
 *   fresh  every upload is a new memfd: memfd_create, fallocate, seal
 *          GROW|SHRINK, map, fill, unmap, seal WRITE|SEAL, MIT-SHM 1.2
 *          AttachFd read-only, ShmPutImage into a server pixmap, detach at
 *          once. A write-sealed memfd can only be mapped PROT_READ, and
 *          XLibre refuses ShmCreatePixmap on a read-only segment
 *          (Xext/shm/shm.c VERIFY_SHMPTR needwrite -> BadAccess), hence the
 *          PutImage copy; under glamor that copy is the texture upload.
 *   reuse  fixed SysV segments with shared-memory pixmaps, filled in place,
 *          reused after IdleNotify: ordinary X practice; ungated matrix.
 *
 * No Sophia dependency; never launches a server. Requires Present 1.0,
 * MIT-SHM 1.2 (fresh) and XFixes 2.0 (bursty update region).
 */
#define _GNU_SOURCE
#include <X11/Xlib-xcb.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/XShm.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <linux/memfd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ipc.h>
#include <sys/mman.h>
#include <sys/shm.h>
#include <time.h>
#include <unistd.h>
#include <xcb/present.h>
#include <xcb/shm.h>
#include <xcb/xcb.h>
#include <xcb/xfixes.h>

#include "../common/fill.h"
#include "../common/journal.h"

#define TILES 8
#define DMG_W 96u
#define DMG_H 24u

enum workload { W_FRESH, W_BURSTY, W_RETAINED, W_IDLE };

struct sysv {
    XShmSegmentInfo info;
    XImage *image;
    size_t len;
};

struct back {
    Pixmap pixmap;
    struct sysv shm; /* reuse lifecycle, fresh/bursty/idle only */
    int busy;        /* presented and not yet idle */
    uint32_t serial;
#ifdef MATCHED_GPU
    uint64_t token;
#endif
};

struct ctx {
    Display *dpy;
    xcb_connection_t *conn;
    Window win;
    GC gc;
    int depth;
    int fresh_lifecycle;
    int bursty;
    uint8_t present_opcode;
    uint8_t shm_first_event;
    uint32_t pending_serial;
    int complete_seen;
    uint64_t complete_ust_ns, complete_recv_ns;
    int bulk_busy;
    Pixmap bulk_target;
    struct sysv bulk_shm;
    struct back *backs;
    unsigned nbacks;
    uint64_t clock_ok, clock_bad;
#ifdef MATCHED_GPU
    uint32_t present_eid;
    uint64_t deadline;
#endif
};

static void die(const char *what) {
    fprintf(stderr, "x-panel: %s (errno %d: %s)\n", what, errno, strerror(errno));
    exit(2);
}
#ifdef MATCHED_GPU
#include "../gpu/x11.h"
#endif

static void sysv_create(struct ctx *c, struct sysv *s, unsigned w, unsigned h) {
    s->image = XShmCreateImage(c->dpy, DefaultVisual(c->dpy, DefaultScreen(c->dpy)),
                               c->depth, ZPixmap, NULL, &s->info, w, h);
    if (!s->image) die("XShmCreateImage");
    if (s->image->bits_per_pixel != 32) die("expected a 32 bpp visual");
    s->len = (size_t)s->image->bytes_per_line * h;
    s->info.shmid = shmget(IPC_PRIVATE, s->len, IPC_CREAT | 0600);
    if (s->info.shmid < 0) die("shmget");
    s->info.shmaddr = shmat(s->info.shmid, NULL, 0);
    if (s->info.shmaddr == (void *)-1) die("shmat");
    s->image->data = s->info.shmaddr;
    s->info.readOnly = False;
    if (!XShmAttach(c->dpy, &s->info)) die("XShmAttach");
    XSync(c->dpy, False);
    shmctl(s->info.shmid, IPC_RMID, NULL);
}

static void sysv_destroy(struct ctx *c, struct sysv *s) {
    XShmDetach(c->dpy, &s->info);
    XSync(c->dpy, False);
    shmdt(s->info.shmaddr);
    s->image->data = NULL;
    XDestroyImage(s->image);
}

/* The Sophia peer's sealed preparation, step for step. */
static int fresh_fd(size_t len, uint64_t generation, int bursty) {
    int fd = memfd_create("x-panel", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) die("memfd_create");
    if (fallocate(fd, 0, 0, (off_t)len)) die("fallocate");
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_GROW | F_SEAL_SHRINK)) die("seal grow|shrink");
    uint8_t *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (p == MAP_FAILED) die("mmap fresh");
    if (bursty) fill_bursty_panel(p, generation);
    else fill_pixels(p, len, generation);
    if (munmap(p, len)) die("munmap fresh");
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE | F_SEAL_SEAL)) die("seal write|seal");
    return fd;
}

/* Fresh lifecycle upload of a w x h image with the given generation into
 * `dst`, copying only the rectangle (rx, ry, rw, rh). xcb closes the fd
 * after sending it; requests run in order, so the copy completes before
 * the detach and the server unmaps the segment at once. */
static void fresh_upload(struct ctx *c, xcb_drawable_t dst, unsigned w, unsigned h,
                         uint64_t generation, unsigned rx, unsigned ry, unsigned rw,
                         unsigned rh, int notify) {
    int fd = fresh_fd((size_t)w * h * 4, generation, c->bursty);
    uint32_t seg = xcb_generate_id(c->conn);
    xcb_shm_attach_fd(c->conn, seg, fd, 1);
    xcb_shm_put_image(c->conn, dst, XGContextFromGC(c->gc), w, h, rx, ry, rw, rh, rx, ry,
                      c->depth, XCB_IMAGE_FORMAT_Z_PIXMAP, notify ? 1 : 0, seg, 0);
    xcb_shm_detach(c->conn, seg);
}

static int handle(struct ctx *c, xcb_generic_event_t *ev) {
    uint8_t type = ev->response_type & 0x7f;
    uint64_t now = mono_ns();
    if (type == 0) {
        xcb_generic_error_t *e = (xcb_generic_error_t *)ev;
        fprintf(stderr, "x-panel: X error %u (major %u minor %u)\n", e->error_code,
                e->major_code, e->minor_code);
        die("asynchronous X request failed");
    }
    if (type == XCB_GE_GENERIC) {
        xcb_ge_generic_event_t *ge = (xcb_ge_generic_event_t *)ev;
        if (ge->extension != c->present_opcode) return 0;
        if (ge->event_type == XCB_PRESENT_EVENT_COMPLETE_NOTIFY) {
            xcb_present_complete_notify_event_t *cn = (xcb_present_complete_notify_event_t *)ev;
            if (cn->window != c->win) die("Present completion for wrong window");
#ifdef MATCHED_GPU
            if (cn->kind != XCB_PRESENT_COMPLETE_KIND_PIXMAP || cn->serial != c->pending_serial ||
                cn->event != c->present_eid || c->complete_seen)
                die("stale, duplicate or foreign GPU completion");
#endif
            if (cn->kind == XCB_PRESENT_COMPLETE_KIND_PIXMAP && cn->serial == c->pending_serial) {
                if (cn->mode == XCB_PRESENT_COMPLETE_MODE_SKIP)
                    die("Present skipped a frame; no delivered-frame row");
                if (cn->mode != XCB_PRESENT_COMPLETE_MODE_COPY &&
                    cn->mode != XCB_PRESENT_COMPLETE_MODE_FLIP &&
                    cn->mode != XCB_PRESENT_COMPLETE_MODE_SUBOPTIMAL_COPY)
                    die("unknown Present completion mode");
                if (cn->ust > UINT64_MAX / 1000ull) die("Present timestamp overflow");
                c->complete_seen = 1;
                c->complete_ust_ns = cn->ust * 1000ull;
                c->complete_recv_ns = now;
            }
            return 1;
        }
        if (ge->event_type == XCB_PRESENT_EVENT_IDLE_NOTIFY) {
            xcb_present_idle_notify_event_t *in = (xcb_present_idle_notify_event_t *)ev;
#ifdef MATCHED_GPU
            gpu_x_idle(c, in);
#else
            for (unsigned i = 0; i < c->nbacks; i++)
                if (in->window == c->win && c->backs[i].pixmap == in->pixmap &&
                    c->backs[i].serial == in->serial) c->backs[i].busy = 0;
#endif
            return 1;
        }
        return 0;
    }
    if (type == c->shm_first_event + XCB_SHM_COMPLETION) {
        xcb_shm_completion_event_t *sc = (xcb_shm_completion_event_t *)ev;
        if (sc->drawable == c->bulk_target) c->bulk_busy = 0;
        return 1;
    }
    return 0;
}

static void pump_one(struct ctx *c) {
#ifdef MATCHED_GPU
    xcb_generic_event_t *ev = NULL;
    while (!(ev = xcb_poll_for_event(c->conn))) {
        uint64_t now = mono_ns();
        if (xcb_connection_has_error(c->conn)) die("GPU X connection lost");
        if (now >= c->deadline) die("GPU consumer/presentation deadline expired");
        int ms = (int)((c->deadline - now + 999999) / 1000000);
        struct pollfd fd = {.fd = xcb_get_file_descriptor(c->conn), .events = POLLIN};
        if (poll(&fd, 1, ms) < 0 && errno != EINTR) die("GPU X event wait failed");
    }
#else
    xcb_generic_event_t *ev = xcb_wait_for_event(c->conn);
#endif
    if (!ev) die("connection lost while waiting for an event");
    handle(c, ev);
    free(ev);
}

static void wait_idle(struct ctx *c, struct back *b) {
    while (b->busy) pump_one(c);
}

/* Present `b` with an optional update region; returns presented ns and
 * sets *source. */
#ifdef DIAGNOSTIC_COLOUR_HOLD
#include "../gpu/hold.h"
static void colour_dispatch(void *, int, short, uint64_t);
#endif
static int64_t present_and_wait(struct ctx *c, struct back *b, uint32_t serial,
                                xcb_xfixes_region_t update, uint64_t *submit_out,
                                const char **source) {
    c->pending_serial = serial;
    c->complete_seen = 0;
    b->busy = 1;
    b->serial = serial;
    uint64_t submit = mono_ns();
    xcb_present_pixmap(c->conn, c->win, b->pixmap, serial, 0, update, 0, 0, 0, 0, 0,
                       XCB_PRESENT_OPTION_NONE, 0, 0, 0, 0, NULL);
    xcb_flush(c->conn);
    *submit_out = submit;
    while (!c->complete_seen) pump_one(c);
    if (c->complete_ust_ns >= submit && c->complete_ust_ns <= c->complete_recv_ns) {
        c->clock_ok++;
        *source = "xpresent";
#ifdef DIAGNOSTIC_COLOUR_HOLD
        hold_presented(serial, c->complete_ust_ns, colour_dispatch, c);
#endif
        return (int64_t)c->complete_ust_ns;
    }
    c->clock_bad++;
#ifdef MATCHED_GPU
    die("GPU Present timestamp is outside the submission/receipt interval");
#endif
    *source = "xpresent_recv";
    return (int64_t)c->complete_recv_ns;
}

/* Put the panel content for `generation` into back buffer b, uploading only
 * the rectangle given (full panel unless bursty). */
static void paint_panel(struct ctx *c, struct back *b, unsigned w, unsigned h,
                        uint64_t generation, unsigned rx, unsigned ry, unsigned rw,
                        unsigned rh) {
#ifdef MATCHED_GPU
    if (w != 1920 || h != 24 || rx || ry || rw != w || rh != h)
        die("GPU full-panel shape differs");
    gpu_x_paint(c, b, generation);
#else
    if (c->fresh_lifecycle) {
        fresh_upload(c, b->pixmap, w, h, generation, rx, ry, rw, rh, 0);
    } else {
        /* reuse: the presented pixmap is the shared memory itself */
        if (c->bursty) fill_bursty_panel((uint8_t *)b->shm.info.shmaddr, generation);
        else fill_pixels((uint8_t *)b->shm.info.shmaddr, b->shm.len, generation);
    }
#endif
}

static void sleep_until_ns(uint64_t t) {
    struct timespec ts = {(time_t)(t / 1000000000ull), (long)(t % 1000000000ull)};
    while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts, NULL) == EINTR) {
    }
}

static void usage(void) {
    fprintf(stderr,
            "usage: x-panel --journal PATH [--display D]\n"
            "               [--workload fresh|bursty|retained|idle] [--lifecycle fresh|reuse]\n"
            "               [--frames N] [--warmup W] [--buffers 2] [--bulk]\n"
            "               [--interval-ms 250] [--idle-sec 60]\n"
            "defaults: fresh/retained 1000 frames + 120 warmup; bursty 240 + 8;\n"
            "          idle 1 frame, no warmup. Panel 1920x24 at (0,0).\n");
    exit(1);
}

#ifdef DIAGNOSTIC_COLOUR_HOLD
#include "../gpu/hold.h"
static void colour_dispatch(void *context, int control, short events, uint64_t deadline) {
    struct ctx *c = context;
    for (;;) {
        xcb_generic_event_t *event;
        while ((event = xcb_poll_for_event(c->conn))) {
            if (hold_now() >= deadline) die("colour hold event deadline");
            handle(c, event); free(event);
        }
        if (xcb_connection_has_error(c->conn) || xcb_flush(c->conn) <= 0)
            die("colour hold X connection lost");
        struct pollfd fds[2] = {
            {.fd = xcb_get_file_descriptor(c->conn), .events = POLLIN},
            {.fd = control, .events = events}
        };
        hold_poll(fds, 2, deadline);
        if (fds[0].revents & (POLLERR | POLLHUP | POLLNVAL)) die("colour hold X lost");
        if (fds[1].revents) {
            while ((event = xcb_poll_for_event(c->conn))) {
                if (hold_now() >= deadline) die("colour hold event deadline");
                handle(c, event); free(event);
            }
            if (xcb_connection_has_error(c->conn)) die("colour hold X lost");
            return;
        }
    }
}
#endif

int main(int argc, char **argv) {
    const char *display = NULL, *journal_path = NULL;
    const char *workload_name = "fresh", *lifecycle = "fresh";
    int frames = -1, warmup = -1, bulk = 0;
    unsigned nbacks = 2, interval_ms = 250, idle_sec = 60;
    const unsigned width = 1920, height = 24;
    static struct option opts[] = {
#ifdef DIAGNOSTIC_COLOUR_HOLD
        {"hold-fd", 1, 0, 'H'},
#endif
        {"display", 1, 0, 'd'},   {"frames", 1, 0, 'n'},      {"warmup", 1, 0, 'u'},
        {"buffers", 1, 0, 'b'},   {"bulk", 0, 0, 'B'},        {"journal", 1, 0, 'j'},
        {"lifecycle", 1, 0, 'L'}, {"workload", 1, 0, 'W'},    {"interval-ms", 1, 0, 'I'},
        {"idle-sec", 1, 0, 'S'},  {0, 0, 0, 0}};
    int o;
    while ((o = getopt_long(argc, argv, "", opts, NULL)) != -1) {
        switch (o) {
#ifdef DIAGNOSTIC_COLOUR_HOLD
        case 'H': hold_fd(optarg); break;
#endif
        case 'd': display = optarg; break;
        case 'n': frames = atoi(optarg); break;
        case 'u': warmup = atoi(optarg); break;
        case 'b': nbacks = (unsigned)atoi(optarg); break;
        case 'B': bulk = 1; break;
        case 'j': journal_path = optarg; break;
        case 'L': lifecycle = optarg; break;
        case 'W': workload_name = optarg; break;
        case 'I': interval_ms = (unsigned)atoi(optarg); break;
        case 'S': idle_sec = (unsigned)atoi(optarg); break;
        default: usage();
        }
    }
    enum workload wl;
    if (!strcmp(workload_name, "fresh")) wl = W_FRESH;
    else if (!strcmp(workload_name, "bursty")) wl = W_BURSTY;
    else if (!strcmp(workload_name, "retained")) wl = W_RETAINED;
    else if (!strcmp(workload_name, "idle")) wl = W_IDLE;
    else usage();
    if (!journal_path || nbacks < 1) usage();
    if (bulk && wl != W_FRESH) die("--bulk applies to the fresh workload only");
    if (strcmp(lifecycle, "fresh") && strcmp(lifecycle, "reuse")) usage();
    if (wl == W_IDLE) {
        if ((frames >= 0 && frames != 1) || (warmup > 0)) die("idle is one frame, no warmup");
        frames = 1;
        warmup = 0;
    }
    if (frames < 0) frames = wl == W_BURSTY ? 240 : 1000;
    if (warmup < 0) warmup = wl == W_BURSTY ? 8 : 120;
    if (frames < 1 || interval_ms < 1) usage();
#ifdef MATCHED_GPU
    if (wl != W_FRESH || strcmp(lifecycle, "fresh") || nbacks != 2 || bulk ||
        frames > 1000000 || warmup < 0 || warmup > 1000000)
        die("GPU mode requires fresh full panel, two buffers and bounded frames");
#endif

#ifdef DIAGNOSTIC_COLOUR_HOLD
    hold_config((unsigned)frames, (unsigned)warmup);
#endif
    struct ctx c = {0};
    c.fresh_lifecycle = !strcmp(lifecycle, "fresh");
    c.bursty = wl == W_BURSTY;
    c.dpy = XOpenDisplay(display);
    if (!c.dpy) die("XOpenDisplay");
    XSetEventQueueOwner(c.dpy, XCBOwnsEventQueue);
    c.conn = XGetXCBConnection(c.dpy);
    int screen = DefaultScreen(c.dpy);
    c.depth = DefaultDepth(c.dpy, screen);

#ifndef MATCHED_GPU
    const xcb_query_extension_reply_t *shm_ext = xcb_get_extension_data(c.conn, &xcb_shm_id);
    if (!shm_ext || !shm_ext->present) die("no MIT-SHM");
    c.shm_first_event = shm_ext->first_event;
    xcb_shm_query_version_reply_t *sv =
        xcb_shm_query_version_reply(c.conn, xcb_shm_query_version(c.conn), NULL);
    if (!sv) die("MIT-SHM version");
    int shm_fd_ok = sv->major_version > 1 || (sv->major_version == 1 && sv->minor_version >= 2);
    int shm_pixmaps = sv->shared_pixmaps && sv->pixmap_format == XCB_IMAGE_FORMAT_Z_PIXMAP;
    free(sv);
    if (c.fresh_lifecycle && !shm_fd_ok) die("--lifecycle fresh needs MIT-SHM 1.2 (fd passing)");
    if (!c.fresh_lifecycle && !shm_pixmaps) die("--lifecycle reuse needs shared ZPixmap pixmaps");
#endif
    const xcb_query_extension_reply_t *pres = xcb_get_extension_data(c.conn, &xcb_present_id);
    if (!pres || !pres->present) die("no Present extension");
    c.present_opcode = pres->major_opcode;
    xcb_present_query_version_reply_t *pv =
        xcb_present_query_version_reply(c.conn, xcb_present_query_version(c.conn, 1, 0), NULL);
    if (!pv) die("Present version");
    free(pv);
    xcb_xfixes_region_t damage_region = 0;
    if (wl == W_BURSTY) {
        xcb_xfixes_query_version_reply_t *xv = xcb_xfixes_query_version_reply(
            c.conn, xcb_xfixes_query_version(c.conn, 2, 0), NULL);
        if (!xv || xv->major_version < 2) die("bursty needs XFixes 2.0 for the update region");
        free(xv);
        damage_region = xcb_generate_id(c.conn);
        xcb_rectangle_t r = {(int16_t)(width - DMG_W), 0, DMG_W, DMG_H};
        xcb_xfixes_create_region(c.conn, damage_region, 1, &r);
    }

    XSetWindowAttributes attrs;
    attrs.override_redirect = True;
    attrs.background_pixmap = None;
    attrs.event_mask = 0;
    c.win = XCreateWindow(c.dpy, RootWindow(c.dpy, screen), 0, 0, width, height, 0, c.depth,
                          InputOutput, CopyFromParent,
                          CWOverrideRedirect | CWBackPixmap | CWEventMask, &attrs);
    c.gc = XCreateGC(c.dpy, c.win, 0, NULL);
    XMapWindow(c.dpy, c.win);
    XSync(c.dpy, False);

    c.nbacks = nbacks;
    c.backs = calloc(nbacks, sizeof *c.backs);
    int shm_backs = !c.fresh_lifecycle && wl != W_RETAINED;
    for (unsigned i = 0; i < nbacks; i++) {
#ifdef MATCHED_GPU
        continue; /* Every generation imports a new pixmap. */
#endif
        if (shm_backs) {
            sysv_create(&c, &c.backs[i].shm, width, height);
            c.backs[i].pixmap = XShmCreatePixmap(c.dpy, c.win, c.backs[i].shm.info.shmaddr,
                                                 &c.backs[i].shm.info, width, height, c.depth);
            if (!c.backs[i].pixmap) die("XShmCreatePixmap");
        } else {
            c.backs[i].pixmap = XCreatePixmap(c.dpy, c.win, width, height, c.depth);
        }
    }
    if (bulk) {
        c.bulk_target = XCreatePixmap(c.dpy, c.win, 1024, 1024, c.depth);
        if (!c.fresh_lifecycle) sysv_create(&c, &c.bulk_shm, 1024, 1024);
    }

    if (wl == W_BURSTY) {
        /* Establish the static background in every back buffer before partial
         * updates. The first Present also initializes the whole window. */
        for (unsigned i = 0; i < nbacks; i++)
            paint_panel(&c, &c.backs[i], width, height, 1, 0, 0, width, height);
        XSync(c.dpy, False);
    }

    /* Retained: 8 tile pixmaps, each uploaded once with generation i+1. */
    Pixmap tiles[TILES] = {0};
    const unsigned tile_w = (width / TILES) & ~1u;
    if (wl == W_RETAINED) {
        for (unsigned i = 0; i < TILES; i++) {
            tiles[i] = XCreatePixmap(c.dpy, c.win, tile_w, height, c.depth);
            if (c.fresh_lifecycle) {
                fresh_upload(&c, tiles[i], tile_w, height, i + 1, 0, 0, tile_w, height, 0);
            } else {
                struct sysv s;
                sysv_create(&c, &s, tile_w, height);
                fill_pixels((uint8_t *)s.info.shmaddr, s.len, i + 1);
                XShmPutImage(c.dpy, tiles[i], c.gc, s.image, 0, 0, 0, 0, tile_w, height, False);
                sysv_destroy(&c, &s);
            }
        }
        xcb_flush(c.conn);
    }

    xcb_present_event_t eid = xcb_generate_id(c.conn);
#ifdef MATCHED_GPU
    c.present_eid = eid;
#endif
    xcb_present_select_input(c.conn, eid, c.win,
                             XCB_PRESENT_EVENT_MASK_COMPLETE_NOTIFY |
                                 XCB_PRESENT_EVENT_MASK_IDLE_NOTIFY);
    XSync(c.dpy, False);

    FILE *j = fopen(journal_path, "w");
    if (!j) die("open journal");
    char damage[32] = "full";
    if (wl == W_BURSTY) snprintf(damage, sizeof damage, "%ux%u+%u+0", DMG_W, DMG_H, width - DMG_W);
    journal_header(j, "x-panel", width, height, nbacks, (unsigned)frames, (unsigned)warmup, bulk,
                   bulk ? 1024 : 0, bulk ? 1024 : 0, lifecycle, workload_name,
                   wl == W_RETAINED ? TILES : 0, wl == W_BURSTY ? interval_ms : 0, damage,
                   wl == W_IDLE ? idle_sec : 0,
                   "xpresent_ust_usec_assumed_CLOCK_MONOTONIC_checked_per_frame");
#ifdef MATCHED_GPU
    gpu_x_open(&c, j);
    fprintf(j, "# pid %d\n# attach dri3_pixmap_from_buffers\n", (int)getpid());
#else
    fprintf(j, "# pid %d\n# attach %s\n", (int)getpid(),
            c.fresh_lifecycle ? "memfd_readonly_putimage" : "sysv_shm_pixmap");
#endif

    unsigned total = (unsigned)(frames + warmup);
    int64_t pace = -1;
    uint64_t t0 = mono_ns();
    for (unsigned f = 1; f <= total; f++) {
#ifdef MATCHED_GPU
        c.deadline = mono_ns() + 5000000000ull;
#endif
        struct back *b = &c.backs[(f - 1) % nbacks];
        uint64_t generation = f; /* Sophia counts warmup in its generation. */
        if (wl == W_BURSTY) {
            uint64_t due = t0 + (uint64_t)(f - 1) * interval_ms * 1000000ull;
            sleep_until_ns(due);
            pace = (int64_t)due;
        }
        wait_idle(&c, b);
        while (bulk && c.bulk_busy) pump_one(&c);

        uint64_t fill_start = mono_ns();
        xcb_xfixes_region_t update = 0;
        switch (wl) {
        case W_FRESH:
        case W_IDLE:
            paint_panel(&c, b, width, height, generation, 0, 0, width, height);
            if (bulk) {
                c.bulk_busy = 1;
                if (c.fresh_lifecycle) {
                    fresh_upload(&c, c.bulk_target, 1024, 1024, generation, 0, 0, 1024, 1024, 1);
                } else {
                    fill_pixels((uint8_t *)c.bulk_shm.info.shmaddr, c.bulk_shm.len, generation);
                    XShmPutImage(c.dpy, c.bulk_target, c.gc, c.bulk_shm.image, 0, 0, 0, 0, 1024,
                                 1024, True);
                    XFlush(c.dpy);
                }
            }
            break;
        case W_BURSTY:
            paint_panel(&c, b, width, height, generation, width - DMG_W, 0, DMG_W, DMG_H);
            update = f == 1 ? 0 : damage_region;
            break;
        case W_RETAINED:
            /* no new pixels: compose the retained tiles at rotated slots */
            for (unsigned i = 0; i < TILES; i++) {
                unsigned slot = (i + f - 1) % TILES;
                xcb_copy_area(c.conn, tiles[i], b->pixmap, XGContextFromGC(c.gc), 0, 0,
                              (int16_t)(slot * tile_w), 0, tile_w, height);
            }
            break;
        }
        uint64_t submit;
        const char *source;
        int64_t presented = present_and_wait(&c, b, f, update, &submit, &source);
        if (f > (unsigned)warmup)
            journal_frame(j, f - (unsigned)warmup, fill_start, submit, presented, pace, source);
        if (wl == W_FRESH || wl == W_RETAINED) pace = (int64_t)c.complete_recv_ns;
    }
    while (bulk && c.bulk_busy) pump_one(&c);
    XSync(c.dpy, False);

    if (wl == W_IDLE) {
        /* Anchored on frame 1's presentation: nothing at all until the end. */
        fflush(j);
        /* +2s settling, requested window, +2s teardown margin. */
        sleep_until_ns(c.complete_recv_ns + ((uint64_t)idle_sec + 4) * 1000000000ull);
    }

#ifdef MATCHED_GPU
    c.deadline = mono_ns() + 5000000000ull;
    for (unsigned i = 0; i < nbacks; i++) wait_idle(&c, &c.backs[i]);
    gpu_close(j, total);
#ifdef DIAGNOSTIC_COLOUR_HOLD
    hold_finish(colour_dispatch, &c);
#endif
#endif
    journal_footer_rusage(j);
    fprintf(j, "# presented_clock_check ok=%llu violations=%llu\n",
            (unsigned long long)c.clock_ok, (unsigned long long)c.clock_bad);
    fclose(j);

    for (unsigned i = 0; i < nbacks; i++) {
#ifdef MATCHED_GPU
        continue; /* Each imported pixmap was destroyed on exact IdleNotify. */
#endif
        XFreePixmap(c.dpy, c.backs[i].pixmap);
        if (shm_backs) sysv_destroy(&c, &c.backs[i].shm);
    }
    if (wl == W_RETAINED)
        for (unsigned i = 0; i < TILES; i++) XFreePixmap(c.dpy, tiles[i]);
    if (bulk) {
        XFreePixmap(c.dpy, c.bulk_target);
        if (!c.fresh_lifecycle) sysv_destroy(&c, &c.bulk_shm);
    }
    if (damage_region) xcb_xfixes_destroy_region(c.conn, damage_region);
    XCloseDisplay(c.dpy);
    return 0;
}
