/* Smoke-only mode setting: refuses anything except one HEADLESS-1 output. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>
#include "headless-output.h"

static struct zwlr_output_manager_v1 *manager;
static struct zwlr_output_head_v1 *head;
static unsigned heads, serial;
static int named, configured;
static void fail(const char *s) { fprintf(stderr, "headless-output: %s\n", s); exit(2); }

static void name(void *d, struct zwlr_output_head_v1 *h, const char *s) {
    (void)d; (void)h;
    if (strcmp(s, "HEADLESS-1")) fail("not HEADLESS-1");
    named = 1;
}
static void description(void *d, struct zwlr_output_head_v1 *h, const char *s) { (void)d;(void)h;(void)s; }
static void pair(void *d, struct zwlr_output_head_v1 *h, int32_t a, int32_t b) { (void)d;(void)h;(void)a;(void)b; }
static void integer(void *d, struct zwlr_output_head_v1 *h, int32_t a) { (void)d;(void)h;(void)a; }
static void current(void *d, struct zwlr_output_head_v1 *h, struct zwlr_output_mode_v1 *m) { (void)d;(void)h;(void)m; }
static void head_finished(void *d, struct zwlr_output_head_v1 *h) { (void)d;(void)h;fail("head disappeared"); }
static void mode_pair(void *d, struct zwlr_output_mode_v1 *m, int32_t a, int32_t b) { (void)d;(void)m;(void)a;(void)b; }
static void mode_integer(void *d, struct zwlr_output_mode_v1 *m, int32_t a) { (void)d;(void)m;(void)a; }
static void mode_void(void *d, struct zwlr_output_mode_v1 *m) { (void)d;(void)m; }
static const struct zwlr_output_mode_v1_listener mode_listener = {
    .size=mode_pair, .refresh=mode_integer, .preferred=mode_void, .finished=mode_void,
};
static void mode(void *d, struct zwlr_output_head_v1 *h, struct zwlr_output_mode_v1 *m) {
    (void)d;(void)h;zwlr_output_mode_v1_add_listener(m, &mode_listener, NULL);
}
static const struct zwlr_output_head_v1_listener head_listener = {
    .name=name, .description=description, .physical_size=pair, .mode=mode,
    .enabled=integer, .current_mode=current, .position=pair, .transform=integer,
    .scale=integer, .finished=head_finished,
};
static void new_head(void *d, struct zwlr_output_manager_v1 *m, struct zwlr_output_head_v1 *h) {
    (void)d;(void)m;if (++heads != 1) fail("expected exactly one head");
    head=h;zwlr_output_head_v1_add_listener(h, &head_listener, NULL);
}
static void done(void *d, struct zwlr_output_manager_v1 *m, uint32_t s) { (void)d;(void)m;serial=s; }
static void finished(void *d, struct zwlr_output_manager_v1 *m) { (void)d;(void)m;fail("manager finished"); }
static const struct zwlr_output_manager_v1_listener manager_listener = { .head=new_head, .done=done, .finished=finished };
static void global(void *d, struct wl_registry *r, uint32_t n, const char *s, uint32_t v) {
    (void)d;(void)v;
    if (!strcmp(s, zwlr_output_manager_v1_interface.name)) {
        manager=wl_registry_bind(r,n,&zwlr_output_manager_v1_interface,1);
        zwlr_output_manager_v1_add_listener(manager,&manager_listener,NULL);
    }
}
static void removed(void *d, struct wl_registry *r, uint32_t n) { (void)d;(void)r;(void)n; }
static const struct wl_registry_listener registry_listener = {.global=global, .global_remove=removed};
static void succeeded(void *d, struct zwlr_output_configuration_v1 *c) { (void)d;(void)c;configured=1; }
static void refused(void *d, struct zwlr_output_configuration_v1 *c) { (void)d;(void)c;fail("configuration refused"); }
static const struct zwlr_output_configuration_v1_listener config_listener = {.succeeded=succeeded, .failed=refused, .cancelled=refused};

int main(void) {
    const char *armed=getenv("EXTERNAL_BAR_HEADLESS_SMOKE");
    if (!armed || strcmp(armed,"1")) fail("smoke wrapper required");
    struct wl_display *display=wl_display_connect(NULL);
    if (!display) fail("connect");
    struct wl_registry *registry=wl_display_get_registry(display);
    wl_registry_add_listener(registry,&registry_listener,NULL);
    if (wl_display_roundtrip(display)<0 || wl_display_roundtrip(display)<0 || !manager || !named || heads!=1)
        fail("head discovery");
    struct zwlr_output_configuration_v1 *config=zwlr_output_manager_v1_create_configuration(manager,serial);
    zwlr_output_configuration_v1_add_listener(config,&config_listener,NULL);
    struct zwlr_output_configuration_head_v1 *setting=zwlr_output_configuration_v1_enable_head(config,head);
    zwlr_output_configuration_head_v1_set_custom_mode(setting,1920,1080,60000);
    zwlr_output_configuration_head_v1_set_position(setting,0,0);
    zwlr_output_configuration_head_v1_set_transform(setting,WL_OUTPUT_TRANSFORM_NORMAL);
    zwlr_output_configuration_head_v1_set_scale(setting,wl_fixed_from_int(1));
    zwlr_output_configuration_v1_apply(config);
    while (!configured) if (wl_display_dispatch(display)<0) fail("dispatch");
    puts("HEADLESS-1 configured 1920x1080@60 scale=1 at 0,0 (smoke only)");
    zwlr_output_configuration_v1_destroy(config);
    wl_display_disconnect(display);
    return 0;
}
