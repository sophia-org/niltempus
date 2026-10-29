#ifndef MATCHED_GPU_PRODUCER_H
#define MATCHED_GPU_PRODUCER_H
#include <stdbool.h>
#include <stdio.h>
#include <sys/types.h>
#include "gpu_bar_bridge.h"
#define GPU_XR24 0x34325258u
#define GPU_PCI "0000:03:00.0"
void gpu_die(const char *message);
bool gpu_device_matches_at(const char *sysfs, dev_t device);
bool gpu_frame_valid(const GpuBarFrame *frame, uint64_t generation);
void gpu_open(FILE *journal, const char *transport);
GpuBarFrame gpu_render(uint64_t generation);
void gpu_release(uint64_t token);
void gpu_close(FILE *journal, uint64_t expected);
#endif
