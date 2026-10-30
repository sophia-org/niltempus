"""Consumer format fixtures; these are not native screenshots."""
import importlib.util
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zlib

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('fixture',ROOT/'test/gpu_pixels.py')
f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f)

def chunk(kind,data):
    return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))

def paeth(a,b,c):
    p=a+b-c; d=[abs(p-a),abs(p-b),abs(p-c)]
    return (a,b,c)[d.index(min(d))]

def png(pattern=1,channels=4,filter_id=0,extra=b'',header=None):
    image=f.image(pattern)[len(f.p.HEADER):]
    if channels==3: image=b''.join(image[i:i+3] for i in range(0,len(image),4))
    stride=1920*channels; previous=bytes(stride); encoded=bytearray()
    for pos in range(0,len(image),stride):
        row=image[pos:pos+stride]; encoded.append(filter_id)
        for i,value in enumerate(row):
            left=row[i-channels] if i>=channels else 0
            up=previous[i]; corner=previous[i-channels] if i>=channels else 0
            prediction=(0,left,up,(left+up)//2,paeth(left,up,corner))[filter_id]
            encoded.append((value-prediction)&255)
        previous=row
    header=header or struct.pack('>IIBBBBB',1920,1080,8,6 if channels==4 else 2,0,0,0)
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',header)+extra+chunk(b'IDAT',zlib.compress(encoded))+chunk(b'IEND',b'')

class Normalize(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.valid=png()
    def decode(self,data):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'capture.png';path.write_bytes(data)
            return subprocess.run([str(ROOT/'build/gpu-normalize-png'),str(path)],capture_output=True,timeout=5)
    def test_rgba_and_rgb_filters_are_exact_without_gamma_transform(self):
        for channels,filter_id,pattern in [(4,0,1),(3,1,2),(4,2,2),(3,3,1),(4,4,1)]:
            with self.subTest(channels=channels,filter=filter_id):
                data=self.valid if (channels,filter_id,pattern)==(4,0,1) else png(pattern,channels,filter_id,chunk(b'gAMA',struct.pack('>I',100000)))
                result=self.decode(data)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(result.stdout,f.image(pattern))
                f.p.check(result.stdout,pattern)
                with self.assertRaises(ValueError): f.p.check(result.stdout,3-pattern)
    def test_bad_png_never_emits_accepted_pam(self):
        bad_crc=bytearray(self.valid);bad_crc[-5]^=1
        for data in (bytes(bad_crc),self.valid[:-12],self.valid+b'junk',
                     self.valid[:33]+chunk(b'acTL',struct.pack('>II',2,0))+self.valid[33:],
                     b'not png',
                     self.valid[:8]+chunk(b'IHDR',struct.pack('>IIBBBBB',1921,1080,8,6,0,0,0))+self.valid[33:],
                     self.valid[:8]+chunk(b'IHDR',struct.pack('>IIBBBBB',1920,1080,16,6,0,0,0))+self.valid[33:]):
            with self.subTest(size=len(data)):
                result=self.decode(data);self.assertNotEqual(result.returncode,0);self.assertEqual(result.stdout,b'')
    def test_x_masks_byte_orders_and_padding(self):
        expected=f.p.HEADER+bytes((0x12,0x56,0xab,255))*1920*1080
        for order in ('lsb','msb'):
            result=subprocess.run([str(ROOT/'build/gpu-x-pixels-test'),order,'ok'],capture_output=True,timeout=5)
            self.assertEqual(result.returncode,0,result.stderr);self.assertEqual(result.stdout,expected)
            result=subprocess.run([str(ROOT/'build/gpu-x-pixels-test'),order,'bad'],capture_output=True,timeout=5)
            self.assertEqual(result.returncode,2);self.assertEqual(result.stdout,b'')

    def test_rgba_alpha_is_preserved_and_oracle_refuses_transparency(self):
        length=struct.unpack('>I',self.valid[33:37])[0]
        raw=bytearray(zlib.decompress(self.valid[41:41+length]))
        raw[4]=0  # First unfiltered pixel's alpha, not a channel conversion.
        data=self.valid[:33]+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
        result=self.decode(data)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(result.stdout[len(f.p.HEADER)+3],0)
        with self.assertRaises(ValueError): f.p.check(result.stdout,1)
    def test_paths_refuse_symlink_fifo_and_missing(self):
        import os
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'real').write_bytes(self.valid);(root/'link').symlink_to(root/'real');os.mkfifo(root/'fifo')
            for name in ('link','fifo','missing'):
                result=subprocess.run([str(ROOT/'build/gpu-normalize-png'),str(root/name)],capture_output=True,timeout=2)
                self.assertEqual(result.returncode,2);self.assertEqual(result.stdout,b'')

if __name__=='__main__':unittest.main()
