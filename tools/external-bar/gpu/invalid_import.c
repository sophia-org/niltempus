/* Separate diagnostic clients only. The real export survives until exit. */
#define _GNU_SOURCE
#include "invalid_import.h"
#include <fcntl.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

static int retained_export = -1;
static uint64_t retained_token;

void gpu_invalid_config(unsigned frames, unsigned warmup) {
    if (frames != 1 || warmup != 0)
        gpu_die("invalid-import diagnostic requires one frame and no warmup");
}

void gpu_invalid_substitute(GpuBarFrame *frame) {
    if (retained_export != -1 || frame->generation != 1 || !frame->token ||
        !gpu_frame_valid(frame, 1) || frame->source_bytes > INT64_MAX)
        gpu_die("invalid-import diagnostic requires one completed export");
    int fd = memfd_create("gpu-invalid-import-not-dmabuf", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0 || ftruncate(fd, (off_t)frame->source_bytes) ||
        fcntl(fd, F_ADD_SEALS, F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_WRITE | F_SEAL_SEAL))
        gpu_die("invalid-import backing creation failed");
    retained_export = frame->fd;
    retained_token = frame->token;
    frame->fd = fd;
    fprintf(stderr, "gpu_invalid_import schema=1 status=injected generation=1 token=%llu bytes=%llu\n",
            (unsigned long long)retained_token, (unsigned long long)frame->source_bytes);
}

_Noreturn void gpu_invalid_refused(const char *transport, uint32_t code,
                                  uint32_t major, uint32_t minor, uint32_t object) {
    struct stat st;
    if (!retained_token || retained_export < 0 || fstat(retained_export, &st))
        gpu_die("invalid-import export custody lost");
    fprintf(stderr, "gpu_invalid_import_refused schema=1 transport=%s token=%llu "
            "code=%u major=%u minor=%u object=%u released=0\n", transport,
            (unsigned long long)retained_token, code, major, minor, object);
    exit(42); /* Expected diagnostic refusal, distinct from a local error. */
}
