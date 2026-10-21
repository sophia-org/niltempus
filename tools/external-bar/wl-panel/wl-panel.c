/* wl-panel: external-bar Wayland reference panel client.
 *
 * A wlr-layer-shell wl_shm panel, 1920x24 at the top of the first output,
 * exclusive zone 24, fresh whole-pixel fill per frame, paced by frame
 * callbacks, presented times from wp_presentation feedback. Optional bulk
 * variant adds a 1024x1024 surface updated on the same submissions.
 *
 * The compositor's presentation clock is asserted to be CLOCK_MONOTONIC
 * (wp_presentation.clock_id == 1) before the journal is opened; any other
 * clock refuses the run with no journal and no numbers. Nothing is
 * converted between clocks anywhere.
 *
 * Usage: wl-panel JOURNAL [--frames N] [--warmup U] [--buffers B] [--bulk]
 * Compile-only artifact for Codex's gated runs; see README in this dir.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <unistd.h>
#include <wayland-client.h>

#include "fill.h"
#include "gen/presentation-time.h"
#include "gen/wlr-layer-shell.h"
#include "journal.h"

#define PANEL_W 1920u
#define PANEL_H 24u
#define BULK_W 1024u
#define BULK_H 1024u
#define MAX_BUFFERS 8u

struct slot {
    struct wl_buffer *buffer;
    uint8_t *data;
    bool busy;
};

struct panel {
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer;
    struct wl_shm_pool *pool;
    struct slot slots[MAX_BUFFERS];
    unsigned nslots;
    uint32_t serial;
    bool configured;
};

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct zwlr_layer_shell_v1 *layer_shell;
static struct wp_presentation *presentation;
static uint32_t presentation_clock = 0;
static bool clock_seen;

static unsigned opt_frames = 1000, opt_warmup = 120, opt_buffers = 3;
static bool opt_bulk;
static bool opt_fresh; /* --lifecycle fresh|reuse, default reuse */

static int64_t *rec_presented; /* per counted frame, -1 until presented */
static struct panel main_panel, bulk_panel;

static void die(const char *what) {
    fprintf(stderr, "wl-panel: %s\n", what);
    exit(2);
}

/* ---------- registry / presentation clock ---------- */

static void presentation_clock_id(void *data, struct wp_presentation *p,
                                  uint32_t clk_id) {
    (void)data; (void)p;
    presentation_clock = clk_id;
    clock_seen = true;
}
static const struct wp_presentation_listener presentation_listener = {
    .clock_id = presentation_clock_id,
};

static void registry_global(void *data, struct wl_registry *registry,
                            uint32_t name, const char *interface,
                            uint32_t version) {
    (void)data; (void)version;
    if (strcmp(interface, wl_compositor_interface.name) == 0)
        compositor = wl_registry_bind(registry, name,
                                      &wl_compositor_interface, 4);
    else if (strcmp(interface, wl_shm_interface.name) == 0)
        shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    else if (strcmp(interface, zwlr_layer_shell_v1_interface.name) == 0)
        layer_shell = wl_registry_bind(registry, name,
                                       &zwlr_layer_shell_v1_interface, 4);
    else if (strcmp(interface, wp_presentation_interface.name) == 0) {
        presentation = wl_registry_bind(registry, name,
                                        &wp_presentation_interface, 1);
        wp_presentation_add_listener(presentation, &presentation_listener,
                                     NULL);
    }
}
static void registry_remove(void *d, struct wl_registry *r, uint32_t n) {
    (void)d; (void)r; (void)n;
}
static const struct wl_registry_listener registry_listener = {
    .global = registry_global, .global_remove = registry_remove,
};

/* ---------- layer surface / buffers ---------- */

static void layer_configure(void *data, struct zwlr_layer_surface_v1 *layer,
                            uint32_t serial, uint32_t w, uint32_t h) {
    struct panel *p = data;
    (void)w; (void)h;
    zwlr_layer_surface_v1_ack_configure(layer, serial);
    p->serial = serial;
    p->configured = true;
}
static void layer_closed(void *data, struct zwlr_layer_surface_v1 *layer) {
    (void)data; (void)layer;
    die("layer surface closed by compositor");
}
static const struct zwlr_layer_surface_v1_listener layer_listener = {
    .configure = layer_configure, .closed = layer_closed,
};

static void buffer_release(void *data, struct wl_buffer *buffer) {
    (void)buffer;
    struct slot *s = data;
    s->busy = false;
}
static const struct wl_buffer_listener buffer_listener = {
    .release = buffer_release,
};

static void panel_init(struct panel *p, unsigned width, unsigned height,
                       uint32_t anchor, int32_t exclusive, unsigned nslots) {
    size_t stride = (size_t)width * 4, size = stride * height;
    p->surface = wl_compositor_create_surface(compositor);
    p->layer = zwlr_layer_shell_v1_get_layer_surface(
        layer_shell, p->surface, NULL, ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        "external-bar-panel");
    zwlr_layer_surface_v1_set_anchor(p->layer, anchor);
    zwlr_layer_surface_v1_set_size(p->layer, width, height);
    zwlr_layer_surface_v1_set_exclusive_zone(p->layer, exclusive);
    zwlr_layer_surface_v1_add_listener(p->layer, &layer_listener, p);
    wl_surface_commit(p->surface);
    while (!p->configured) {
        if (wl_display_dispatch(display) < 0)
            die("dispatch while waiting for configure");
    }
    if (opt_fresh) {
        p->nslots = 0; /* fresh mode creates one pool per submission */
        return;
    }
    int fd = memfd_create("wl-panel", MFD_ALLOW_SEALING | MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, (off_t)(size * nslots)) < 0)
        die("memfd setup");
    /* The pool shape libwayland accepts without its SIGBUS handler. */
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK | F_SEAL_GROW) < 0)
        die("memfd seals");
    uint8_t *map = mmap(NULL, size * nslots, PROT_READ | PROT_WRITE,
                        MAP_SHARED, fd, 0);
    if (map == MAP_FAILED)
        die("mmap pool");
    p->pool = wl_shm_create_pool(shm, fd, (int32_t)(size * nslots));
    close(fd);
    p->nslots = nslots;
    for (unsigned i = 0; i < nslots; i++) {
        p->slots[i].data = map + size * i;
        p->slots[i].buffer = wl_shm_pool_create_buffer(
            p->pool, (int32_t)(size * i), (int32_t)width, (int32_t)height,
            (int32_t)stride, WL_SHM_FORMAT_ARGB8888);
        wl_buffer_add_listener(p->slots[i].buffer, &buffer_listener,
                               &p->slots[i]);
    }
    (void)stride;
}

static struct slot *free_slot(struct panel *p) {
    for (unsigned i = 0; i < p->nslots; i++)
        if (!p->slots[i].busy)
            return &p->slots[i];
    return NULL;
}

/* ---------- per-frame feedback ---------- */

struct feedback {
    int64_t *presented_ns; /* counted frame slot or NULL (warmup/bulk) */
};

static void feedback_sync_output(void *d, struct wp_presentation_feedback *f,
                                 struct wl_output *o) {
    (void)d; (void)f; (void)o;
}
static void feedback_presented(void *data,
                               struct wp_presentation_feedback *feedback,
                               uint32_t tv_sec_hi, uint32_t tv_sec_lo,
                               uint32_t tv_nsec, uint32_t refresh, uint32_t s_hi,
                               uint32_t s_lo, uint32_t flags) {
    (void)refresh; (void)s_hi; (void)s_lo; (void)flags;
    struct feedback *fb = data;
    if (fb->presented_ns) {
        uint64_t sec = ((uint64_t)tv_sec_hi << 32) | tv_sec_lo;
        *fb->presented_ns = (int64_t)(sec * 1000000000ull + tv_nsec);
    }
    wp_presentation_feedback_destroy(feedback);
    free(fb);
}
static void feedback_discarded(void *data,
                               struct wp_presentation_feedback *feedback) {
    wp_presentation_feedback_destroy(feedback);
    free(data); /* presented stays -1: discarded is never renumbered */
}
static const struct wp_presentation_feedback_listener feedback_listener = {
    .sync_output = feedback_sync_output,
    .presented = feedback_presented,
    .discarded = feedback_discarded,
};

static uint64_t callback_time;
static bool callback_done;
static void frame_done(void *data, struct wl_callback *callback,
                       uint32_t time_ms) {
    (void)data; (void)time_ms;
    callback_time = mono_ns(); /* client receipt, per journal.h */
    callback_done = true;
    wl_callback_destroy(callback);
}
static const struct wl_callback_listener frame_listener = {.done = frame_done};

/* ---------- fresh lifecycle: one sealed memfd per submission ----------
 * Per frame: memfd_create, fallocate, seal GROW|SHRINK, mmap, fill,
 * munmap, then pool+buffer from the fd and close it. No WRITE seal:
 * wlroots' wl_shm mmaps pools PROT_READ|PROT_WRITE MAP_SHARED
 * (types/wlr_shm.c mapping_create), so a write-sealed pool would fail
 * the compositor's mmap with EPERM. The buffer is destroyed on
 * wl_buffer.release; --buffers bounds submissions in flight. */
static unsigned fresh_inflight, fresh_inflight_bulk;

struct fresh_slot {
    struct wl_buffer *buffer;
    unsigned *inflight;
};

static void fresh_release(void *data, struct wl_buffer *buffer) {
    (void)buffer;
    struct fresh_slot *s = data;
    wl_buffer_destroy(s->buffer);
    (*s->inflight)--;
    free(s);
}

static const struct wl_buffer_listener fresh_listener = {
    .release = fresh_release,
};

static void submit_fresh(struct wl_surface *surface, unsigned width,
                         unsigned height, uint64_t generation,
                         unsigned *inflight) {
    size_t size = (size_t)width * height * 4;
    int fd = memfd_create("wl-panel-fresh", MFD_ALLOW_SEALING | MFD_CLOEXEC);
    if (fd < 0 || fallocate(fd, 0, 0, (off_t)size) < 0)
        die("fresh memfd setup");
    if (fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK | F_SEAL_GROW) < 0)
        die("fresh memfd seals");
    uint8_t *map = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED)
        die("fresh mmap");
    fill_pixels(map, size, generation);
    munmap(map, size);
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, (int32_t)size);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(
        pool, 0, (int32_t)width, (int32_t)height, (int32_t)(width * 4),
        WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool); /* the buffer keeps the pool alive */
    close(fd);
    struct fresh_slot *s = calloc(1, sizeof(*s));
    if (!s)
        die("alloc");
    s->buffer = buffer;
    s->inflight = inflight;
    (*inflight)++;
    wl_buffer_add_listener(buffer, &fresh_listener, s);
    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, (int32_t)width, (int32_t)height);
}

/* ---------- main ---------- */

int main(int argc, char **argv) {
    if (argc < 2)
        die("usage: wl-panel JOURNAL [--frames N] [--warmup U] "
            "[--buffers B] [--bulk]");
    const char *journal_path = argv[1];
    for (int i = 2; i < argc; i++) {
        if (strcmp(argv[i], "--frames") == 0 && i + 1 < argc)
            opt_frames = (unsigned)strtoul(argv[++i], NULL, 10);
        else if (strcmp(argv[i], "--warmup") == 0 && i + 1 < argc)
            opt_warmup = (unsigned)strtoul(argv[++i], NULL, 10);
        else if (strcmp(argv[i], "--buffers") == 0 && i + 1 < argc)
            opt_buffers = (unsigned)strtoul(argv[++i], NULL, 10);
        else if (strcmp(argv[i], "--bulk") == 0)
            opt_bulk = true;
        else if (strcmp(argv[i], "--lifecycle") == 0 && i + 1 < argc) {
            const char *v = argv[++i];
            if (strcmp(v, "fresh") == 0)
                opt_fresh = true;
            else if (strcmp(v, "reuse") == 0)
                opt_fresh = false;
            else
                die("--lifecycle must be fresh or reuse");
        }
        else
            die("unknown argument");
    }
    if (opt_buffers < 1 || opt_buffers > MAX_BUFFERS || opt_frames < 1)
        die("bad frames/buffers");

    display = wl_display_connect(NULL);
    if (!display)
        die("no Wayland display");
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0)
        die("registry roundtrip");
    if (!compositor || !shm || !layer_shell || !presentation)
        die("compositor lacks compositor/shm/layer-shell/presentation");
    /* Binds from registry callbacks follow the first sync request. */
    if (wl_display_roundtrip(display) < 0)
        die("bound-global roundtrip");
    if (!clock_seen)
        die("no presentation clock_id");
    if (presentation_clock != CLOCK_MONOTONIC) {
        fprintf(stderr,
                "wl-panel: presentation clock_id %u is not "
                "CLOCK_MONOTONIC (1); refusing to record\n",
                presentation_clock);
        return 2; /* no journal, no numbers */
    }

    FILE *journal = fopen(journal_path, "w");
    if (!journal)
        die("open journal");
    journal_header(journal, "wl-panel", PANEL_W, PANEL_H, opt_buffers,
                   opt_frames, opt_warmup, opt_bulk ? 1 : 0,
                   opt_bulk ? BULK_W : 0, opt_bulk ? BULK_H : 0,
                   opt_fresh ? "fresh" : "reuse",
                   opt_fresh
                       ? "wp_presentation clock_id asserted CLOCK_MONOTONIC "
                         "(id=1), recorded as-is, not converted; fresh pools "
                         "seal GROW|SHRINK only (no WRITE seal: wlroots "
                         "wl_shm mmaps pools PROT_READ|PROT_WRITE MAP_SHARED)"
                       : "wp_presentation clock_id asserted CLOCK_MONOTONIC "
                         "(id=1), recorded as-is, not converted");

    panel_init(&main_panel, PANEL_W, PANEL_H,
               ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                   ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                   ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT,
               (int32_t)PANEL_H, opt_buffers);
    if (opt_bulk)
        panel_init(&bulk_panel, BULK_W, BULK_H,
                   ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                       ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT,
                   0, opt_buffers);

    unsigned total = opt_warmup + opt_frames;
    rec_presented = calloc(opt_frames, sizeof(int64_t));
    uint64_t *fill_start = calloc(total + 1, sizeof(uint64_t));
    uint64_t *submit = calloc(total + 1, sizeof(uint64_t));
    int64_t *pace = calloc(total + 1, sizeof(int64_t));
    if (!rec_presented || !fill_start || !submit || !pace)
        die("alloc");
    for (unsigned k = 0; k < opt_frames; k++)
        rec_presented[k] = -1;

    for (unsigned i = 1; i <= total; i++) {
        bool counted = i > opt_warmup;
        unsigned k = counted ? i - opt_warmup : 0; /* counted index 1..F */
        uint64_t generation = counted ? k : i;     /* fill generation */
        struct slot *slot = NULL, *bslot = NULL;
        callback_done = (i == 1);
        /* Wait for the previous frame callback and capacity on every
         * surface that takes part in this submission: a free pool slot
         * (reuse) or in-flight headroom under --buffers (fresh). */
        for (;;) {
            if (opt_fresh) {
                if (callback_done && fresh_inflight < opt_buffers &&
                    (!opt_bulk || fresh_inflight_bulk < opt_buffers))
                    break;
            } else {
                slot = free_slot(&main_panel);
                bslot = opt_bulk ? free_slot(&bulk_panel) : NULL;
                if (callback_done && slot && (!opt_bulk || bslot))
                    break;
            }
            if (wl_display_dispatch(display) < 0)
                die("dispatch while pacing");
        }
        pace[i] = (i == 1) ? -1 : (int64_t)callback_time;
        fill_start[i] = mono_ns();
        if (opt_fresh) {
            /* fill_start precedes memfd_create: the whole producer
             * lifecycle is inside fill->presented, as on the peer. */
            submit_fresh(main_panel.surface, PANEL_W, PANEL_H, generation,
                         &fresh_inflight);
            struct feedback *fb = calloc(1, sizeof(*fb));
            fb->presented_ns = counted ? &rec_presented[k - 1] : NULL;
            struct wp_presentation_feedback *pf =
                wp_presentation_feedback(presentation, main_panel.surface);
            wp_presentation_feedback_add_listener(pf, &feedback_listener, fb);
            struct wl_callback *cb = wl_surface_frame(main_panel.surface);
            wl_callback_add_listener(cb, &frame_listener, NULL);
            callback_done = false;
            if (opt_bulk) {
                submit_fresh(bulk_panel.surface, BULK_W, BULK_H, generation,
                             &fresh_inflight_bulk);
                struct feedback *bfb = calloc(1, sizeof(*bfb));
                struct wp_presentation_feedback *bpf =
                    wp_presentation_feedback(presentation, bulk_panel.surface);
                wp_presentation_feedback_add_listener(bpf, &feedback_listener,
                                                      bfb);
                wl_surface_commit(bulk_panel.surface);
            }
            submit[i] = mono_ns();
            wl_surface_commit(main_panel.surface);
            continue;
        }
        fill_pixels(slot->data, (size_t)PANEL_W * PANEL_H * 4, generation);
        slot->busy = true;
        wl_surface_attach(main_panel.surface, slot->buffer, 0, 0);
        wl_surface_damage_buffer(main_panel.surface, 0, 0, PANEL_W, PANEL_H);
        struct feedback *fb = calloc(1, sizeof(*fb));
        fb->presented_ns = counted ? &rec_presented[k - 1] : NULL;
        struct wp_presentation_feedback *pf =
            wp_presentation_feedback(presentation, main_panel.surface);
        wp_presentation_feedback_add_listener(pf, &feedback_listener, fb);
        struct wl_callback *cb = wl_surface_frame(main_panel.surface);
        wl_callback_add_listener(cb, &frame_listener, NULL);
        callback_done = false;
        if (opt_bulk) {
            fill_pixels(bslot->data, (size_t)BULK_W * BULK_H * 4, generation);
            bslot->busy = true;
            wl_surface_attach(bulk_panel.surface, bslot->buffer, 0, 0);
            wl_surface_damage_buffer(bulk_panel.surface, 0, 0, BULK_W,
                                     BULK_H);
            struct feedback *bfb = calloc(1, sizeof(*bfb));
            struct wp_presentation_feedback *bpf =
                wp_presentation_feedback(presentation, bulk_panel.surface);
            wp_presentation_feedback_add_listener(bpf, &feedback_listener,
                                                  bfb);
            wl_surface_commit(bulk_panel.surface);
        }
        submit[i] = mono_ns();
        wl_surface_commit(main_panel.surface);
    }
    /* Drain presentations for up to 5s; the rest keep presented = -1. */
    uint64_t drain_end = mono_ns() + 5000000000ull;
    for (;;) {
        unsigned got = 0;
        for (unsigned k = 0; k < opt_frames; k++)
            if (rec_presented[k] >= 0)
                got++;
        if (got == opt_frames || mono_ns() >= drain_end)
            break;
        struct timespec ts = {.tv_sec = 0, .tv_nsec = 100000000};
        if (wl_display_dispatch_timeout(display, &ts) < 0)
            break;
    }
    for (unsigned i = opt_warmup + 1; i <= total; i++) {
        unsigned k = i - opt_warmup;
        journal_frame(journal, k, fill_start[i], submit[i],
                      rec_presented[k - 1], pace[i], "wp_presentation");
    }
    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    fprintf(journal,
            "# rusage utime_ns=%lld stime_ns=%lld minflt=%lld majflt=%lld\n",
            (long long)(ru.ru_utime.tv_sec * 1000000000ll +
                        ru.ru_utime.tv_usec * 1000ll),
            (long long)(ru.ru_stime.tv_sec * 1000000000ll +
                        ru.ru_stime.tv_usec * 1000ll),
            (long long)ru.ru_minflt, (long long)ru.ru_majflt);
    fprintf(journal, "# pid %ld\n", (long)getpid());
    fprintf(journal, "# mode frame_callback\n");
    fclose(journal);
    return 0;
}
