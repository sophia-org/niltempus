#ifndef GPU_INVALID_IMPORT_H
#define GPU_INVALID_IMPORT_H
#include "producer.h"
void gpu_invalid_config(unsigned frames, unsigned warmup);
void gpu_invalid_substitute(GpuBarFrame *frame);
_Noreturn void gpu_invalid_refused(const char *transport, uint32_t code,
                                  uint32_t major, uint32_t minor, uint32_t object);
#endif
