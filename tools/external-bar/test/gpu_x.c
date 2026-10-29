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
#define MATCHED_GPU
#define main x_panel_program
#include "../x-panel/x-panel.c"
#undef main

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
    for (unsigned i=0;i<12;i++) refusal(i);
    FILE *j=tmpfile(); assert(j); gpu_open(j,"test");
    struct back backs[2]={0};
    struct ctx c={.win=9,.present_opcode=73,.present_eid=77,.nbacks=2,.backs=backs,
                  .pending_serial=2};
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
    puts("X GPU completion/release controls passed; no display/GPU");
}
