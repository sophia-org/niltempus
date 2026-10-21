/* x-panel: MIT-SHM reference panel client for the XLibre arm of the
 * external bar. Visible workload of the Sophia timing peer's panel arm:
 * a 1920x24 strip at (0,0), freshly filled every frame with the shared
 * whole-pixel fill, one frame in flight, presented through X Present from
 * a shared-memory pixmap. Bulk variant: an extra 1024x1024 (4 MiB) shared
 * image copied into an offscreen pixmap every frame and never shown, the
 * analogue of the peer's accepted-then-released 4 MiB probe.
 *
 * Modes:
 *   present   XShmCreatePixmap + PresentPixmap; presented_ns from
 *             COMPLETE_NOTIFY ust (checked against submit/receipt), buffer
 *             reuse gated on IDLE_NOTIFY. Requires shared pixmaps + Present.
 *   putimage  XShmPutImage with completion events; presented_ns is the
 *             ShmCompletion receipt (copy done, not scanout). Fallback only.
 *   auto      present when the server offers it, otherwise putimage.
 *
 * No Sophia dependency. Build with the Makefile next to this file.
 * This file never launches a server; it connects to $DISPLAY or --display.
 */
#define _GNU_SOURCE
#include <X11/Xlib-xcb.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/XShm.h>
#include <errno.h>
#include <getopt.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <linux/memfd.h>
#include <sys/ipc.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/shm.h>
#include <unistd.h>
#include <xcb/present.h>
#include <xcb/shm.h>
#include <xcb/xcb.h>

#include "../common/fill.h"
#include "../common/journal.h"

/* Fresh lifecycle (--lifecycle fresh): one sealed memfd per frame, the same
 * producer work as the Sophia peer's sealed ISB preparation: memfd_create,
 * fallocate, seal GROW|SHRINK, map, fill, unmap, seal WRITE|SEAL, then pass
 * the fd to the server (MIT-SHM 1.2 AttachFd). A write-sealed memfd can only
 * be mapped PROT_READ, so it is attached read-only, and XLibre refuses
 * ShmCreatePixmap on a read-only segment (Xext/shm/shm.c VERIFY_SHMPTR with
 * needwrite, BadAccess). So each frame's segment is copied with ShmPutImage
 * into one of --buffers server pixmaps and detached at once; the pixmap is
 * presented and reused after IdleNotify. Under glamor that copy is the GPU
 * upload, the counterpart of the Sophia renderer uploading sealed content.
 * The server maps and unmaps each frame's segment, as the Sophia Session
 * does. */
struct fresh {
    uint32_t seg;
    int live;
};

struct shmbuf {
    XShmSegmentInfo info;
    XImage *image;
    Pixmap pixmap; /* present mode only */
    size_t len;
    int busy;      /* 1 while the server may still read it */
};

struct ctx {
    Display *dpy;
    xcb_connection_t *conn;
    Window win;
    GC gc;
    int depth;
    int present;           /* 1 = present mode */
    uint8_t present_opcode;
    uint8_t shm_first_event;
    uint32_t pending_serial;    /* Present serial awaited, 0 = none */
    uint32_t pending_msc_serial; /* NotifyMSC serial awaited (putimage pacing) */
    int msc_seen;
    uint64_t msc_recv_ns;
    int have_present;           /* Present extension usable for events */
    int complete_seen;
    uint64_t complete_ust_ns;
    uint64_t complete_recv_ns;
    int shm_completion_seen;    /* panel putimage completion */
    int bulk_completion_seen;
    struct shmbuf *bufs;
    unsigned nbufs;
    struct shmbuf bulk;
    Pixmap bulk_target;
    uint64_t clock_ok, clock_bad;
    int fresh_mode;
    struct fresh bulk_fresh;
};

static void die(const char *what) {
    fprintf(stderr, "x-panel: %s (errno %d: %s)\n", what, errno, strerror(errno));
    exit(2);
}

static void create_shm(struct ctx *c, struct shmbuf *b, unsigned w, unsigned h,
                       int want_pixmap) {
    b->image = XShmCreateImage(c->dpy, DefaultVisual(c->dpy, DefaultScreen(c->dpy)),
                               c->depth, ZPixmap, NULL, &b->info, w, h);
    if (!b->image) die("XShmCreateImage");
    if (b->image->bits_per_pixel != 32) die("expected a 32 bpp visual");
    b->len = (size_t)b->image->bytes_per_line * h;
    b->info.shmid = shmget(IPC_PRIVATE, b->len, IPC_CREAT | 0600);
    if (b->info.shmid < 0) die("shmget");
    b->info.shmaddr = shmat(b->info.shmid, NULL, 0);
    if (b->info.shmaddr == (void *)-1) die("shmat");
    b->image->data = b->info.shmaddr;
    b->info.readOnly = False;
    if (!XShmAttach(c->dpy, &b->info)) die("XShmAttach");
    XSync(c->dpy, False);
    shmctl(b->info.shmid, IPC_RMID, NULL); /* both sides attached; reclaim on exit */
    if (want_pixmap) {
        b->pixmap = XShmCreatePixmap(c->dpy, c->win, b->info.shmaddr, &b->info,
                                     w, h, c->depth);
        if (!b->pixmap) die("XShmCreatePixmap");
    }
    b->busy = 0;
}

static int fresh_fd(size_t len, uint64_t generation) {
    int fd = memfd_create("x-panel", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) die("memfd_create");
    if (fallocate(fd, 0, 0, (off_t)len)) die("fallocate");
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_GROW | F_SEAL_SHRINK)) die("seal grow|shrink");
    uint8_t *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (p == MAP_FAILED) die("mmap fresh");
    fill_pixels(p, len, generation);
    if (munmap(p, len)) die("munmap fresh");
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_WRITE | F_SEAL_SEAL)) die("seal write|seal");
    return fd;
}

/* Attach a fresh sealed segment; xcb closes the fd after sending it. */
static uint32_t fresh_attach(struct ctx *c, int fd) {
    uint32_t seg = xcb_generate_id(c->conn);
    /* read-only: a write-sealed memfd cannot be mapped PROT_WRITE */
    xcb_shm_attach_fd(c->conn, seg, fd, 1);
    return seg;
}

static void fresh_release(struct ctx *c, struct fresh *f) {
    if (!f->live) return;
    xcb_shm_detach(c->conn, f->seg);
    f->live = 0;
}

/* Handle one xcb event; returns 1 if it was consumed as a state change. */
static int handle(struct ctx *c, xcb_generic_event_t *ev) {
    uint8_t type = ev->response_type & 0x7f;
    uint64_t now = mono_ns();
    if (type == XCB_GE_GENERIC) {
        xcb_ge_generic_event_t *ge = (xcb_ge_generic_event_t *)ev;
        if (ge->extension != c->present_opcode) return 0;
        if (ge->event_type == XCB_PRESENT_EVENT_COMPLETE_NOTIFY) {
            xcb_present_complete_notify_event_t *cn =
                (xcb_present_complete_notify_event_t *)ev;
            if (cn->kind == XCB_PRESENT_COMPLETE_KIND_NOTIFY_MSC &&
                cn->serial == c->pending_msc_serial) {
                c->msc_seen = 1;
                c->msc_recv_ns = now;
                return 1;
            }
            if (cn->kind == XCB_PRESENT_COMPLETE_KIND_PIXMAP &&
                cn->serial == c->pending_serial) {
                c->complete_seen = 1;
                c->complete_ust_ns = cn->ust * 1000ull;
                c->complete_recv_ns = now;
            }
            return 1;
        }
        if (ge->event_type == XCB_PRESENT_EVENT_IDLE_NOTIFY) {
            xcb_present_idle_notify_event_t *in =
                (xcb_present_idle_notify_event_t *)ev;
            for (unsigned i = 0; i < c->nbufs; i++)
                if (c->bufs[i].pixmap == in->pixmap) c->bufs[i].busy = 0;
            return 1;
        }
        return 0;
    }
    if (type == c->shm_first_event + XCB_SHM_COMPLETION) {
        xcb_shm_completion_event_t *sc = (xcb_shm_completion_event_t *)ev;
        if (sc->drawable == c->bulk_target) {
            c->bulk_completion_seen = 1;
            c->bulk.busy = 0;
        } else {
            c->shm_completion_seen = 1;
            c->complete_recv_ns = now;
            for (unsigned i = 0; i < c->nbufs; i++)
                if (c->bufs[i].busy && c->bufs[i].pixmap == 0) c->bufs[i].busy = 0;
        }
        return 1;
    }
    return 0;
}

static void pump_until(struct ctx *c, int *flag) {
    while (!*flag) {
        xcb_generic_event_t *ev = xcb_wait_for_event(c->conn);
        if (!ev) die("connection lost while waiting for an event");
        handle(c, ev);
        free(ev);
    }
}

static void usage(void) {
    fprintf(stderr,
            "usage: x-panel [--display D] [--frames N=1000] [--warmup W=120]\n"
            "               [--width 1920] [--height 24] [--x 0] [--y 0]\n"
            "               [--buffers 2] [--bulk] [--mode auto|present|putimage]\n"
            "               [--unpaced] [--lifecycle reuse|fresh]\n"
            "               --journal PATH\n");
    exit(1);
}

int main(int argc, char **argv) {
    const char *display = NULL, *journal_path = NULL, *mode = "auto";
    unsigned frames = 1000, warmup = 120, width = 1920, height = 24, nbufs = 2;
    int x = 0, y = 0, bulk = 0, unpaced = 0, fresh_mode = 0;
    const char *lifecycle = "reuse";
    static struct option opts[] = {
        {"display", 1, 0, 'd'}, {"frames", 1, 0, 'n'}, {"warmup", 1, 0, 'u'},
        {"width", 1, 0, 'w'},   {"height", 1, 0, 'h'}, {"x", 1, 0, 'x'},
        {"y", 1, 0, 'y'},       {"buffers", 1, 0, 'b'}, {"bulk", 0, 0, 'B'},
        {"mode", 1, 0, 'm'},    {"journal", 1, 0, 'j'}, {"unpaced", 0, 0, 'U'},
        {"lifecycle", 1, 0, 'L'},
        {0, 0, 0, 0}};
    int o;
    while ((o = getopt_long(argc, argv, "", opts, NULL)) != -1) {
        switch (o) {
        case 'd': display = optarg; break;
        case 'n': frames = (unsigned)atoi(optarg); break;
        case 'u': warmup = (unsigned)atoi(optarg); break;
        case 'w': width = (unsigned)atoi(optarg); break;
        case 'h': height = (unsigned)atoi(optarg); break;
        case 'x': x = atoi(optarg); break;
        case 'y': y = atoi(optarg); break;
        case 'b': nbufs = (unsigned)atoi(optarg); break;
        case 'B': bulk = 1; break;
        case 'm': mode = optarg; break;
        case 'j': journal_path = optarg; break;
        case 'U': unpaced = 1; break;
        case 'L': lifecycle = optarg; break;
        default: usage();
        }
    }
    if (!journal_path || nbufs < 1 || frames < 1) usage();
    if (!strcmp(lifecycle, "fresh")) fresh_mode = 1;
    else if (strcmp(lifecycle, "reuse")) usage();

    struct ctx c = {0};
    c.dpy = XOpenDisplay(display);
    if (!c.dpy) die("XOpenDisplay");
    XSetEventQueueOwner(c.dpy, XCBOwnsEventQueue);
    c.conn = XGetXCBConnection(c.dpy);
    int screen = DefaultScreen(c.dpy);
    c.depth = DefaultDepth(c.dpy, screen);

    int shm_major, shm_minor; Bool shm_pixmaps;
    if (!XShmQueryVersion(c.dpy, &shm_major, &shm_minor, &shm_pixmaps)) die("no MIT-SHM");
    const xcb_query_extension_reply_t *shm_ext = xcb_get_extension_data(c.conn, &xcb_shm_id);
    if (!shm_ext || !shm_ext->present) die("no MIT-SHM (xcb)");
    c.shm_first_event = shm_ext->first_event;
    const xcb_query_extension_reply_t *pres = xcb_get_extension_data(c.conn, &xcb_present_id);
    int can_present = pres && pres->present && shm_pixmaps &&
                      XShmPixmapFormat(c.dpy) == ZPixmap;
    if (!strcmp(mode, "present")) {
        if (!can_present) die("present mode requested but Present + shared pixmaps unavailable");
        c.present = 1;
    } else if (!strcmp(mode, "putimage")) {
        c.present = 0;
    } else {
        c.present = can_present;
    }
    c.have_present = pres && pres->present;
    if (!c.present && !c.have_present && !unpaced)
        die("putimage mode needs Present for vblank pacing; pass --unpaced to run without (not comparable)");
    c.fresh_mode = fresh_mode;
    if (fresh_mode) {
        if (!c.present) die("--lifecycle fresh needs present mode (Present + shared pixmaps)");
        xcb_shm_query_version_reply_t *sv = xcb_shm_query_version_reply(
            c.conn, xcb_shm_query_version(c.conn), NULL);
        if (!sv || sv->major_version < 1 || (sv->major_version == 1 && sv->minor_version < 2))
            die("--lifecycle fresh needs MIT-SHM 1.2 (fd passing)");
        free(sv);
    }
    if (c.have_present) {
        c.present_opcode = pres->major_opcode;
        xcb_present_query_version_reply_t *v = xcb_present_query_version_reply(
            c.conn, xcb_present_query_version(c.conn, 1, 0), NULL);
        if (!v) die("Present version");
        free(v);
    }

    XSetWindowAttributes attrs;
    attrs.override_redirect = True;
    attrs.background_pixmap = None;
    attrs.event_mask = 0;
    c.win = XCreateWindow(c.dpy, RootWindow(c.dpy, screen), x, y, width, height, 0,
                          c.depth, InputOutput, CopyFromParent,
                          CWOverrideRedirect | CWBackPixmap | CWEventMask, &attrs);
    c.gc = XCreateGC(c.dpy, c.win, 0, NULL);
    XMapWindow(c.dpy, c.win);
    XSync(c.dpy, False);

    c.nbufs = nbufs;
    c.bufs = calloc(nbufs, sizeof *c.bufs);
    size_t panel_len = (size_t)width * height * 4;
    if (fresh_mode) {
        /* server-side pixmaps the fresh segments are copied into */
        for (unsigned i = 0; i < nbufs; i++) {
            c.bufs[i].pixmap = XCreatePixmap(c.dpy, c.win, width, height, c.depth);
            c.bufs[i].busy = 0;
        }
    } else {
        for (unsigned i = 0; i < nbufs; i++) create_shm(&c, &c.bufs[i], width, height, c.present);
    }
    if (bulk && fresh_mode) {
        c.bulk_target = XCreatePixmap(c.dpy, c.win, 1024, 1024, c.depth);
    } else if (bulk) {
        create_shm(&c, &c.bulk, 1024, 1024, 0);
        c.bulk_target = XCreatePixmap(c.dpy, c.win, 1024, 1024, c.depth);
    }
    if (c.have_present) {
        xcb_present_event_t eid = xcb_generate_id(c.conn);
        xcb_present_select_input(c.conn, eid, c.win,
                                 XCB_PRESENT_EVENT_MASK_COMPLETE_NOTIFY |
                                     XCB_PRESENT_EVENT_MASK_IDLE_NOTIFY);
        xcb_flush(c.conn);
    }

    FILE *j = fopen(journal_path, "w");
    if (!j) die("open journal");
    journal_header(j, "x-panel", width, height, nbufs, frames, warmup, bulk,
                   bulk ? 1024 : 0, bulk ? 1024 : 0, lifecycle,
                   c.present ? "xpresent_ust_usec_assumed_CLOCK_MONOTONIC_checked"
                             : "shm_completion_receipt_not_scanout");
    char buf[64];
    snprintf(buf, sizeof buf, "%d", getpid());
    fprintf(j, "# pid %s\n# mode %s\n# attach %s\n", buf, c.present ? "present" : "putimage",
            fresh_mode ? "fd_readonly_putimage" : "sysv");

    int64_t pace = -1; /* receipt of the event that released this submission */
    unsigned total = frames + warmup;
    for (unsigned f = 1; f <= total; f++) {
        struct shmbuf *b = &c.bufs[(f - 1) % nbufs];
        if (b->busy) {
            /* one frame in flight normally frees this before we get here */
            while (b->busy) {
                xcb_generic_event_t *ev = xcb_wait_for_event(c.conn);
                if (!ev) die("connection lost");
                handle(&c, ev);
                free(ev);
            }
        }
        if (bulk && c.bulk.busy) pump_until(&c, &c.bulk_completion_seen);

        if (!c.present && c.have_present) {
            /* putimage pacing: wait for the next vblank, as a frame callback
             * or a completed PresentPixmap would. That receipt is what
             * released this frame, so it is this frame's pace. */
            c.pending_msc_serial = 0x80000000u | f;
            c.msc_seen = 0;
            xcb_present_notify_msc(c.conn, c.win, c.pending_msc_serial, 0, 1, 0);
            xcb_flush(c.conn);
            pump_until(&c, &c.msc_seen);
            pace = (int64_t)c.msc_recv_ns;
        }
        /* Warmup frame w fills generation w; counted frame k fills
         * generation k, so counted frames match the peer's 1..F exactly. */
        uint64_t generation = f > warmup ? f - warmup : f;
        uint64_t fill_start = mono_ns();
        if (fresh_mode) {
            /* fill_start covers the whole producer lifecycle, as the peer's
             * begin_usec covers sealed preparation. */
            int fd = fresh_fd(panel_len, generation);
            uint32_t seg = fresh_attach(&c, fd);
            xcb_shm_put_image(c.conn, b->pixmap, XGContextFromGC(c.gc), width, height,
                              0, 0, width, height, 0, 0, c.depth, XCB_IMAGE_FORMAT_Z_PIXMAP,
                              0, seg, 0);
            /* Requests run in order: the copy completes before the detach,
             * so the server unmaps this frame's segment right away. */
            xcb_shm_detach(c.conn, seg);
            if (bulk) {
                int bfd = fresh_fd(1024u * 1024u * 4u, generation);
                c.bulk_fresh.seg = fresh_attach(&c, bfd);
                c.bulk_fresh.live = 1;
                c.bulk_completion_seen = 0;
                c.bulk.busy = 1;
                xcb_shm_put_image(c.conn, c.bulk_target, XGContextFromGC(c.gc), 1024, 1024,
                                  0, 0, 1024, 1024, 0, 0, c.depth, XCB_IMAGE_FORMAT_Z_PIXMAP,
                                  1, c.bulk_fresh.seg, 0);
                fresh_release(&c, &c.bulk_fresh); /* ordered after the copy */
            }
        } else {
        fill_pixels((uint8_t *)b->info.shmaddr, b->len, generation);
        if (bulk) {
            fill_pixels((uint8_t *)c.bulk.info.shmaddr, c.bulk.len, generation);
            c.bulk_completion_seen = 0;
            c.bulk.busy = 1;
            XShmPutImage(c.dpy, c.bulk_target, c.gc, c.bulk.image, 0, 0, 0, 0,
                         1024, 1024, True);
        }
        }
        uint64_t submit;
        int64_t presented = -1;
        const char *source;
        if (c.present) {
            c.pending_serial = f;
            c.complete_seen = 0;
            b->busy = 1;
            xcb_present_pixmap(c.conn, c.win, b->pixmap, f, 0, 0, 0, 0, 0, 0, 0,
                               XCB_PRESENT_OPTION_NONE, 0, 0, 0, 0, NULL);
            xcb_flush(c.conn);
            submit = mono_ns();
            pump_until(&c, &c.complete_seen);
            if (c.complete_ust_ns >= submit && c.complete_ust_ns <= c.complete_recv_ns) {
                presented = (int64_t)c.complete_ust_ns;
                source = "xpresent";
                c.clock_ok++;
            } else {
                presented = (int64_t)c.complete_recv_ns;
                source = "xpresent_recv";
                c.clock_bad++;
            }
        } else {
            c.shm_completion_seen = 0;
            b->busy = 1;
            XShmPutImage(c.dpy, c.win, c.gc, b->image, 0, 0, 0, 0, width, height, True);
            XFlush(c.dpy);
            submit = mono_ns();
            pump_until(&c, &c.shm_completion_seen);
            presented = (int64_t)c.complete_recv_ns;
            source = "shm_completion";
        }
        if (f > warmup)
            journal_frame(j, f - warmup, fill_start, submit, presented, pace, source);
        /* Present mode: this frame's COMPLETE_NOTIFY releases the next
         * submission (one frame in flight), so it is the next frame's pace.
         * Putimage mode sets its pace from NotifyMSC above; unpaced keeps -1. */
        if (c.present) pace = (int64_t)c.complete_recv_ns;
    }
    if (bulk && c.bulk.busy) pump_until(&c, &c.bulk_completion_seen);
    XSync(c.dpy, False);

    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    fprintf(j, "# rusage utime_ns=%llu stime_ns=%llu minflt=%ld majflt=%ld\n",
            (unsigned long long)ru.ru_utime.tv_sec * 1000000000ull +
                (unsigned long long)ru.ru_utime.tv_usec * 1000ull,
            (unsigned long long)ru.ru_stime.tv_sec * 1000000000ull +
                (unsigned long long)ru.ru_stime.tv_usec * 1000ull,
            ru.ru_minflt, ru.ru_majflt);
    if (c.present)
        fprintf(j, "# presented_clock_check ok=%llu violations=%llu\n",
                (unsigned long long)c.clock_ok, (unsigned long long)c.clock_bad);
    fclose(j);

    if (fresh_mode) {
        for (unsigned i = 0; i < nbufs; i++) XFreePixmap(c.dpy, c.bufs[i].pixmap);
        xcb_flush(c.conn);
    } else {
        for (unsigned i = 0; i < nbufs; i++) {
            XShmDetach(c.dpy, &c.bufs[i].info);
            shmdt(c.bufs[i].info.shmaddr);
        }
        if (bulk) { XShmDetach(c.dpy, &c.bulk.info); shmdt(c.bulk.info.shmaddr); }
    }
    XCloseDisplay(c.dpy);
    return 0;
}
