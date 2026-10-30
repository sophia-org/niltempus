#define _GNU_SOURCE
#include <assert.h>
#include <string.h>
#include <fcntl.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <sys/socket.h>
#include <wayland-client.h>
#include "producer.h"
#include "linux-dmabuf.h"
static unsigned destroyed;
static void test_destroy(struct wl_buffer *buffer) { assert(buffer); destroyed++; }
static struct capture {
    const struct zwp_linux_buffer_params_v1_listener *listener;
    int sent_fd, queued_fd, marker;
} captures[3];
static unsigned imports;
static struct zwp_linux_buffer_params_v1 *test_params(struct zwp_linux_dmabuf_v1 *d) {
    (void)d; assert(imports<3);
    return (void*)&captures[imports++];
}
static int test_listener(struct zwp_linux_buffer_params_v1 *p,
                          const struct zwp_linux_buffer_params_v1_listener *listener, void *data) {
    (void)data; ((struct capture*)p)->listener=listener; return 0;
}
static void test_add(struct zwp_linux_buffer_params_v1 *p, int fd, uint32_t plane,
                      uint32_t offset, uint32_t stride, uint32_t hi, uint32_t lo) {
    assert(!plane && !offset && stride==7680 && !hi && !lo);
    struct capture *c=(void*)p;c->sent_fd=fd;
    c->queued_fd=fcntl(fd,F_DUPFD_CLOEXEC,0);assert(c->queued_fd>=0);
}
static struct wl_buffer *test_immed(struct zwp_linux_buffer_params_v1 *p, int32_t w,
                                    int32_t h, uint32_t format, uint32_t flags) {
    struct capture *c=(void*)p;
    assert(c->listener && w==1920 && h==24 && format==GPU_XR24 && !flags);
    assert(fcntl(c->queued_fd,F_GETFD)>=0);
    return (void*)&c->marker;
}
static void test_params_destroy(struct zwp_linux_buffer_params_v1 *params) {
    struct capture *c=(void*)params;assert(c && c->listener);
    assert(!close(c->queued_fd));c->queued_fd=-1;
}
static bool test_device(const char *sysfs, dev_t device) {
    assert(!strcmp(sysfs, "/sys"));
    return major(device)==226 && (minor(device)==0 || minor(device)==128);
}
#ifdef DIAGNOSTIC_COLOUR_HOLD
#include "../wl-panel/gen/presentation-time.h"
#include "../gpu/hold.h"
unsigned diagnostic_holds;
static void *held_slot, *held_feedback;
static int dummy;
static int test_buffer_listener(struct wl_buffer *buffer, const struct wl_buffer_listener *listener, void *data) {
    (void)buffer; (void)listener; held_slot=data; return 0;
}
static void test_attach(struct wl_surface *surface, struct wl_buffer *buffer, int32_t x, int32_t y) {
    (void)surface; assert(buffer && !x && !y);
}
static void test_damage(struct wl_surface *surface, int32_t x, int32_t y, int32_t w, int32_t h) {
    (void)surface; assert(!x && !y && w==1920 && h==24);
}
static struct wp_presentation_feedback *test_feedback(struct wp_presentation *p, struct wl_surface *s) {
    (void)p; (void)s; return (void *)&dummy;
}
static int test_feedback_listener(struct wp_presentation_feedback *p, const struct wp_presentation_feedback_listener *l, void *data) {
    (void)p; (void)l; held_feedback=data; return 0;
}
static struct wl_callback *test_frame(struct wl_surface *s) { (void)s; return (void *)&dummy; }
static int test_frame_listener(struct wl_callback *c, const struct wl_callback_listener *l, void *d) {
    (void)c; (void)l; (void)d; return 0;
}
static void test_commit(struct wl_surface *);
static void test_hold(unsigned, uint64_t, hold_dispatch, void *);
#define wl_buffer_add_listener test_buffer_listener
#define wl_surface_attach test_attach
#define wl_surface_damage_buffer test_damage
#define wp_presentation_feedback(...) test_feedback(__VA_ARGS__)
#define wp_presentation_feedback_add_listener test_feedback_listener
#define wl_surface_frame test_frame
#define wl_callback_add_listener test_frame_listener
#define wl_surface_commit test_commit
#define hold_presented test_hold
#endif
#define wl_buffer_destroy test_destroy
#define zwp_linux_buffer_params_v1_destroy test_params_destroy
#define zwp_linux_dmabuf_v1_create_params test_params
#define zwp_linux_buffer_params_v1_add_listener test_listener
#define zwp_linux_buffer_params_v1_add test_add
#define zwp_linux_buffer_params_v1_create_immed test_immed
#define gpu_device_matches_at test_device
#define MATCHED_GPU
#define main wl_panel_program
#include "../wl-panel/wl-panel.c"
#undef main

#ifdef DIAGNOSTIC_COLOUR_HOLD
static void test_commit(struct wl_surface *surface) {
    (void)surface;
    struct feedback *fb=held_feedback;
    assert(fb && fb->presented_ns);
    *fb->presented_ns=100*(diagnostic_holds+1);
    free(fb); held_feedback=NULL;
}
static void test_hold(unsigned generation, uint64_t presented, hold_dispatch dispatch, void *context) {
    (void)dispatch; (void)context;
    assert(generation==diagnostic_holds+1 && presented==100*generation);
    struct fresh_slot *slot=held_slot;
    assert(slot && slot->token==generation);
    fresh_release(slot,slot->buffer); held_slot=NULL;
    diagnostic_holds++;
}
#endif

static void table(uint32_t format, uint64_t modifier) {
    struct { uint32_t format, padding; uint64_t modifier; } row = {format, 0, modifier};
    int fd = memfd_create("test-format-table", MFD_CLOEXEC);
    assert(fd>=0 && write(fd, &row, sizeof row)==sizeof row);
    dma_table(NULL,NULL,fd,sizeof row);
    assert(fcntl(fd,F_GETFD)==-1 && errno==EBADF);
}
static void main_device(dev_t dev) {
    struct wl_array a = {.size=sizeof dev, .data=&dev};
    dma_main(NULL,NULL,&a);
}
static void target(dev_t dev) {
    struct wl_array a = {.size=sizeof dev, .data=&dev};
    dma_target(NULL,NULL,&a);
}
static void formats(uint16_t index) {
    struct wl_array a = {.size=sizeof index, .data=&index};
    dma_formats(NULL,NULL,&a);
}
static void refusal(unsigned which) {
    pid_t pid=fork(); assert(pid>=0);
    if (!pid) {
        memset(&dma_feedback,0,sizeof dma_feedback);
        table(which==5 ? 0 : GPU_XR24, which==6 ? 1 : 0);
        if (which==0) { main_device(makedev(226,129)); _exit(0); }
        main_device(makedev(226,0));
        if (which==1) dma_done(NULL,NULL);
        target(makedev(226,which==4 ? 129 : 128));
        dma_flags(NULL,NULL,which==2 ? 2 : 0);
        if (which==3) dma_flags(NULL,NULL,0);
        formats(which==7 ? 1 : 0);
        if (which==8) target(makedev(226,128));
        if (which==9) {
            struct wl_array bad={.size=1,.data="x"}; feedback_device(&bad);
        }
        dma_tranche_done(NULL,NULL);
        if (which==10) dma_tranche_done(NULL,NULL);
        if (which==11) dma_params_listener.failed(NULL,NULL);
        if (which==12) dma_params_listener.created(NULL,NULL,NULL);
        dma_done(NULL,NULL);
        _exit(0);
    }
    int status; assert(waitpid(pid,&status,0)==pid);
    assert(WIFEXITED(status) && WEXITSTATUS(status)==2);
}
int main(void) {
    /* Exercise the installed dispatch timeout without a compositor or GPU. */
    int sockets[2]; assert(!socketpair(AF_UNIX,SOCK_STREAM,0,sockets));
    struct wl_display *empty=wl_display_connect_to_fd(sockets[0]); assert(empty);
    struct timespec zero={0};
    assert(wl_display_dispatch_timeout(empty,&zero)==0);
    wl_display_disconnect(empty);close(sockets[1]);
    for (unsigned i=0;i<13;i++) refusal(i);
    table(GPU_XR24,0);
    main_device(makedev(226,0));
    target(makedev(226,128));
    formats(0); /* Flags need not precede formats. */
    dma_flags(NULL,NULL,1);
    dma_tranche_done(NULL,NULL);
    dma_done(NULL,NULL);
    assert(dma_feedback.ready);
    main_device(makedev(226,128)); /* Complete atomic update, same table. */
    assert(!dma_feedback.ready);
    target(makedev(226,0)); formats(0); /* Omitted v4 flags default to zero. */
    dma_tranche_done(NULL,NULL); dma_done(NULL,NULL);
    assert(dma_feedback.ready);
    munmap(dma_feedback.table,dma_feedback.size);

    FILE *journal=tmpfile(); assert(journal);
    gpu_open(journal,"test");
#ifdef DIAGNOSTIC_COLOUR_HOLD
    int64_t presented[3]={-1,-1,-1}; rec_presented=presented;
    main_panel.surface=(void *)&dummy;
    for (unsigned generation=1;generation<=3;generation++) {
        uint64_t submitted;
        fresh_main_frame(generation,generation,true,&submitted);
        assert(submitted && diagnostic_holds==generation && !fresh_inflight);
    }
    gpu_close(journal,3); fclose(journal);
#else
    unsigned inflight=2;
    struct fresh_slot *first=calloc(1,sizeof *first), *second=calloc(1,sizeof *second);
    first->inflight=second->inflight=&inflight;
    first->buffer=gpu_wayland_buffer(1,&first->token,&first->params);
    assert(captures[0].listener==&dma_params_listener && fcntl(captures[0].sent_fd,F_GETFD)==-1);
    second->buffer=gpu_wayland_buffer(2,&second->token,&second->params);
    assert(captures[1].listener==&dma_params_listener && fcntl(captures[1].sent_fd,F_GETFD)==-1);
    fresh_release(second,second->buffer); assert(inflight==1 && destroyed==1);
    fresh_release(first,first->buffer); assert(inflight==0 && destroyed==2);
    gpu_close(journal,2); fclose(journal);
#endif
    puts("Wayland GPU feedback/release controls passed; no display/GPU");
}
