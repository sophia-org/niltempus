/* Exercise the capture entry point's manager refusal without opening X. */
#include <X11/Xlib.h>
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
static Atom manager;
static Window owner;
static Display *open_display(const char *unused) {
    (void)unused;
    _XPrivDisplay d = calloc(1, sizeof(*d)); assert(d);
    d->nscreens = 1; d->screens = calloc(1, sizeof(Screen)); assert(d->screens);
    return (Display *)d;
}
static Atom atom(Display *d, const char *name, Bool only) {
    (void)d; assert(!strcmp(name, "_NET_WM_CM_S0") && only); return manager;
}
static Window selection(Display *d, Atom a) { (void)d; assert(a == 99); return owner; }
static Status geometry(Display *d, Window root, XWindowAttributes *attrs) {
    (void)d; (void)root; (void)attrs;
    _exit(42); /* Reached geometry: no manager was admitted. */
}
#define XOpenDisplay open_display
#define XInternAtom atom
#define XGetSelectionOwner selection
#define XGetWindowAttributes geometry
#define main capture_main
#include "../gpu/capture_x.c"
#undef main
int main(void) {
    for (int i = 0; i < 3; i++) {
        pid_t child = fork(); assert(child >= 0);
        if (!child) {
            manager = i == 0 ? None : 99; owner = i == 2 ? 123 : None;
            char *argv[] = {"capture", NULL}; _exit(capture_main(1, argv));
        }
        int status; assert(waitpid(child, &status, 0) == child && WIFEXITED(status));
        assert(WEXITSTATUS(status) == (i == 2 ? 2 : 42));
    }
    return 0;
}
