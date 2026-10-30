#ifndef GPU_COLOUR_HOLD_H
#define GPU_COLOUR_HOLD_H
#include <stdint.h>
#include <poll.h>
/* Diagnostic binaries only. The inherited socketpair is parent custody, not
 * SO_PEERCRED proof of the writer. Dispatch must service the compositor too. */
typedef void (*hold_dispatch)(void *, int, short, uint64_t);
void hold_fd(const char *value);
void hold_config(unsigned frames, unsigned warmup);
void hold_presented(unsigned generation, uint64_t presented, hold_dispatch dispatch, void *context);
void hold_finish(hold_dispatch dispatch, void *context);
uint64_t hold_now(void);
int hold_poll(struct pollfd *fds, unsigned count, uint64_t deadline);
#endif
