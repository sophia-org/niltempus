CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -Werror
BUILD := build
COMMON := common/fill.c common/fill.h common/journal.h
WL_GEN := wl-panel/gen/wlr-layer-shell.c wl-panel/gen/presentation-time.c wl-panel/gen/xdg-shell.c
X_PKGS := x11 xext x11-xcb xcb xcb-present xcb-shm xcb-xfixes

all: $(BUILD)/wl-panel $(BUILD)/x-panel $(BUILD)/fill-parity

$(BUILD):
	mkdir -p $@

$(BUILD)/wl-panel: wl-panel/wl-panel.c wl-panel/output.h $(COMMON) $(WL_GEN) | $(BUILD)
	$(CC) $(CFLAGS) -std=c11 -Icommon -o $@ $< common/fill.c $(WL_GEN) $$(pkg-config --cflags --libs wayland-client) -lm

$(BUILD)/x-panel: x-panel/x-panel.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c $$(pkg-config --cflags --libs $(X_PKGS))

$(BUILD)/fill-parity: test/fill_parity.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c

$(BUILD)/x-events: test/x_events.c x-panel/x-panel.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c $$(pkg-config --cflags --libs $(X_PKGS))

$(BUILD)/headless-output.h: test/wlr-output-management-unstable-v1.xml | $(BUILD)
	wayland-scanner client-header $< $@

$(BUILD)/headless-output.c: test/wlr-output-management-unstable-v1.xml | $(BUILD)
	wayland-scanner private-code $< $@

$(BUILD)/headless-output: test/headless_output.c $(BUILD)/headless-output.h $(BUILD)/headless-output.c
	$(CC) $(CFLAGS) -I$(BUILD) -o $@ $< $(BUILD)/headless-output.c $$(pkg-config --cflags --libs wayland-client)

# GPU targets are explicit and never replace the CPU artifacts above. Build the
# pinned bridge through cargo-slot first; callers pass that slot's static library.
GPU_INC := -I../gpu-bar/bridge/include -I$(BUILD) -Igpu
GPU_LIBS := -ldl -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
GPU_COMMON := gpu/producer.c gpu/producer.h ../gpu-bar/bridge/include/gpu_bar_bridge.h

$(BUILD)/linux-dmabuf.h: gpu/linux-dmabuf-v1.xml | $(BUILD)
	wayland-scanner client-header $< $@

$(BUILD)/linux-dmabuf.c: gpu/linux-dmabuf-v1.xml | $(BUILD)
	wayland-scanner private-code $< $@

$(BUILD)/wl-panel-gpu: wl-panel/wl-panel.c wl-panel/output.h gpu/wayland.h $(GPU_COMMON) $(COMMON) $(WL_GEN) $(BUILD)/linux-dmabuf.c $(BUILD)/linux-dmabuf.h $(GPU_BRIDGE) | $(BUILD)
	test -f "$(GPU_BRIDGE)"
	$(CC) $(CFLAGS) -std=c11 -DMATCHED_GPU $(GPU_INC) -Icommon -o $@ $< common/fill.c gpu/producer.c $(WL_GEN) $(BUILD)/linux-dmabuf.c $$(pkg-config --cflags --libs wayland-client) "$(GPU_BRIDGE)" $(GPU_LIBS)

$(BUILD)/x-panel-gpu: x-panel/x-panel.c gpu/x11.h $(GPU_COMMON) $(COMMON) $(GPU_BRIDGE) | $(BUILD)
	test -f "$(GPU_BRIDGE)"
	$(CC) $(CFLAGS) -DMATCHED_GPU $(GPU_INC) -o $@ $< common/fill.c gpu/producer.c $$(pkg-config --cflags --libs $(X_PKGS) xcb-dri3) "$(GPU_BRIDGE)" $(GPU_LIBS)

gpu: $(BUILD)/wl-panel-gpu $(BUILD)/x-panel-gpu

$(BUILD)/gpu-producer-test: test/gpu_producer.c test/mock_bridge.c $(GPU_COMMON) | $(BUILD)
	$(CC) $(CFLAGS) $(GPU_INC) -o $@ $< test/mock_bridge.c gpu/producer.c

$(BUILD)/gpu-wl-test: test/gpu_wayland.c test/mock_bridge.c wl-panel/wl-panel.c gpu/wayland.h $(GPU_COMMON) $(COMMON) $(WL_GEN) $(BUILD)/linux-dmabuf.c $(BUILD)/linux-dmabuf.h | $(BUILD)
	$(CC) $(CFLAGS) -std=c11 $(GPU_INC) -Icommon -o $@ $< test/mock_bridge.c common/fill.c gpu/producer.c $(WL_GEN) $(BUILD)/linux-dmabuf.c $$(pkg-config --cflags --libs wayland-client)

$(BUILD)/gpu-x-test: test/gpu_x.c test/mock_bridge.c x-panel/x-panel.c gpu/x11.h $(GPU_COMMON) $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) $(GPU_INC) -o $@ $< test/mock_bridge.c common/fill.c gpu/producer.c $$(pkg-config --cflags --libs $(X_PKGS) xcb-dri3)

gpu-check: $(BUILD)/gpu-producer-test $(BUILD)/gpu-wl-test $(BUILD)/gpu-x-test
	$(BUILD)/gpu-producer-test
	$(BUILD)/gpu-wl-test
	$(BUILD)/gpu-x-test
.PHONY: all gpu gpu-check
