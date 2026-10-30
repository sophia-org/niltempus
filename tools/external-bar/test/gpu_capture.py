"""Private capture-controller fixtures. No Session, GPU, display or native run."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('controller', ROOT / 'gpu/capture_sophia.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
r = c.r


def write(path, data):
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, 'wb') as file:
        file.write(data)


def image(pattern):
    left, right = (((255, 0, 0, 255), (0, 0, 255, 255)) if pattern == 1
                   else ((0, 255, 0, 255), (255, 255, 255, 255)))
    row = bytes(left) * 960 + bytes(right) * 960
    return c.pixels.HEADER + row * 24 + bytes((10, 20, 30, 255)) * 1920 * 1056


def phase(generation=1, status='presented'):
    values = dict(schema=1, status=status, output=7, candidate_generation=generation,
                  staging_bytes=0, resident_bytes=184320, retiring_bytes=0, backing_bytes=184320)
    if status == 'presented':
        values.update(presentation_epoch=generation * 10, connection_epoch=2, content_grant_epoch=3)
    return ('sophia_live_shell_content ' + ' '.join(f'{k}={v}' for k, v in values.items()) + '\n').encode()


class Fixture:
    def __init__(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.delivery = self.root / 'captures'
        self.delivery.mkdir(mode=0o700)
        self.evidence = self.root / 'evidence'
        self.evidence.mkdir(mode=0o700)
        self.directory = c.private_directory(str(self.delivery))
        exe = os.path.realpath(sys.executable)
        self.pin = dict(pid=os.getpid(), starttime=c.snapshot(os.getpid()), executable=exe,
                        sha256=hashlib.sha256(Path(exe).read_bytes()).hexdigest())
        self.session = c.Session(self.pin)
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.endpoint = self.root / 'launch.sock'
        self.listener.bind(str(self.endpoint))
        self.listener.listen(1)
        self.log = self.root / 'session.log'
        self.mapping = (b'sophia_live_native_head schema=2 status=ready output=7 head=9 '
                        b'connector=DP-1 connector_id=93 mode=1920x1080 '
                        b'refresh_millihz=120000 mirrored=false\n')
        self.prefix = (f'sophia_developer_launch schema=1 status=enabled admission=host-admin socket={self.endpoint}\n'.encode()
                       + self.mapping)
        write(self.log, self.prefix + phase(1, 'prepared') + phase())
        self.expected = dict(schema=1, session=self.pin, session_log=str(self.log),
                             delivery_directory=str(self.delivery), output_generation=1,
                             hold=dict(generation=1, pattern=1, connection_epoch=2,
                                       content_grant_epoch=3, presentation_epoch=10))

    def publish(self, ordinal=1, change=None, missing=(), refusal=False):
        generation = 40 + ordinal
        pam = image((1, 2, 1)[ordinal - 1])
        receipt = dict(schema=1, status='delivered', binding='launch_custody_not_per_message',
                       launch_generation=generation, transaction=1, connector=r.CONNECTOR,
                       namespace=6, peer_pid=100 + ordinal, peer_starttime=900 + ordinal,
                       exe_dev=self.session.identity[0], exe_ino=self.session.identity[1],
                       request=ordinal, output=7, output_generation=1,
                       presentation_generation=100 + ordinal, width=1920, height=1080,
                       pam_sha256=hashlib.sha256(pam).hexdigest())
        if change:
            receipt.update(change)
        for suffix, data in [('pam', pam), ('stderr', b''), ('json', json.dumps(receipt).encode())]:
            if suffix not in missing:
                path = self.delivery / f'capture-{generation}.{suffix}'
                temporary = path.with_name(path.name + '.partial') if suffix != 'stderr' else path
                write(temporary, data)
                if temporary != path:
                    temporary.rename(path)
        if refusal:
            write(self.delivery / f'capture-{generation}.refused', b'refused\n')
        return receipt

    def verify(self, generation=41):
        return r.verify(self.directory, generation, dict(self.expected, output=7),
                        self.session.identity, c.pixels.check)

    def run(self, launcher):
        with patch.object(c, 'launch', side_effect=launcher) as call:
            value = c.capture(self.expected, self.session, self.directory,
                              time.monotonic() + 2, self.evidence)
            self_call = call.call_args.args
            assert self_call[0] == [self.pin['executable'], 'launch', '--socket', str(self.endpoint),
                                   '--recipe', 'capture', '--', self.pin['executable'], 'capture',
                                   '7', '--diagnostic-stream']
            assert call.call_count == 1
            return value

    def close(self):
        self.listener.close()
        self.session.close()
        os.close(self.directory)
        self.temp.cleanup()


class Controller(unittest.TestCase):
    def setUp(self):
        self.f = Fixture()
        self.addCleanup(self.f.close)

    def test_same_namespace_pin_and_nonconnecting_listener_ownership(self):
        self.f.session.check()
        path, _ = c.launch_socket(self.f.log.read_bytes(), self.f.session)
        self.assertEqual(path, str(self.f.endpoint))
        self.f.listener.setblocking(False)
        with self.assertRaises(BlockingIOError):
            self.f.listener.accept()  # No credential-probe connection was made.
        for key, value in [('sha256', '0' * 64), ('starttime', self.f.pin['starttime'] + 1)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                c.Session(dict(self.f.pin, **{key: value}))
        real = c.namespace_ids
        with patch.object(c, 'namespace_ids', side_effect=lambda pid: {} if pid == 'self' else real(pid)):
            with self.assertRaisesRegex(ValueError, 'namespaces'):
                self.f.session.check()
        with patch.object(c.os, 'readlink', return_value='socket:[0]'):
            with self.assertRaisesRegex(ValueError, 'listener does not belong'):
                c.launch_socket(self.f.log.read_bytes(), self.f.session)

    def test_receipt_success_and_identity_mutations(self):
        original = self.f.publish()
        self.assertEqual(self.f.verify()['receipt'], original)
        path = self.f.delivery / 'capture-41.json'
        mutations = {'schema': True, 'status': 'pending', 'binding': 'peercred', 'launch_generation': 40,
                     'transaction': 2, 'connector': 'pci-0000:16:00.0/DP-1', 'namespace': 0,
                     'peer_pid': 0, 'peer_starttime': 0, 'exe_dev': self.f.session.identity[0] + 1,
                     'exe_ino': self.f.session.identity[1] + 1, 'request': 0, 'output': 8,
                     'output_generation': 2, 'presentation_generation': 0, 'width': 1919,
                     'height': 1079, 'pam_sha256': '0' * 64}
        for key, value in mutations.items():
            path.write_text(json.dumps(dict(original, **{key: value})))
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.f.verify()
        for key in original:
            changed = dict(original)
            del changed[key]
            path.write_text(json.dumps(changed))
            with self.subTest(missing=key), self.assertRaises(ValueError):
                self.f.verify()
        for text in [json.dumps(dict(original, extra=1)), json.dumps(original)[:-1] + ',"schema":1}']:
            path.write_text(text)
            with self.assertRaises(ValueError):
                self.f.verify()

    def test_missing_nonempty_unsafe_stderr_and_incomplete_publication_refuse(self):
        self.f.publish()
        stderr = self.f.delivery / 'capture-41.stderr'
        stderr.unlink()
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            self.f.verify()
        write(stderr, b'error')
        with self.assertRaises(ValueError):
            self.f.verify()
        stderr.write_bytes(b'')
        stderr.chmod(0o644)
        with self.assertRaises(ValueError):
            self.f.verify()
        stderr.chmod(0o600)
        stderr.unlink()
        stderr.symlink_to(self.f.log)
        with self.assertRaises(OSError):
            self.f.verify()
        stderr.unlink()
        write(stderr, b'')
        for name in ['capture-41.json.partial', 'capture-41.refused', 'foreign']:
            path = self.f.delivery / name
            write(path, b'')
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.f.verify()
            path.unlink()

    def test_pixels_hash_and_short_pam_refuse(self):
        receipt = self.f.publish()
        path = self.f.delivery / 'capture-41.pam'
        valid = path.read_bytes()
        for data in [valid[:-1], valid + b'X', image(2)]:
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                self.f.verify()
        receipt['pam_sha256'] = hashlib.sha256(image(2)).hexdigest()
        (self.f.delivery / 'capture-41.json').write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, 'pattern'):
            self.f.verify()

    def test_owner_hold_and_output_mapping_refuse_stale_or_unbound(self):
        log = self.f.log.read_bytes()
        self.assertEqual(r.output_mapping(log)['output'], '7')
        r.held(log, self.f.expected['hold'], 7)
        for extra in [phase(), phase(2, 'prepared'), phase(2),
                      b'sophia_live_shell_content schema=3 status=claims_revoked reason=closed\n']:
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                r.held(log + extra, self.f.expected['hold'], 7)
        for changed in [log.replace(b'candidate_generation=1', b'candidate_generation=01'),
                        log.replace(b'connection_epoch=2', b'connection_epoch=4'),
                        log.replace(b'presentation_epoch=10', b'presentation_epoch=11'),
                        log.replace(b'status=presented', b'status=presented extra=1')]:
            with self.assertRaises(ValueError):
                r.held(changed, self.f.expected['hold'], 7)
        for changed in [log.replace(b'connector=DP-1', b'connector=DP-2'), log + self.f.mapping,
                        log.replace(b'120000', b'119880'), log.replace(b'mirrored=false', b'mirrored=true'),
                        log.replace(b'output=7 head', b'output=07 head'), log.replace(b'connector_id=93', b'connector_id=93 extra=1')]:
            with self.assertRaises(ValueError):
                r.output_mapping(changed)

    def test_success_uses_terminal_files_not_started_and_no_counter_equality(self):
        def launch(*args):
            self.f.publish()
            return {'version': 1, 'request': 1, 'status': 'started', 'process': 555}
        result = self.f.run(launch)
        self.assertEqual(result['status'], 'capture_verified')
        self.assertEqual(result['launch']['process'], 555)
        self.assertNotEqual(result['delivery']['receipt']['peer_pid'], 555)
        self.assertNotEqual(result['delivery']['receipt']['presentation_generation'], self.f.expected['hold']['presentation_epoch'])
        with self.assertRaisesRegex(ValueError, 'attempt sequence'):
            self.f.run(launch)

    def test_inotify_completion_without_sleep_or_busy_poll(self):
        release = threading.Event()
        errors = []
        def writer():
            if not release.wait(2):
                errors.append('no wait reached')
                return
            try:
                self.f.publish()
            except BaseException as error:
                errors.append(error)
        thread = threading.Thread(target=writer)
        thread.start()
        real = c.select.select
        def select(read, write, exceptional, timeout):
            if len(read) == 2:
                release.set()
            return real(read, write, exceptional, timeout)
        try:
            with patch.object(c.select, 'select', side_effect=select):
                result = self.f.run(lambda *_: {'version': 1, 'request': 1, 'status': 'started', 'process': 555})
            self.assertEqual(result['status'], 'capture_verified')
        finally:
            release.set()
            thread.join(2)
        self.assertEqual(errors, [])
        self.assertFalse(thread.is_alive())

    def test_started_without_terminal_or_failed_attempt_ends_run(self):
        for refused in [False, True]:
            def launch(*_):
                if refused:
                    write(self.f.delivery / 'capture-41.refused', b'failure')
                return {'version': 1, 'request': 1, 'status': 'started', 'process': 555}
            with patch.object(c, 'launch', side_effect=launch) as call:
                with self.assertRaises(ValueError):
                    c.capture(self.f.expected, self.f.session, self.f.directory,
                              time.monotonic() + 0.03, self.f.evidence)
                self.assertEqual(call.call_count, 1)

    def test_hold_changes_during_capture_and_ambiguous_receipts_refuse(self):
        def advance(*_):
            self.f.publish()
            with self.f.log.open('ab') as file:
                file.write(phase(2, 'prepared'))
            return {'version': 1, 'request': 1, 'status': 'started', 'process': 555}
        with self.assertRaisesRegex(ValueError, 'held candidate'):
            self.f.run(advance)
        # A second unrelated fresh receipt is never guessed away.
        for path in self.f.delivery.iterdir():
            path.unlink()
        self.f.log.write_bytes(self.f.prefix + phase(1, 'prepared') + phase())
        def ambiguous(*_):
            self.f.publish()
            self.f.publish(2)
            return {'version': 1, 'request': 1, 'status': 'started', 'process': 555}
        with self.assertRaisesRegex(ValueError, 'ambiguous'):
            self.f.run(ambiguous)

    def test_three_captures_increase_serials_and_keep_incarnation(self):
        previous = None
        for ordinal in (1, 2, 3):
            self.f.expected['hold'].update(generation=ordinal, pattern=(1, 2, 1)[ordinal - 1],
                                           presentation_epoch=ordinal * 10)
            with self.f.log.open('ab') as file:
                if ordinal > 1:
                    file.write(phase(ordinal, 'prepared') + phase(ordinal))
            result = self.f.run(lambda *_, n=ordinal: (self.f.publish(n), {'status': 'started'})[1])
            current = result['delivery']['receipt']
            r.advance(previous, current)
            if previous:
                for key in ('request', 'presentation_generation', 'launch_generation'):
                    with self.subTest(key=key), self.assertRaises(ValueError):
                        r.advance(previous, dict(current, **{key: previous[key]}))
                for key in ('namespace', 'output', 'output_generation', 'exe_dev', 'exe_ino'):
                    with self.subTest(key=key), self.assertRaises(ValueError):
                        r.advance(previous, dict(current, **{key: current[key] + 1}))
            previous = current

    def test_actual_launch_client_bounds_exit_and_preserved_streams(self):
        good = "print('{\"version\":1,\"request\":1,\"status\":\"started\",\"process\":123}')"
        for index, code in enumerate([good, "print('bad')", "print('x'*10000)",
                                      "import sys; sys.stderr.write('error')", "import signal; signal.pause()"]):
            evidence = self.f.root / f'client-{index}'
            evidence.mkdir(mode=0o700)
            argv = [sys.executable, '-I', '-B', '-c', code]
            if index == 0:
                result = c.launch(argv, self.f.session, time.monotonic() + 2, evidence)
                self.assertEqual(result['process'], 123)
            else:
                with self.assertRaises(ValueError):
                    c.launch(argv, self.f.session, time.monotonic() + 0.2, evidence)
            self.assertLessEqual((evidence / 'launch.stdout').stat().st_size, 4097)
            self.assertLessEqual((evidence / 'launch.stderr').stat().st_size, 4097)

    def test_cli_records_refusal_and_never_reuses_evidence(self):
        expected = self.f.root / 'expected.json'
        changed = copy.deepcopy(self.f.expected)
        changed['schema'] = True
        write(expected, json.dumps(changed).encode())
        evidence = self.f.root / 'cli-evidence'
        argv = [sys.executable, '-I', '-B', str(ROOT / 'gpu/capture_sophia.py'),
                '--expected', str(expected), '--evidence', str(evidence)]
        first = subprocess.run(argv, capture_output=True, timeout=3)
        self.assertEqual(first.returncode, 1)
        refusal = (evidence / 'refused.json').read_bytes()
        self.assertEqual(json.loads(refusal)['retry'], False)
        self.assertEqual(evidence.stat().st_mode & 0o777, 0o700)
        self.assertEqual((evidence / 'refused.json').stat().st_mode & 0o777, 0o600)
        second = subprocess.run(argv, capture_output=True, timeout=3)
        self.assertNotEqual(second.returncode, 0)
        self.assertEqual((evidence / 'refused.json').read_bytes(), refusal)
        self.listener_still_uncontacted()

    def test_real_head_format_partial_tail_and_accepted_socket(self):
        # Saved desktop-matched-cpu-02/sophia1 row; nominal, not measured cadence.
        real = (b'sophia_live_native_head schema=2 status=ready output=1 head=1 '
                b'connector=DP-1 connector_id=94 mode=1920x1080 '
                b'refresh_millihz=120000 mirrored=false\n')
        self.assertEqual(r.output_mapping(real)['output'], '1')
        before = self.f.log.read_bytes()
        with self.f.log.open('ab') as file:
            file.write(b'sophia_live_shell_content schema=')
        prefix, _ = c.log_bytes(str(self.f.log))
        self.assertEqual(prefix, before)
        with self.f.log.open('ab') as file:
            file.write(b'3 status=claims_revoked reason=closed\n')
        after, _ = c.log_bytes(str(self.f.log))
        self.assertTrue(after.startswith(prefix))
        with self.assertRaises(ValueError):
            r.held(after, self.f.expected['hold'], 7)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.connect(str(self.f.endpoint))
            accepted, _ = self.f.listener.accept()
            with accepted:
                self.assertEqual(c.launch_socket(before, self.f.session)[0], str(self.f.endpoint))

    def listener_still_uncontacted(self):
        self.f.listener.setblocking(False)
        with self.assertRaises(BlockingIOError):
            self.f.listener.accept()

    def test_preflight_failure_never_launches(self):
        for change in ('wrong-output', 'prior-partial'):
            with self.subTest(change=change), patch.object(c, 'launch') as launch:
                if change == 'wrong-output':
                    self.f.log.write_bytes((self.f.prefix + phase()).replace(b'connector=DP-1', b'connector=DP-2'))
                else:
                    self.f.log.write_bytes(self.f.prefix + phase())
                    write(self.f.delivery / 'capture-41.pam.partial', b'')
                with self.assertRaises(ValueError):
                    c.capture(self.f.expected, self.f.session, self.f.directory,
                              time.monotonic() + 1, self.f.evidence)
                launch.assert_not_called()
        self.listener_still_uncontacted()


if __name__ == '__main__':
    unittest.main()
