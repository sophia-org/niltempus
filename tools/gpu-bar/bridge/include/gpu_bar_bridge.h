#ifndef GPU_BAR_BRIDGE_H
#define GPU_BAR_BRIDGE_H
#include <stddef.h>
#include <stdint.h>

#define GPU_BAR_BRIDGE_ABI 1
#define GPU_BAR_WIDTH 1920
#define GPU_BAR_HEIGHT 24

typedef struct GpuBarBridge GpuBarBridge;
typedef struct {
    uint64_t token, generation, offset, source_bytes, modifier;
    uint32_t width, height, stride, format;
    int32_t fd;
} GpuBarFrame;
typedef struct {
    uint32_t schema, major, minor, pci_domain, pci_bus, pci_slot, pci_function;
} GpuBarDevice;

/* All calls return 0 on success, -1 on refusal. Error text is NUL-terminated
 * and truncated to capacity-1; NULL/0 discards it. Pointers must be valid and
 * disjoint; callers serialize ALL calls on an owner. A Rust panic aborts the
 * process: it never unwinds into C or permits recovery of uncertain GPU work.
 * Every refusal stops the benchmark. No software/readback/reuse fallback.
 *
 * open selects the explicit canonical /dev/dri/renderD<N> and physical PCI
 * address (domain:bus:slot.function). This is not a Sophia grant.
 * render accepts generations 1,2,... and at most TWO unreleased fresh frames.
 * It returns only after producer GPU completion and FOREIGN/GENERAL handoff.
 * On success C owns fd (CLOEXEC duplicate): close or transfer it exactly once.
 * On failure out is empty, fd=-1. Token belongs only to this owner/frame.
 *
 * release does NOT wait for a consumer: call only after its matching release
 * event and with no unsent/re-sendable FD. Presentation is not release. The
 * bridge keeps its own export alive until this call; it never redraws it.
 * close refuses while frames remain or after a renderer failure. In the latter
 * case retain the owner until process exit; do not retry or reopen the run.
 * A successful close nulls the pointer and invalidates all aliases.
 * These calls do not establish native import, pixel parity or presentation. */
int32_t gpu_bar_bridge_open(const char *, const char *, GpuBarBridge **, char *, size_t);
int32_t gpu_bar_bridge_render(GpuBarBridge *, uint64_t, GpuBarFrame *, char *, size_t);
#ifdef DIAGNOSTIC_COLOUR_HOLD
/* Present only in the diagnostic-feature bridge; generations exactly 1..3. */
int32_t gpu_bar_bridge_render_colour(GpuBarBridge *, uint64_t, GpuBarFrame *, char *, size_t);
#endif
int32_t gpu_bar_bridge_release(GpuBarBridge *, uint64_t, char *, size_t);
int32_t gpu_bar_bridge_device(GpuBarBridge *, GpuBarDevice *, char *, size_t);
int32_t gpu_bar_bridge_close(GpuBarBridge **, char *, size_t);
#endif
