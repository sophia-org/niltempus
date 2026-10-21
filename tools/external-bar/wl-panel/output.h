/* Bounded output selection; a native multi-head run must name its output. */
struct observed_output {
    struct wl_output *object;
    uint32_t global;
    char name[128];
    int width, height, scale, transform;
};
static struct observed_output outputs[16];
static unsigned output_count;
static struct observed_output *selected_output;
static const char *requested_output;

static void output_check(struct observed_output *o) {
    if (o->width < (int)PANEL_W || o->height < (int)PANEL_H ||
        o->scale != 1 || o->transform != WL_OUTPUT_TRANSFORM_NORMAL)
        die("selected output cannot contain the panel at scale 1 without rotation");
}
static void output_geometry(void *data, struct wl_output *o, int32_t x, int32_t y,
                            int32_t pw, int32_t ph, int32_t sub, const char *make,
                            const char *model, int32_t transform) {
    (void)o;(void)x;(void)y;(void)pw;(void)ph;(void)sub;(void)make;(void)model;
    ((struct observed_output *)data)->transform=transform;
}
static void output_mode(void *data, struct wl_output *o, uint32_t flags,
                        int32_t w, int32_t h, int32_t refresh) {
    (void)o;(void)refresh;
    if (flags & WL_OUTPUT_MODE_CURRENT) {
        ((struct observed_output *)data)->width=w;
        ((struct observed_output *)data)->height=h;
    }
}
static void output_done(void *data, struct wl_output *o) {
    (void)o;if (data==selected_output) output_check(data);
}
static void output_scale(void *data, struct wl_output *o, int32_t scale) {
    (void)o;((struct observed_output *)data)->scale=scale;
}
static void output_name(void *data, struct wl_output *o, const char *name) {
    (void)o;struct observed_output *observed=data;
    if (strlen(name)>=sizeof observed->name) die("output name exceeds bound");
    if (observed==selected_output && strcmp(name, observed->name)) die("selected output identity changed");
    strcpy(observed->name,name);
}
static void output_description(void *data, struct wl_output *o, const char *s) { (void)data;(void)o;(void)s; }
static const struct wl_output_listener output_listener = {
    .geometry=output_geometry,.mode=output_mode,.done=output_done,.scale=output_scale,
    .name=output_name,.description=output_description,
};
static void output_bind(struct wl_registry *registry, uint32_t name, uint32_t version) {
    if (version<4 || output_count==16) die("named output protocol or output bound unavailable");
    struct observed_output *o=&outputs[output_count++];
    o->global=name;
    o->object=wl_registry_bind(registry,name,&wl_output_interface,4);
    wl_output_add_listener(o->object,&output_listener,o);
}
static void output_select(void) {
    if (!requested_output && output_count!=1) die("multiple outputs: pass --output NAME");
    for (unsigned i=0;i<output_count;i++) {
        if (!requested_output || !strcmp(outputs[i].name,requested_output)) {
            if (selected_output) die("duplicate output name");
            selected_output=&outputs[i];
        }
    }
    if (!selected_output || !selected_output->name[0]) die("requested output absent or unnamed");
    output_check(selected_output);
}
