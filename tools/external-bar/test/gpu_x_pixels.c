#include "../gpu/x_pixels.h"
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    if (argc != 3) return 3;
    XImage image = {0};
    image.width=1920; image.height=1080; image.format=ZPixmap;
    image.depth=24; image.bits_per_pixel=32; image.bitmap_pad=32;
    image.bitmap_unit=32; image.bytes_per_line=1920*4+128;
    image.byte_order=!strcmp(argv[1],"lsb")?LSBFirst:MSBFirst;
    image.bitmap_bit_order=image.byte_order;
    image.red_mask=0xff0000; image.green_mask=0xff00; image.blue_mask=0xff;
    image.data=malloc((size_t)image.bytes_per_line*1080);
    if (!image.data || !XInitImage(&image)) return 3;
    memset(image.data,0xee,(size_t)image.bytes_per_line*1080);
    for (int y=0;y<1080;y++) for (int x=0;x<1920;x++) {
        unsigned char *p=(unsigned char *)image.data+y*image.bytes_per_line+4*x;
        const unsigned char a[]={0xab,0x56,0x12,0xcc}, b[]={0xcc,0x12,0x56,0xab};
        memcpy(p,image.byte_order==LSBFirst?a:b,4);
    }
    if (!strcmp(argv[2],"bad")) image.red_mask=0xff;
    int result=x_pixels(&image,stdout)?0:2;
    free(image.data);
    return result;
}
