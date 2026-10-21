CC ?= cc
CFLAGS ?= -O2 -Wall -Wextra -Werror
BUILD := build
COMMON := common/fill.c common/fill.h common/journal.h
WL_GEN := wl-panel/gen/wlr-layer-shell.c wl-panel/gen/presentation-time.c wl-panel/gen/xdg-shell.c
X_PKGS := x11 xext x11-xcb xcb xcb-present xcb-shm

all: $(BUILD)/wl-panel $(BUILD)/x-panel $(BUILD)/fill-parity

$(BUILD):
	mkdir -p $@

$(BUILD)/wl-panel: wl-panel/wl-panel.c $(COMMON) $(WL_GEN) | $(BUILD)
	$(CC) $(CFLAGS) -std=c11 -Icommon -o $@ $< common/fill.c $(WL_GEN) $$(pkg-config --cflags --libs wayland-client) -lm

$(BUILD)/x-panel: x-panel/x-panel.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c $$(pkg-config --cflags --libs $(X_PKGS))

$(BUILD)/fill-parity: test/fill_parity.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c

$(BUILD)/x-events: test/x_events.c x-panel/x-panel.c $(COMMON) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< common/fill.c $$(pkg-config --cflags --libs $(X_PKGS))

.PHONY: all
