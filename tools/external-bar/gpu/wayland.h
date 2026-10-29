/* Included only by wl-panel-gpu; no SHM fallback or explicit-sync path. */
#include "linux-dmabuf.h"
#include "producer.h"

static struct zwp_linux_dmabuf_v1 *dmabuf;
static uint32_t dmabuf_global;
static struct {
    void *table;
    size_t size;
    bool main, target, flags, formats, compatible, ready;
    bool target_matches, tranche_linear;
} dma_feedback;

static dev_t feedback_device(struct wl_array *value) {
    dev_t device;
    if (value->size != sizeof device || !value->data) die("invalid feedback device");
    memcpy(&device, value->data, sizeof device);
    return device;
}

static void dma_table(void *data, struct zwp_linux_dmabuf_feedback_v1 *object,
                      int fd, uint32_t size) {
    (void)data; (void)object;
    struct stat st;
    if (dma_feedback.main || !size || size % 16 || size > 65536 * 16 ||
        fstat(fd, &st) || st.st_size < (off_t)size)
        die("invalid dmabuf format table");
    void *map = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) die("map dmabuf format table");
    if (dma_feedback.table) munmap(dma_feedback.table, dma_feedback.size);
    dma_feedback.table = map;
    dma_feedback.size = size;
    dma_feedback.ready = false;
}

static void dma_main(void *data, struct zwp_linux_dmabuf_feedback_v1 *object,
                     struct wl_array *device) {
    (void)data; (void)object;
    if (dma_feedback.main || !dma_feedback.table ||
        !gpu_device_matches_at("/sys", feedback_device(device)))
        die("dmabuf main device is not the pinned GPU");
    dma_feedback.main = true;
    dma_feedback.ready = false;
    dma_feedback.compatible = false;
}

static void dma_target(void *data, struct zwp_linux_dmabuf_feedback_v1 *object,
                       struct wl_array *device) {
    (void)data; (void)object;
    if (!dma_feedback.main || dma_feedback.target) die("misordered dmabuf tranche");
    dma_feedback.target = true;
    dma_feedback.target_matches = gpu_device_matches_at("/sys", feedback_device(device));
}

static void dma_flags(void *data, struct zwp_linux_dmabuf_feedback_v1 *object,
                      uint32_t flags) {
    (void)data; (void)object;
    if (!dma_feedback.target || dma_feedback.flags || flags & ~1u)
        die("invalid v4 dmabuf tranche flags");
    dma_feedback.flags = true;
}

static void dma_formats(void *data, struct zwp_linux_dmabuf_feedback_v1 *object,
                        struct wl_array *indices) {
    (void)data; (void)object;
    if (!dma_feedback.target || !indices->size ||
        indices->size % 2 || !indices->data) die("invalid dmabuf tranche indices");
    for (size_t i = 0; i < indices->size; i += 2) {
        uint16_t index;
        uint32_t format;
        uint64_t modifier;
        memcpy(&index, (char *)indices->data + i, 2);
        if ((size_t)index >= dma_feedback.size / 16) die("dmabuf index outside table");
        memcpy(&format, (char *)dma_feedback.table + index * 16, 4);
        memcpy(&modifier, (char *)dma_feedback.table + index * 16 + 8, 8);
        if (format == GPU_XR24 && modifier == 0) dma_feedback.tranche_linear = true;
    }
    dma_feedback.formats = true;
}

static void dma_tranche_done(void *data, struct zwp_linux_dmabuf_feedback_v1 *object) {
    (void)data; (void)object;
    if (!dma_feedback.target || !dma_feedback.formats)
        die("incomplete dmabuf tranche");
    dma_feedback.compatible |= dma_feedback.target_matches && dma_feedback.tranche_linear;
    dma_feedback.target = dma_feedback.flags = dma_feedback.formats = false;
    dma_feedback.target_matches = dma_feedback.tranche_linear = false;
}

static void dma_done(void *data, struct zwp_linux_dmabuf_feedback_v1 *object) {
    (void)data; (void)object;
    if (!dma_feedback.main || dma_feedback.target || !dma_feedback.compatible)
        die("no same-GPU XR24/LINEAR feedback tranche");
    dma_feedback.ready = true;
    dma_feedback.main = false;
}

static const struct zwp_linux_dmabuf_feedback_v1_listener dma_listener = {
    .done = dma_done, .format_table = dma_table, .main_device = dma_main,
    .tranche_done = dma_tranche_done, .tranche_target_device = dma_target,
    .tranche_formats = dma_formats, .tranche_flags = dma_flags,
};

static void gpu_dispatch_until(uint64_t deadline) {
    uint64_t now = mono_ns();
    if (now >= deadline) die("GPU consumer/presentation deadline expired");
    uint64_t wait = deadline - now;
    if (wait > 100000000) wait = 100000000;
    struct timespec timeout = {.tv_sec = 0, .tv_nsec = (long)wait};
    if (wl_display_dispatch_timeout(display, &timeout) < 0)
        die("GPU Wayland dispatch failed");
}

static void gpu_wayland_init(struct wl_surface *surface, FILE *journal) {
    if (!dmabuf) die("linux-dmabuf v4 is required");
    struct zwp_linux_dmabuf_feedback_v1 *feedback =
        zwp_linux_dmabuf_v1_get_surface_feedback(dmabuf, surface);
    zwp_linux_dmabuf_feedback_v1_add_listener(feedback, &dma_listener, NULL);
    uint64_t deadline = mono_ns() + 5000000000ull;
    while (!dma_feedback.ready) gpu_dispatch_until(deadline);
    gpu_open(journal, "linux_dmabuf_v4");
    fprintf(journal, "# gpu_import schema=1 protocol=linux_dmabuf version=4 main_pci=" GPU_PCI
            " tranche_pci=" GPU_PCI " format=XR24 modifier=0 sync=implicit\n");
}

static void dma_created(void *data, struct zwp_linux_buffer_params_v1 *params,
                        struct wl_buffer *buffer) {
    (void)data; (void)params; (void)buffer;
    die("unexpected created event for immediate import");
}
static void dma_failed(void *data, struct zwp_linux_buffer_params_v1 *params) {
    (void)data; (void)params;
    die("compositor refused DMA-BUF import");
}
static const struct zwp_linux_buffer_params_v1_listener dma_params_listener = {
    .created = dma_created, .failed = dma_failed,
};

static struct wl_buffer *gpu_wayland_buffer(uint64_t generation, uint64_t *token,
                                            struct zwp_linux_buffer_params_v1 **retained) {
    uint64_t deadline = mono_ns() + 5000000000ull;
    while (!dma_feedback.ready) gpu_dispatch_until(deadline);
    GpuBarFrame frame = gpu_render(generation);
    struct zwp_linux_buffer_params_v1 *params = zwp_linux_dmabuf_v1_create_params(dmabuf);
    if (!params || zwp_linux_buffer_params_v1_add_listener(params, &dma_params_listener, NULL))
        die("cannot observe DMA-BUF import failure");
    zwp_linux_buffer_params_v1_add(params, frame.fd, 0, (uint32_t)frame.offset,
                                  frame.stride, 0, 0);
    close(frame.fd); /* libwayland queued its own FD copy. */
    struct wl_buffer *buffer = zwp_linux_buffer_params_v1_create_immed(
        params, frame.width, frame.height, frame.format, 0);
    if (!buffer) die("dmabuf wl_buffer creation failed");
    *retained = params; /* Keep failure listener through consumer release. */
    *token = frame.token;
    return buffer;
}
