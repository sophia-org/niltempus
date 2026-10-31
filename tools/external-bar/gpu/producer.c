#define _GNU_SOURCE
#include "producer.h"
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysmacros.h>
#include <unistd.h>
#ifdef DIAGNOSTIC_INVALID_IMPORT
#include "invalid_import.h"
#endif
#ifdef DIAGNOSTIC_RELEASE_DELAY
#include <errno.h>
#include <time.h>
#endif

static GpuBarBridge *owner;
static uint64_t rendered, released;

#ifdef DIAGNOSTIC_RELEASE_DELAY
void gpu_delay_config(unsigned frames, unsigned warmup) {
    if (frames != 1 || warmup != 0)
        gpu_die("release-delay diagnostic requires one frame and no warmup");
}
static uint64_t delay_now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) gpu_die("release-delay clock failed");
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}
static void delay_record(const char *phase, uint64_t token, uint64_t now) {
    fprintf(stderr, "gpu_release_delay schema=1 phase=%s token=%llu monotonic_ns=%llu\n",
            phase, (unsigned long long)token, (unsigned long long)now);
}
static void delay_release(uint64_t token) {
    if (!token || rendered != 1 || released) gpu_die("release-delay export count differs");
    uint64_t start = delay_now(), until = start + 100000000ull;
    delay_record("held", token, start);
    struct timespec end = {(time_t)(until / 1000000000ull), (long)(until % 1000000000ull)};
    int rc;
    do { rc = clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &end, NULL); } while (rc == EINTR);
    uint64_t now = delay_now();
    if (rc || now < until || now - start > 5000000000ull) gpu_die("release-delay wait failed");
    delay_record("resume", token, now);
}
#endif

void gpu_die(const char *message) {
    fprintf(stderr, "matched-gpu: %s\n", message);
    exit(2); /* Process exit retains uncertain renderer/consumer custody. */
}

bool gpu_device_matches_at(const char *sysfs, dev_t device) {
    char path[PATH_MAX], resolved[PATH_MAX];
    int n = snprintf(path, sizeof path, "%s/dev/char/%u:%u/device",
                     sysfs, major(device), minor(device));
    if (n < 0 || (size_t)n >= sizeof path || !realpath(path, resolved)) return false;
    const char *name = strrchr(resolved, '/');
    return name && !strcasecmp(name + 1, GPU_PCI);
}

bool gpu_frame_valid(const GpuBarFrame *f, uint64_t generation) {
    if (!generation || !f->token || f->generation != generation || f->fd < 0 ||
        f->width != 1920 || f->height != 24 || f->format != GPU_XR24 ||
        f->modifier != 0 || f->offset > UINT32_MAX || f->stride < 1920 * 4)
        return false;
    uint64_t end = f->offset + (uint64_t)f->stride * 23 + 1920 * 4;
    return end <= f->source_bytes;
}

void gpu_open(FILE *journal, const char *transport) {
    char error[512];
    GpuBarDevice device;
    if (owner) gpu_die("producer already open");
    if (gpu_bar_bridge_open("/dev/dri/renderD128", GPU_PCI, &owner,
                                     error, sizeof error)) gpu_die(error);
    if (gpu_bar_bridge_device(owner, &device, error, sizeof error)) gpu_die(error);
    if (device.schema != 1 || device.major != 226 || device.minor != 128 ||
        device.pci_domain || device.pci_bus != 3 || device.pci_slot || device.pci_function)
        gpu_die("producer device differs from pinned renderD128 / " GPU_PCI);
    fprintf(journal, "# gpu_producer schema=1 transport=%s producer=vello_wgpu pci=" GPU_PCI
            " node=/dev/dri/renderD128 format=XR24 modifier=0 planes=1 buffers=2 lifecycle=fresh\n",
            transport);
}

GpuBarFrame gpu_render(uint64_t generation) {
    char error[512];
    GpuBarFrame frame;
#ifdef DIAGNOSTIC_COLOUR_HOLD
    if (gpu_bar_bridge_render_colour(owner, generation, &frame, error, sizeof error)) gpu_die(error);
#else
    if (gpu_bar_bridge_render(owner, generation, &frame, error, sizeof error)) gpu_die(error);
#endif
    int flags = fcntl(frame.fd, F_GETFD);
    off_t size = lseek(frame.fd, 0, SEEK_END);
    if (!gpu_frame_valid(&frame, generation) ||
        flags < 0 || !(flags & FD_CLOEXEC) || size < 0 || (uint64_t)size != frame.source_bytes)
        gpu_die("exported frame layout, descriptor or backing size differs");
    rendered++;
#ifdef DIAGNOSTIC_INVALID_IMPORT
    gpu_invalid_substitute(&frame);
#endif
    return frame;
}

void gpu_release(uint64_t token) {
#ifdef DIAGNOSTIC_INVALID_IMPORT
    gpu_die("invalid-import diagnostic must never release the real export");
#endif
#ifdef DIAGNOSTIC_RELEASE_DELAY
    // The caller already validated the real consumer event. Delay applying it.
    delay_release(token);
#endif
    char error[512];
    if (gpu_bar_bridge_release(owner, token, error, sizeof error)) gpu_die(error);
    released++;
#ifdef DIAGNOSTIC_RELEASE_DELAY
    delay_record("released", token, delay_now());
#endif
}

void gpu_close(FILE *journal, uint64_t expected) {
    char error[512];
    if (rendered != expected || released != expected) gpu_die("incomplete consumer release drain");
    if (gpu_bar_bridge_close(&owner, error, sizeof error)) gpu_die(error);
    fprintf(journal, "# gpu_end schema=1 rendered=%llu released=%llu\n",
            (unsigned long long)rendered, (unsigned long long)released);
}
