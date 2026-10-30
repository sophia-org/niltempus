#include "gpu_bar_bridge.h"
#include <assert.h>
#include <string.h>

_Static_assert(sizeof(GpuBarFrame) == 64, "frame ABI");
_Static_assert(offsetof(GpuBarFrame, fd) == 56, "fd offset");
_Static_assert(sizeof(GpuBarDevice) == 28, "device ABI");
int main(void) {
    char error[64];
    memset(error, 42, sizeof error);
    GpuBarBridge *owner = NULL;
    assert(gpu_bar_bridge_open("/dev/null", "0000:03:00.0", &owner, error, sizeof error) == -1);
    assert(!owner && error[0] != '\0');
#ifdef DIAGNOSTIC_COLOUR_HOLD
    GpuBarFrame colour = {.fd=99,.token=99};
    assert(gpu_bar_bridge_render_colour(NULL,1,&colour,error,sizeof error)==-1);
    assert(colour.fd==-1 && colour.token==0);
#endif
    GpuBarFrame frame = {.fd = 99, .token = 99};
    assert(gpu_bar_bridge_render(owner, 1, &frame, error, sizeof error) == -1);
    assert(frame.fd == -1 && frame.token == 0);
    GpuBarDevice device = {.schema = 99};
    assert(gpu_bar_bridge_device(owner, &device, error, sizeof error) == -1);
    assert(device.schema == 0);
    assert(gpu_bar_bridge_release(owner, 1, error, sizeof error) == -1);
    assert(gpu_bar_bridge_close(&owner, error, sizeof error) == -1);
    assert(!owner);
    return 0;
}
