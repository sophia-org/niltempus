"""Synthetic journal controls; no display, renderer or device is used."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("gpu_receipts", ROOT / "gpu/qualify.py")
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)


def fixture(transport):
    wire, arm, source, clock, imported = q.TRANSPORTS[transport]
    header = (f"# header arm={arm} width=1920 height=24 buffers=2 "
              "fill=vello_shade_mod128_opaque_rgb frames=2 warmup=1 bulk=0 bulk_width=0 "
              "bulk_height=0 lifecycle=fresh workload=fresh tiles=0 interval_ms=0 damage=full "
              f"idle_sec=0 clock=CLOCK_MONOTONIC_ns presented_clock={clock}\n")
    producer = (f"# gpu_producer schema=1 transport={wire} producer=vello_wgpu pci=0000:03:00.0 "
                "node=/dev/dri/renderD128 format=XR24 modifier=0 planes=1 buffers=2 lifecycle=fresh\n")
    end = "# gpu_end schema=1 rendered=3 released=3\n"
    rows = f"1\t10\t20\t30\t-1\t{source}\n2\t40\t50\t60\t30\t{source}\n"
    footer = "# rusage utime_ns=1 stime_ns=2 minflt=0 majflt=0\n# ctxsw voluntary=0 involuntary=1\n"
    # Wayland writes its rows after drain; X writes them during presentation.
    if transport == "wayland":
        return ("# output name=DP-1 width=1920 height=1080 scale=1\n" + header + producer +
                f"# gpu_import {imported}\n" + end + rows + footer + "# pid 1234\n# mode frame_callback\n")
    return (header + producer + f"# gpu_import {imported}\n# pid 1234\n# attach dri3_pixmap_from_buffers\n" +
            rows + end + footer + "# presented_clock_check ok=3 violations=0\n")


class Receipts(unittest.TestCase):
    def check(self, text, transport):
        return q.qualify(text, transport, 2, 1)

    def test_both_journal_orders_and_scope(self):
        for mode in q.TRANSPORTS:
            result = self.check(fixture(mode), mode)
            self.assertEqual(result["status"], "receipts_valid")
            self.assertEqual(result["client_pid"], 1234)
            self.assertIn("no artifact/process", result["limits"])

    def test_each_receipt_required_exact_and_unambiguous(self):
        for mode in q.TRANSPORTS:
            text = fixture(mode)
            for line in text.splitlines():
                if not line.startswith("# gpu_"):
                    continue
                for change in ("", line + "\n" + line, "noise " + line, " " + line,
                               line + " extra=1", line + " schema=1", line + " ",
                               line.replace("schema=1", "schema=01")):
                    with self.subTest(mode=mode, line=line, change=change):
                        with self.assertRaises(ValueError):
                            self.check(text.replace(line + "\n", change + "\n"), mode)
            with self.assertRaises(ValueError):
                self.check(text + "# gpu_unknown schema=1\n", mode)

    def test_wrong_device_layout_counts_and_transport(self):
        for mode in q.TRANSPORTS:
            for before, after in (
                ("0000:03:00.0", "0000:16:00.0"), ("renderD128", "renderD129"),
                ("format=XR24", "format=AR24"), ("modifier=0", "modifier=1"),
                ("planes=1", "planes=2"), ("buffers=2", "buffers=3"),
                ("released=3", "released=2"), ("rendered=3", "rendered=4"),
                ("released=3", "released=3.0"), ("released=3", "released=True"),
                ("producer=vello_wgpu", "producer=cpu_shm"),
                ("sync=implicit", "sync=explicit"),
            ):
                with self.subTest(mode=mode, before=before, after=after):
                    with self.assertRaises(ValueError):
                        self.check(fixture(mode).replace(before, after), mode)
            with self.assertRaises(ValueError):
                self.check(fixture(mode), "x11" if mode == "wayland" else "wayland")

    def test_header_and_frame_refusals(self):
        for mode in q.TRANSPORTS:
            text = fixture(mode)
            header = next(line for line in text.splitlines() if line.startswith("# header "))
            changes = [text.rstrip("\n"), text.replace("\n", "\r\n"),
                       text.replace("\n", "\u2028", 1),
                       text + header + "\n", text.replace(header, ""),
                       text.replace("vello_shade_mod128_opaque_rgb", "shade_mod128_a255"),
                       text.replace("frames=2", "frames=02"), text.replace("width=1920", "width=1921"),
                       text.replace("lifecycle=fresh", "lifecycle=reuse"),
                       text.replace("1\t10", "2\t10"), text.replace("2\t40", "02\t40"),
                       text.replace("2\t40", "2\t18446744073709551616"),
                       text.replace("40\t50\t60", "40\t50\t-1"),
                       text.replace("40\t50\t60", "40\t50\t45"),
                       text.replace("40\t50\t60", "10\t20\t30"),
                       text.replace(q.TRANSPORTS[mode][2] + "\n", "fallback\n")]
            for changed in changes:
                with self.subTest(mode=mode, changed=changed):
                    with self.assertRaises(ValueError):
                        self.check(changed, mode)
        for change in ("", "# presented_clock_check ok=2 violations=0",
                       "# presented_clock_check ok=3 violations=1"):
            with self.assertRaises(ValueError):
                self.check(fixture("x11").replace("# presented_clock_check ok=3 violations=0", change), "x11")

    def test_metadata_and_each_adjacent_order_swap_refuse(self):
        for mode in q.TRANSPORTS:
            text = fixture(mode)
            for line in text.splitlines():
                if not line.startswith("# "):
                    continue
                for changed in (text.replace(line + "\n", ""), text + line + "\n"):
                    with self.subTest(mode=mode, line=line), self.assertRaises(ValueError):
                        self.check(changed, mode)
            lines = text.splitlines()
            for index in range(len(lines) - 1):
                changed = list(lines)
                changed[index:index + 2] = reversed(changed[index:index + 2])
                with self.subTest(mode=mode, index=index), self.assertRaises(ValueError):
                    self.check("\n".join(changed) + "\n", mode)
            for extra in ("# attach memfd_readonly_putimage", "# attach sysv_shm_pixmap",
                          "# mode idle_hold", "# mode absolute_timer", "# arbitrary value"):
                with self.assertRaises(ValueError):
                    self.check(text + extra + "\n", mode)
            for before, after in (("pid 1234", "pid 0"), ("pid 1234", "pid 01234"),
                                  ("utime_ns=1", "utime_ns=-1"), ("voluntary=0", "voluntary=0.0")):
                with self.assertRaises(ValueError):
                    self.check(text.replace(before, after), mode)
        with self.assertRaises(ValueError):
            self.check(fixture("wayland").replace("name=DP-1", "name=DP-2"), "wayland")

    def test_declared_workload_is_not_inferred(self):
        for args in (("cpu", 2, 1), ("wayland", True, 1), ("wayland", 2, 1.0),
                     ("wayland", 0, 1), ("wayland", 2, -1), ("wayland", 2, 2)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                q.qualify(fixture("wayland"), *args)

    def test_cli_reports_only_receipt_scope_and_refuses_without_output(self):
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp) / "journal.tsv"
            argv = [sys.executable, "-I", "-B", str(ROOT / "gpu/qualify.py"), str(journal),
                    "--transport", "wayland", "--frames", "2", "--warmup", "1"]
            journal.write_text(fixture("wayland"))
            result = subprocess.run(argv, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "receipts_valid")
            journal.write_text(fixture("wayland").replace("released=3", "released=2"))
            result = subprocess.run(argv, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertIn("GPU receipt refusal", result.stderr)


if __name__ == "__main__":
    unittest.main()
