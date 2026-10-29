#define _GNU_SOURCE
#include "producer.h"
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysmacros.h>
#include <unistd.h>

static GpuBarBridge *owner;
static uint64_t rendered, released;

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
    if (gpu_bar_bridge_render(owner, generation, &frame, error, sizeof error)) gpu_die(error);
    int flags = fcntl(frame.fd, F_GETFD);
    off_t size = lseek(frame.fd, 0, SEEK_END);
    if (!gpu_frame_valid(&frame, generation) ||
        flags < 0 || !(flags & FD_CLOEXEC) || size < 0 || (uint64_t)size != frame.source_bytes)
        gpu_die("exported frame layout, descriptor or backing size differs");
    rendered++;
    return frame;
}

void gpu_release(uint64_t token) {
    char error[512];
    if (gpu_bar_bridge_release(owner, token, error, sizeof error)) gpu_die(error);
    released++;
}

void gpu_close(FILE *journal, uint64_t expected) {
    char error[512];
    if (rendered != expected || released != expected) gpu_die("incomplete consumer release drain");
    if (gpu_bar_bridge_close(&owner, error, sizeof error)) gpu_die(error);
    fprintf(journal, "# gpu_end schema=1 rendered=%llu released=%llu\n",
            (unsigned long long)rendered, (unsigned long long)released);
}
