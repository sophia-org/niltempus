/* Untimed root composition read, not a producer readback or scanout claim. */
#include "x_pixels.h"
#include <stdlib.h>
static void refuse(const char *why) { fprintf(stderr, "X capture refused: %s\n", why); exit(2); }
int main(int argc, char **argv) {
    (void)argv;
    if (argc != 1) refuse("no arguments; DISPLAY/XAUTHORITY come from the pinned native wrapper");
    Display *display = XOpenDisplay(NULL);
    if (!display) refuse("display");
    if (ScreenCount(display) != 1) refuse("single screen required");
    Atom manager = XInternAtom(display, "_NET_WM_CM_S0", True);
    if (manager != None && XGetSelectionOwner(display, manager) != None)
        refuse("compositing manager owns the screen");
    Window root = DefaultRootWindow(display);
    XWindowAttributes attrs;
    if (!XGetWindowAttributes(display, root, &attrs) || attrs.width != 1920 || attrs.height != 1080 || attrs.depth != 24)
        refuse("root geometry/depth");
    XImage *image = XGetImage(display, root, 0, 0, 1920, 1080, AllPlanes, ZPixmap);
    if (!x_pixels(image, stdout)) refuse("root image/layout/write");
    XDestroyImage(image);
    XCloseDisplay(display);
    return 0;
}
