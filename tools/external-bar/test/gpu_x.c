#define _GNU_SOURCE
#include <assert.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>
#include <xcb/xcb.h>
#include <xcb/dri3.h>
static unsigned freed;
static int passed_fd;
uint32_t xcb_generate_id(xcb_connection_t *c) {
    (void)c;static uint32_t next=12;return next++;
}
xcb_void_cookie_t xcb_dri3_pixmap_from_buffers(xcb_connection_t *c, xcb_pixmap_t pixmap,
    xcb_window_t win, uint8_t count, uint16_t w, uint16_t h, uint32_t stride, uint32_t offset,
    uint32_t s1, uint32_t o1, uint32_t s2, uint32_t o2, uint32_t s3, uint32_t o3,
    uint8_t depth, uint8_t bpp, uint64_t modifier, const int32_t *fds) {
    (void)c;assert(pixmap && win==9 && count==1 && w==1920 && h==24);
    assert(stride==7680 && !offset && !s1 && !o1 && !s2 && !o2 && !s3 && !o3);
    assert(depth==24 && bpp==32 && !modifier && fcntl(*fds,F_GETFD)>=0);
    passed_fd=*fds;assert(!close(*fds)); /* Models XCB's ownership transfer. */
    return (xcb_void_cookie_t){0};
}
xcb_void_cookie_t xcb_free_pixmap(xcb_connection_t *c, xcb_pixmap_t pixmap) {
    (void)c; assert(pixmap); freed++; return (xcb_void_cookie_t){0};
}
#ifdef DIAGNOSTIC_INVALID_IMPORT
static unsigned invalid_case;
xcb_void_cookie_t xcb_dri3_pixmap_from_buffers_checked(xcb_connection_t *c, xcb_pixmap_t pixmap,
    xcb_window_t win, uint8_t count, uint16_t w, uint16_t h, uint32_t stride, uint32_t offset,
    uint32_t s1, uint32_t o1, uint32_t s2, uint32_t o2, uint32_t s3, uint32_t o3,
    uint8_t depth, uint8_t bpp, uint64_t modifier, const int32_t *fds) {
    xcb_dri3_pixmap_from_buffers(c, pixmap, win, count, w, h, stride, offset,
        s1, o1, s2, o2, s3, o3, depth, bpp, modifier, fds);
    return (xcb_void_cookie_t){.sequence=100};
}
xcb_generic_error_t *xcb_request_check(xcb_connection_t *c, xcb_void_cookie_t request) {
    (void)c; assert(request.sequence == 100);
    static xcb_generic_error_t e;
    e = (xcb_generic_error_t){.error_code=2, .major_code=145,
        .minor_code=XCB_DRI3_PIXMAP_FROM_BUFFERS, .full_sequence=100};
    if (invalid_case == 1) return NULL;
    if (invalid_case == 2) e.major_code++;
    if (invalid_case == 3) e.minor_code++;
    if (invalid_case == 4) e.full_sequence++;
    return &e;
}
const xcb_query_extension_reply_t *xcb_get_extension_data(xcb_connection_t *c, xcb_extension_t *e) {
    (void)c; assert(e == &xcb_dri3_id);
    static const xcb_query_extension_reply_t reply = {.present=1, .major_opcode=145};
    return invalid_case == 5 ? NULL : &reply;
}
#endif
#ifdef DIAGNOSTIC_COLOUR_HOLD
#include <xcb/present.h>
#include "../gpu/hold.h"
unsigned diagnostic_holds;
static void test_hold(unsigned, uint64_t, hold_dispatch, void *);
#define hold_presented test_hold
#endif
#define MATCHED_GPU
#define main x_panel_program
#include "../x-panel/x-panel.c"
#undef main

#ifdef DIAGNOSTIC_COLOUR_HOLD
static unsigned pending_stage=2;
static uint32_t pending_pixmap, pending_serial;
static uint64_t pending_ust;
xcb_void_cookie_t xcb_present_pixmap(xcb_connection_t *connection, xcb_window_t window,
    xcb_pixmap_t pixmap, uint32_t serial, xcb_xfixes_region_t valid, xcb_xfixes_region_t update,
    int16_t x, int16_t y, xcb_randr_crtc_t crtc, xcb_sync_fence_t wait_fence,
    xcb_sync_fence_t idle_fence, uint32_t options, uint64_t msc, uint64_t divisor,
    uint64_t remainder, uint32_t count, const xcb_present_notify_t *notifies) {
    (void)connection;(void)valid;(void)update;(void)x;(void)y;(void)crtc;
    (void)msc;(void)divisor;(void)remainder;(void)notifies;
    assert(window==9 && !wait_fence && !idle_fence && !options && !count);
    assert(pending_stage==2);
    pending_pixmap=pixmap;pending_serial=serial;pending_stage=0;
    pending_ust=(mono_ns()+999)/1000;
    return (xcb_void_cookie_t){0};
}
int xcb_flush(xcb_connection_t *connection) { (void)connection; return 1; }
int xcb_connection_has_error(xcb_connection_t *connection) { (void)connection; return 0; }
xcb_generic_event_t *xcb_poll_for_event(xcb_connection_t *connection) {
    (void)connection;
    if (pending_stage==0) {
        xcb_present_idle_notify_event_t *event=calloc(1,sizeof *event);
        *event=(xcb_present_idle_notify_event_t){.response_type=XCB_GE_GENERIC,.extension=73,
            .event_type=XCB_PRESENT_EVENT_IDLE_NOTIFY,.window=9,.event=77,
            .pixmap=pending_pixmap,.serial=pending_serial};
        pending_stage++;return (void *)event;
    }
    if (pending_stage==1) {
        while (mono_ns()<pending_ust*1000) {} /* At most one synthetic clock microsecond. */
        xcb_present_complete_notify_event_t *event=calloc(1,sizeof *event);
        *event=(xcb_present_complete_notify_event_t){.response_type=XCB_GE_GENERIC,.extension=73,
            .event_type=XCB_PRESENT_EVENT_COMPLETE_NOTIFY,.window=9,.event=77,
            .kind=XCB_PRESENT_COMPLETE_KIND_PIXMAP,.serial=pending_serial,
            .mode=XCB_PRESENT_COMPLETE_MODE_COPY,.ust=pending_ust};
        pending_stage++;return (void *)event;
    }
    return NULL;
}
static void test_hold(unsigned generation,uint64_t presented,hold_dispatch dispatch,void *context) {
    (void)dispatch; struct ctx *c=context;
    assert(generation==diagnostic_holds+1 && presented && c->complete_seen);
    for (unsigned i=0;i<c->nbacks;i++) assert(!c->backs[i].token && !c->backs[i].busy);
    diagnostic_holds++;
}
#endif

static xcb_present_idle_notify_event_t idle = {
    .response_type=XCB_GE_GENERIC, .extension=73,
    .event_type=XCB_PRESENT_EVENT_IDLE_NOTIFY,
    .window=9, .event=77, .pixmap=12, .serial=1,
};
static void refusal(unsigned which) {
    pid_t pid=fork(); assert(pid>=0);
    if (!pid) {
        struct back b={.pixmap=12,.serial=1,.busy=1,.token=1};
        struct ctx c={.win=9,.present_opcode=73,.present_eid=77,.nbacks=1,.backs=&b,
                      .pending_serial=1};
        xcb_present_idle_notify_event_t ev=idle;
        if (which==0) ev.window++;
        if (which==1) ev.event++;
        if (which==2) ev.pixmap++;
        if (which==3) ev.serial++;
        if (which==4) ev.idle_fence=1;
        if (which==5) b.busy=0;
        if (which==6) b.token=0;
        if (which<7) handle(&c,(xcb_generic_event_t*)&ev);
        else {
            xcb_present_complete_notify_event_t cn={
                .response_type=XCB_GE_GENERIC,.extension=73,
                .event_type=XCB_PRESENT_EVENT_COMPLETE_NOTIFY,.window=9,.event=77,
                .kind=XCB_PRESENT_COMPLETE_KIND_PIXMAP,.serial=1,
                .mode=XCB_PRESENT_COMPLETE_MODE_COPY,.ust=1};
            if (which==7) cn.serial++;
            if (which==8) cn.event++;
            if (which==9) cn.kind=XCB_PRESENT_COMPLETE_KIND_NOTIFY_MSC;
            if (which==10) c.complete_seen=1;
            if (which==11) cn.mode=XCB_PRESENT_COMPLETE_MODE_SKIP;
            handle(&c,(xcb_generic_event_t*)&cn);
        }
        _exit(0);
    }
    int status; assert(waitpid(pid,&status,0)==pid);
    assert(WIFEXITED(status) && WEXITSTATUS(status)==2);
}
int main(void) {
#ifdef DIAGNOSTIC_INVALID_IMPORT
    for (invalid_case = 0; invalid_case < 6; invalid_case++) {
        pid_t child = fork(); assert(child >= 0);
        if (!child) {
            FILE *j = tmpfile(); assert(j); gpu_open(j, "fixture");
            struct back b = {0}; struct ctx c = {.win=9};
            gpu_x_paint(&c, &b, 1);
            _exit(99);
        }
        int status; assert(waitpid(child, &status, 0) == child);
        assert(WIFEXITED(status) && WEXITSTATUS(status) == (invalid_case ? 2 : 42));
    }
    puts("X invalid-import identity controls passed; synthetic errors, no display/GPU");
    return 0;
#endif
    for (unsigned i=0;i<12;i++) refusal(i);
    FILE *j=tmpfile(); assert(j); gpu_open(j,"test");
    struct back backs[2]={0};
    struct ctx c={.win=9,.present_opcode=73,.present_eid=77,.nbacks=2,.backs=backs,
                  .pending_serial=2};
#ifdef DIAGNOSTIC_COLOUR_HOLD
    c.fresh_lifecycle=true;
    for (unsigned generation=1;generation<=3;generation++) {
        struct back *b=&backs[(generation-1)%2];
        paint_panel(&c,b,1920,24,generation,0,0,1920,24);
        uint64_t submitted; const char *source;
        c.deadline=mono_ns()+5000000000ull;
        assert(present_and_wait(&c,b,generation,0,&submitted,&source)>0);
        assert(diagnostic_holds==generation && !b->token && !strcmp(source,"xpresent"));
    }
    gpu_close(j,3);fclose(j);
#else
    gpu_x_paint(&c,&backs[0],1);assert(fcntl(passed_fd,F_GETFD)==-1);
    backs[0].serial=1;backs[0].busy=1;
    gpu_x_paint(&c,&backs[1],2);assert(fcntl(passed_fd,F_GETFD)==-1);
    backs[1].serial=2;backs[1].busy=1;
    xcb_present_complete_notify_event_t cn={
        .response_type=XCB_GE_GENERIC,.extension=73,.event_type=XCB_PRESENT_EVENT_COMPLETE_NOTIFY,
        .window=9,.event=77,.kind=XCB_PRESENT_COMPLETE_KIND_PIXMAP,.serial=2,
        .mode=XCB_PRESENT_COMPLETE_MODE_COPY,.ust=1};
    handle(&c,(xcb_generic_event_t*)&cn);
    assert(c.complete_seen && backs[0].busy && backs[1].busy && !freed);
    xcb_present_idle_notify_event_t second=idle;second.pixmap=13;second.serial=2;
    handle(&c,(xcb_generic_event_t*)&second);
    assert(backs[0].busy && !backs[1].busy && !backs[1].token && !backs[1].pixmap && freed==1);
    handle(&c,(xcb_generic_event_t*)&idle);
    assert(!backs[0].busy && !backs[0].token && !backs[0].pixmap && freed==2);
    gpu_close(j,2);fclose(j);
#endif
    puts("X GPU completion/release controls passed; no display/GPU");
}
