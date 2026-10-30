/* Socket/event-loop fixture; no display, renderer or resource release. */
#include "../gpu/hold.h"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
struct context { int events; unsigned dispatched; };
static void dispatch(void *data, int control, short events, uint64_t deadline) {
    struct context *c = data;
    struct pollfd fds[2] = {{.fd=control,.events=events},{.fd=c->events,.events=POLLIN}};
    for (;;) {
        hold_poll(fds, 2, deadline);
        if (fds[1].revents) {
            char byte;
            if (read(c->events, &byte, 1) != 1 || byte != 'R') exit(3);
            printf("dispatch=%u\n", ++c->dispatched); fflush(stdout);
        }
        if (fds[0].revents) return;
    }
}
int main(int argc, char **argv) {
    if (argc != 4) return 3;
    hold_fd(argv[1]);
    struct context c = {.events=atoi(argv[2])};
    hold_config((unsigned)atoi(argv[3]),0);
    for (unsigned g=1;g<=3;g++) hold_presented(g,100*g,dispatch,&c);
    hold_finish(dispatch,&c);
    return 0;
}
