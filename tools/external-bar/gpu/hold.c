#define _GNU_SOURCE
#include "hold.h"
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/random.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
static int control = -1;
static unsigned next = 1;
static void refuse(const char *why) {
    fprintf(stderr, "colour-hold refused: %s\n", why);
    exit(2); /* No retries or invented consumer releases. */
}
uint64_t hold_now(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) refuse("clock");
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}
int hold_poll(struct pollfd *fds, unsigned count, uint64_t deadline) {
    for (;;) {
        uint64_t now = hold_now();
        if (now >= deadline) refuse("deadline");
        uint64_t ms = (deadline - now + 999999) / 1000000;
        int n = poll(fds, count, ms > INT_MAX ? INT_MAX : (int)ms);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) refuse("poll/deadline");
        return n;
    }
}
void hold_fd(const char *value) {
    char *end;
    errno = 0;
    long n = strtol(value, &end, 10);
    char canonical[32];
    snprintf(canonical, sizeof canonical, "%ld", n);
    if (control != -1 || errno || *end || strcmp(value, canonical) || n < 3 || n > INT_MAX)
        refuse("invalid or duplicate hold fd");
    struct stat st;
    struct sockaddr_storage address;
    socklen_t len = sizeof address;
    int type; socklen_t type_len = sizeof type;
    struct ucred peer; socklen_t peer_len = sizeof peer;
    if (fstat((int)n, &st) || !S_ISSOCK(st.st_mode) ||
        getsockname((int)n, (struct sockaddr *)&address, &len) || address.ss_family != AF_UNIX ||
        getsockopt((int)n, SOL_SOCKET, SO_TYPE, &type, &type_len) || type != SOCK_STREAM ||
        getsockopt((int)n, SOL_SOCKET, SO_PEERCRED, &peer, &peer_len) || peer.uid != getuid())
        refuse("hold fd must be a same-user Unix stream");
    control = (int)n;
    int flags = fcntl(control, F_GETFL);
    if (flags < 0 || fcntl(control, F_SETFL, flags | O_NONBLOCK) || fcntl(control, F_SETFD, FD_CLOEXEC))
        refuse("hold fd flags");
}
void hold_config(unsigned frames, unsigned warmup) {
    if (control < 0 || frames != 3 || warmup) refuse("requires three frames, no warmup and hold fd");
}
static void send_record(const char *text, hold_dispatch dispatch, void *context, uint64_t deadline) {
    size_t left = strlen(text);
    while (left) {
        if (hold_now() >= deadline) refuse("send deadline");
        ssize_t n = send(control, text, left, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR) continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            dispatch(context, control, POLLOUT, deadline);
            continue;
        }
        if (n <= 0) refuse("send");
        text += n; left -= (size_t)n;
    }
}
void hold_presented(unsigned generation, uint64_t presented, hold_dispatch dispatch, void *context) {
    if (generation != next || generation > 3 || !presented) refuse("presentation sequence");
    struct pollfd p = {.fd = control, .events = POLLIN};
    if (poll(&p, 1, 0) != 0) refuse("early ACK or closed controller");
    unsigned char random[16];
    if (getrandom(random, sizeof random, GRND_NONBLOCK) != sizeof random) refuse("challenge entropy");
    char nonce[33];
    for (unsigned i = 0; i < sizeof random; i++) snprintf(nonce + 2*i, 3, "%02x", random[i]);
    char ready[256], expected[160], received[160];
    snprintf(ready, sizeof ready,
        "{\"schema\":1,\"status\":\"ready\",\"generation\":%u,\"pattern\":%u,\"presented_ns\":%" PRIu64 ",\"nonce\":\"%s\"}\n",
        generation, generation == 2 ? 2 : 1, presented, nonce);
    snprintf(expected, sizeof expected,
        "{\"schema\":1,\"status\":\"ack\",\"generation\":%u,\"nonce\":\"%s\"}\n", generation, nonce);
    uint64_t deadline = hold_now() + 80000000000ull;
    send_record(ready, dispatch, context, deadline);
    size_t used = 0, length = strlen(expected);
    for (;;) {
        dispatch(context, control, POLLIN, deadline);
        ssize_t n = recv(control, received + used, sizeof received - used, 0);
        if (n < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) continue;
        if (n <= 0) refuse("ACK read/EOF");
        used += (size_t)n;
        if (used > length || memcmp(received, expected, used)) refuse("foreign or stale ACK");
        if (used == length) break;
    }
    next++;
}
void hold_finish(hold_dispatch dispatch, void *context) {
    if (next != 4) refuse("incomplete holds");
    struct pollfd p = {.fd = control, .events = POLLIN};
    if (poll(&p, 1, 0) != 0) refuse("extra ACK or closed controller");
    send_record("{\"schema\":1,\"status\":\"done\",\"generation\":3}\n", dispatch, context, hold_now()+5000000000ull);
    if (close(control)) refuse("close hold fd");
    control = -1;
}
