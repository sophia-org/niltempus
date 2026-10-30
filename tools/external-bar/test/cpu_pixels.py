import hashlib
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cpu_pixels', Path(__file__).parents[1] / 'cpu/pixels.py')
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class Pixels(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Asymmetric channels and rows expose channel swaps and vertical flips.
        cls.strip = b''.join(bytes((10+y, x & 255, 201-y, 255)) for y in range(p.BAR_HEIGHT) for x in range(p.WIDTH))
        cls.bgra = b''.join(bytes((201-y, x & 255, 10+y, 255)) for y in range(p.BAR_HEIGHT) for x in range(p.WIDTH))
        cls.pam = p.HEADER + cls.strip + bytes(p.WIDTH * (p.HEIGHT-p.BAR_HEIGHT) * 4)
        cls.checksum = p.fnv64(cls.bgra)

    def test_known_fnv_vectors(self):
        self.assertEqual(p.fnv64(b''), 'cbf29ce484222325')
        self.assertEqual(p.fnv64(b'hello'), 'a430d84680aabd0b')

    def test_all_rows_and_channels_match(self):
        row = p.check(self.pam, self.checksum)
        self.assertEqual(row['checked_pixels'], 46080)
        self.assertEqual(row['bar_bgra_sha256'], hashlib.sha256(self.bgra).hexdigest())
        self.assertEqual(row['capture_sha256'], hashlib.sha256(self.pam).hexdigest())

    def test_format_length_and_checksum_refusals(self):
        for data in [self.pam[:-1], self.pam+b'\0', self.pam.replace(b'DEPTH 4', b'DEPTH 3', 1),
                     self.pam.replace(b'HEIGHT 1080', b'HEIGHT 1079', 1),
                     self.pam.replace(b'RGB_ALPHA', b'RGB      ', 1)]:
            with self.subTest(size=len(data)), self.assertRaisesRegex(ValueError, 'PAM'):
                p.check(data, self.checksum)
        with self.assertRaisesRegex(ValueError, 'differs from Lom'):
            p.check(self.pam, '0'*16)
        for checksum in [self.checksum.upper(), self.checksum+'0', 1]:
            with self.subTest(checksum=checksum), self.assertRaisesRegex(ValueError, 'canonical'):
                p.check(self.pam, checksum)

    def test_channel_row_alpha_and_last_pixel_refusals(self):
        wrong = bytearray(self.pam)
        wrong[len(p.HEADER):len(p.HEADER)+len(self.bgra)] = self.bgra
        flipped = b''.join(self.strip[y*p.WIDTH*4:(y+1)*p.WIDTH*4]
                           for y in reversed(range(p.BAR_HEIGHT)))
        alpha = bytearray(self.pam)
        alpha[len(p.HEADER)+3] = 254
        last = bytearray(self.pam)
        last[len(p.HEADER)+len(self.strip)-2] ^= 1
        grey = p.HEADER + bytes((71,71,71,255))*p.WIDTH*p.HEIGHT
        edge = bytearray(self.pam)
        edge[len(p.HEADER)+959*4] ^= 1
        mirrored = b''.join(self.strip[(y*p.WIDTH+x)*4:(y*p.WIDTH+x+1)*4]
                            for y in range(p.BAR_HEIGHT) for x in reversed(range(p.WIDTH)))
        cases = [(wrong, 'differs'),
                 (p.HEADER+flipped+self.pam[len(p.HEADER)+len(self.strip):], 'differs'),
                 (p.HEADER+mirrored+self.pam[len(p.HEADER)+len(self.strip):], 'differs'),
                 (alpha, 'opaque'), (last, 'differs'), (edge, 'differs'), (grey, 'red/blue')]
        for data, reason in cases:
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, reason):
                p.check(data, self.checksum)

    def test_pixels_outside_strip_do_not_change_match(self):
        data = bytearray(self.pam)
        data[-1] = 1
        self.assertEqual(p.check(data, self.checksum)['bar_bgra_sha256'], hashlib.sha256(self.bgra).hexdigest())


if __name__ == '__main__':
    unittest.main()
