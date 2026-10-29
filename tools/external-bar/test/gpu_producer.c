#define _GNU_SOURCE
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>
#include "producer.h"

int main(void) {
    GpuBarFrame valid={.token=1,.generation=1,.fd=3,.width=1920,.height=24,
                      .stride=7680,.format=GPU_XR24,.source_bytes=184320};
    assert(gpu_frame_valid(&valid,1));
    for (unsigned i=0;i<11;i++) {
        GpuBarFrame f=valid;
        if(i==0) f.token=0;
        if(i==1) f.generation=2;
        if(i==2) f.fd=-1;
        if(i==3) f.width=1919;
        if(i==4) f.height=25;
        if(i==5) f.format++;
        if(i==6) f.modifier=1;
        if(i==7) f.offset=(uint64_t)UINT32_MAX+1;
        if(i==8) f.stride=7679;
        if(i==9) f.source_bytes--;
        if(i==10) f.offset=1;
        assert(!gpu_frame_valid(&f,1));
    }
    valid.offset=64;valid.stride=8192;valid.source_bytes=64+23*8192+7680;
    assert(gpu_frame_valid(&valid,1));
    char root[]="/tmp/gpu-sysfs-XXXXXX";assert(mkdtemp(root));
    const char *dirs[]={"/dev","/dev/char","/dev/char/226:0","/dev/char/226:128",
                       "/dev/char/226:129","/0000:03:00.0","/0000:16:00.0"};
    char p[256],target[256];
    for(unsigned i=0;i<sizeof dirs/sizeof *dirs;i++) {
        snprintf(p,sizeof p,"%s%s",root,dirs[i]);assert(!mkdir(p,0700));
    }
    unsigned minors[]={0,128,129};
    for(unsigned i=0;i<3;i++) {
        snprintf(p,sizeof p,"%s/dev/char/226:%u/device",root,minors[i]);
        snprintf(target,sizeof target,"%s/%s",root,i==2?"0000:16:00.0":GPU_PCI);
        assert(!symlink(target,p));
    }
    assert(gpu_device_matches_at(root,makedev(226,0)));
    assert(gpu_device_matches_at(root,makedev(226,128)));
    assert(!gpu_device_matches_at(root,makedev(226,129)));
    assert(!gpu_device_matches_at(root,makedev(226,130)));
    for(unsigned i=0;i<3;i++) {
        snprintf(p,sizeof p,"%s/dev/char/226:%u/device",root,minors[i]);assert(!unlink(p));
    }
    for(size_t i=sizeof dirs/sizeof *dirs;i>0;i--) {
        snprintf(p,sizeof p,"%s%s",root,dirs[i-1]);assert(!rmdir(p));
    }
    assert(!rmdir(root));
    puts("GPU producer layout/physical-device controls passed; no GPU");
}
