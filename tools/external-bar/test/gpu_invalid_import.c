#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <sys/wait.h>
#include "../gpu/invalid_import.c"

int main(void) {
    for (unsigned which = 0; which < 7; which++) {
        pid_t child = fork(); assert(child >= 0);
        if (!child) {
            gpu_invalid_config(which == 5 ? 2 : 1, which == 6 ? 1 : 0);
            FILE *j = tmpfile(); assert(j); gpu_open(j, "fixture");
            GpuBarFrame frame = gpu_render(1);
            struct stat source, replacement;
            assert(retained_export != frame.fd && retained_token == frame.token);
            assert(!fstat(retained_export, &source) && !fstat(frame.fd, &replacement));
            assert(source.st_ino != replacement.st_ino || source.st_dev != replacement.st_dev);
            assert(S_ISREG(replacement.st_mode) && replacement.st_size == source.st_size);
            assert((uint64_t)replacement.st_size == frame.source_bytes && gpu_frame_valid(&frame, 1));
            assert(fcntl(frame.fd, F_GET_SEALS) ==
                   (F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_WRITE | F_SEAL_SEAL));
            assert(pwrite(frame.fd, "x", 1, 0) == -1 && errno == EPERM);
            assert(!close(frame.fd) && fcntl(retained_export, F_GETFD) >= 0);
            if (which == 1) gpu_invalid_refused("fixture", 7, 0, 0, 73);
            if (which == 2) gpu_release(frame.token);
            if (which == 3) {
                assert(!close(retained_export));
                gpu_invalid_refused("fixture", 7, 0, 0, 73);
            }
            if (which == 4) gpu_render(2);
            _exit(0);
        }
        int status; assert(waitpid(child, &status, 0) == child);
        unsigned expected = which == 0 ? 0 : which == 1 ? 42 : 2;
        assert(WIFEXITED(status) && WEXITSTATUS(status) == (int)expected);
    }
    puts("Invalid import FD/custody/refusal controls passed; mock exports, no GPU");
}
