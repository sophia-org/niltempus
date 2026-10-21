/* Protocol refusals must not become delivered-frame measurements. */
#define main x_panel_program
#include "../x-panel/x-panel.c"
#undef main
#include <assert.h>
#include <sys/wait.h>

static void completion(unsigned mode, unsigned window, uint64_t ust) {
    struct ctx c = { .win = 9, .present_opcode = 73, .pending_serial = 5 };
    xcb_present_complete_notify_event_t event = {
        .response_type = XCB_GE_GENERIC, .extension = 73,
        .event_type = XCB_PRESENT_EVENT_COMPLETE_NOTIFY,
        .kind = XCB_PRESENT_COMPLETE_KIND_PIXMAP, .mode = mode,
        .window = window, .serial = 5, .ust = ust
    };
    handle(&c, (xcb_generic_event_t *)&event);
    assert(c.complete_seen && c.complete_ust_ns == ust * 1000);
}

static void refusal(unsigned which) {
    pid_t child = fork();
    assert(child >= 0);
    if (child == 0) {
        if (which == 0) completion(XCB_PRESENT_COMPLETE_MODE_SKIP, 9, 1);
        if (which == 1) completion(255, 9, 1);
        if (which == 2) completion(XCB_PRESENT_COMPLETE_MODE_COPY, 10, 1);
        if (which == 3) completion(XCB_PRESENT_COMPLETE_MODE_COPY, 9, UINT64_MAX);
        if (which == 4) {
            struct ctx c = {0};
            xcb_generic_error_t error = { .error_code = 10 };
            handle(&c, (xcb_generic_event_t *)&error);
        }
        _exit(0);
    }
    int status;
    assert(waitpid(child, &status, 0) == child);
    assert(WIFEXITED(status) && WEXITSTATUS(status) == 2);
}

int main(void) {
    completion(XCB_PRESENT_COMPLETE_MODE_COPY, 9, 1);
    completion(XCB_PRESENT_COMPLETE_MODE_FLIP, 9, 1);
    completion(XCB_PRESENT_COMPLETE_MODE_SUBOPTIMAL_COPY, 9, 1);
    for (unsigned i = 0; i < 5; i++) refusal(i);
    struct back buffer = { .pixmap = 12, .serial = 6, .busy = 1 };
    struct ctx c = { .win = 9, .present_opcode = 73, .nbacks = 1, .backs = &buffer };
    xcb_present_idle_notify_event_t event = {
        .response_type = XCB_GE_GENERIC, .extension = 73,
        .event_type = XCB_PRESENT_EVENT_IDLE_NOTIFY,
        .window = 9, .pixmap = 12, .serial = 5
    };
    handle(&c, (xcb_generic_event_t *)&event);
    assert(buffer.busy); /* a late idle cannot release the next generation */
    event.serial = 6;
    handle(&c, (xcb_generic_event_t *)&event);
    assert(!buffer.busy);
    puts("X event controls passed (no display connection)");
    return 0;
}
