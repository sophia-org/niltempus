/* Device-free protocol fixtures only: never linked into either GPU client. */
#define _GNU_SOURCE
#include "gpu_bar_bridge.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
struct GpuBarBridge { uint64_t generation, slots[2]; };
int gpu_bar_bridge_open(const char *node, const char *pci, GpuBarBridge **out,
                        char *error, size_t capacity) {
    (void)error; (void)capacity;
    assert(!strcmp(node, "/dev/dri/renderD128") && !strcmp(pci, "0000:03:00.0"));
    *out = calloc(1, sizeof **out);
    assert(*out);
    return 0;
}
int gpu_bar_bridge_device(GpuBarBridge *owner, GpuBarDevice *out, char *error, size_t capacity) {
    (void)error; (void)capacity; assert(owner);
    *out = (GpuBarDevice){.schema=1, .major=226, .minor=128, .pci_bus=3};
    return 0;
}
int gpu_bar_bridge_render(GpuBarBridge *owner, uint64_t generation, GpuBarFrame *out,
                          char *error, size_t capacity) {
    (void)error; (void)capacity;
    assert(owner && generation == owner->generation + 1);
    unsigned slot = owner->slots[0] ? 1 : 0;
    assert(!owner->slots[slot]);
    owner->slots[slot] = generation;
    owner->generation = generation;
    int fd = memfd_create("mock-export-not-a-dmabuf", MFD_CLOEXEC);
    assert(fd >= 0 && ftruncate(fd, 1920 * 24 * 4) == 0);
    *out = (GpuBarFrame){.token=generation, .generation=generation, .fd=fd,
        .width=1920, .height=24, .stride=1920*4, .source_bytes=1920*24*4, .format=0x34325258};
    return 0;
}
int gpu_bar_bridge_release(GpuBarBridge *owner, uint64_t token, char *error, size_t capacity) {
    (void)error; (void)capacity;
    for (unsigned i=0; i<2; i++) if (owner->slots[i] == token && token) {
        owner->slots[i]=0; return 0;
    }
    abort();
}
int gpu_bar_bridge_close(GpuBarBridge **owner, char *error, size_t capacity) {
    (void)error; (void)capacity;
    assert(*owner && !(*owner)->slots[0] && !(*owner)->slots[1]);
    free(*owner); *owner=NULL; return 0;
}
