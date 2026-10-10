"""Offline failure diagnostics: no network, weights, or remote state changes."""
import copy
from email.message import Message
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import socket
import ssl
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root

spec = importlib.util.spec_from_file_location('qwen_download_diagnostics', Path(__file__).with_name('fixture.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
ROOT = Path(__file__).resolve().parents[3]
CATALOG = json.loads((ROOT / 'src-tauri/resources/asr/catalog.json').read_text(encoding='utf-8'))
MODELS = m.reviewed_models(CATALOG)
AUDIO = json.loads((ROOT / 'scripts/asr-components/fixtures.json').read_text(encoding='utf-8'))['audio']
ENTRIES = m.fixture_files(MODELS, AUDIO)
SECRET = 'never-emit-this-secret'
SIGNED_URL = 'https://cas-bridge.xethub.hf.co/blob?X-Amz-Signature=' + SECRET


def request_file(root):
    path = root / 'download-request.json'
    path.write_text(json.dumps({'root': str(root), 'models': MODELS, 'audio': AUDIO}), encoding='utf-8')
    return path


def failure_record(index=0, received=0):
    return {'schema': 1, 'category': 'network-error', 'file': ENTRIES[index][2],
            'bytes_received': received, 'http_status': None, 'files_completed': index,
            'completed_bytes': sum(e[0]['bytes'] for e in ENTRIES[:index]),
            'weights_complete': index == 18, 'transfer_state': 'partial' if index or received else 'not-observed'}


def error_line(record):
    return m.DOWNLOAD_ERROR_PREFIX + json.dumps(record, ensure_ascii=True) + '\n'


def completed_child(stdout=b'', stderr=b'', returncode=1):
    """An already-exited child, including output produced before the first poll."""
    def launch(command, **kwargs):
        for field, value in [('stdout', stdout), ('stderr', stderr)]:
            kwargs[field].write(value if isinstance(value, bytes) else value.encode('utf-8'))
        return SimpleNamespace(returncode=returncode, poll=lambda: returncode, wait=lambda **kw: returncode, kill=lambda: None)
    return launch


class Response(io.BytesIO):
    status = 200


class Diagnostics(unittest.TestCase):
    def assert_private(self, value):
        serialized = json.dumps(value)
        for forbidden in (SECRET, SIGNED_URL, 'X-Amz', 'Authorization', 'Bearer', 'PROXY_PASSWORD'):
            self.assertNotIn(forbidden, serialized)

    def test_http_error_retains_pinned_identity_without_response_data(self):
        item, _, identity = ENTRIES[0]
        error = m.urllib.error.HTTPError(SIGNED_URL, 403, 'Authorization: Bearer ' + SECRET,
                                         {'Authorization': SECRET}, None)
        with temporary_root() as root, patch.object(m.urllib.request, 'build_opener', return_value=SimpleNamespace(open=lambda *a, **k: (_ for _ in ()).throw(error))):
            with self.assertRaises(m.FixtureDownloadError) as caught:
                m.download(item, root / 'file', m.time.monotonic() + 20, identity)
        record = caught.exception.evidence
        self.assertEqual(record['category'], 'http-error')
        self.assertEqual(record['http_status'], 403)
        self.assertEqual(record['file'], identity)
        self.assertEqual(record['file']['expected_sha256'], item['sha256'])
        self.assertEqual(record['file']['expected_bytes'], item['bytes'])
        self.assertEqual(record['bytes_received'], 0)
        self.assert_private(record)

    def test_known_and_unknown_error_categories_do_not_copy_messages(self):
        item, _, identity = ENTRIES[0]
        cases = [(m.urllib.error.URLError(socket.gaierror(-3, SECRET)), 'dns-error'),
                 (m.urllib.error.URLError(ssl.SSLError(SECRET)), 'tls-error'),
                 (m.urllib.error.URLError(SECRET), 'network-error'),
                 (TimeoutError(SECRET), 'download-timeout'),
                 (PermissionError(SECRET), 'filesystem-error'),
                 (m.DownloadCheckError(SIGNED_URL), 'unexpected-error'),
                 (RuntimeError('PROXY_PASSWORD=' + SECRET), 'unexpected-error')]
        for error, category in cases:
            with self.subTest(category=category), temporary_root() as root:
                opener = SimpleNamespace(open=lambda *a, **k: (_ for _ in ()).throw(error))
                with patch.object(m.urllib.request, 'build_opener', return_value=opener), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download(item, root / 'file', m.time.monotonic() + 20, identity)
                self.assertEqual(caught.exception.evidence['category'], category)
                self.assert_private(caught.exception.evidence)

    def test_status_size_hash_and_deadline_have_distinct_categories(self):
        payload = b'abc'
        cases = [(206, 3, hashlib.sha256(payload).hexdigest(), 'http-status'),
                 (200, 2, hashlib.sha256(payload).hexdigest(), 'size-mismatch'),
                 (200, 4, hashlib.sha256(payload).hexdigest(), 'size-mismatch'),
                 (200, 3, 'a'*64, 'hash-mismatch')]
        for status, size, digest, category in cases:
            with self.subTest(category=category, size=size), temporary_root() as root:
                item = {'url': ENTRIES[0][0]['url'], 'bytes': size, 'sha256': digest}
                response = Response(payload); response.status = status
                with patch.object(m.urllib.request, 'build_opener', return_value=SimpleNamespace(open=lambda *a, **k: response)), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download(item, root/'file', m.time.monotonic()+20)
                self.assertEqual(caught.exception.evidence['category'], category)
                self.assertEqual(caught.exception.evidence['http_status'], status)
        with temporary_root() as root, patch.object(m.urllib.request, 'build_opener') as opener, self.assertRaises(m.FixtureDownloadError) as caught:
            m.download(ENTRIES[0][0], root/'file', m.time.monotonic()-1)
        opener.assert_not_called()
        self.assertEqual(caught.exception.evidence['category'], 'download-timeout')

    def test_partial_read_is_not_reported_as_completed_weights(self):
        class Interrupted(Response):
            def read(self, *args):
                if self.tell(): raise m.urllib.error.URLError(SIGNED_URL)
                return super().read(3)
        with temporary_root() as root:
            opener = SimpleNamespace(open=lambda *a, **k: Interrupted(b'abc'))
            with patch.object(m.urllib.request, 'build_opener', return_value=opener), self.assertRaises(m.FixtureDownloadError) as caught:
                m.fetch_pair(MODELS, AUDIO, root)
        record = caught.exception.evidence
        self.assertEqual(record['bytes_received'], 3)
        self.assertEqual(record['files_completed'], 0)
        self.assertEqual(record['completed_bytes'], 0)
        self.assertEqual(record['transfer_state'], 'partial')
        self.assertIs(record['weights_complete'], False)
        self.assert_private(record)

    def test_completed_files_and_complete_weights_are_separate_from_audio(self):
        for failed_index in (1, 18):
            calls = []
            def download(item, destination, deadline, identity):
                calls.append((item, deadline))
                if len(calls)-1 == failed_index:
                    raise m.FixtureDownloadError({'schema': 1, 'category': 'http-error', 'file': identity,
                                                 'bytes_received': 0, 'http_status': 429})
            with self.subTest(index=failed_index), temporary_root() as root, patch.object(m, 'download', side_effect=download), self.assertRaises(m.FixtureDownloadError) as caught:
                m.fetch_pair(MODELS, AUDIO, root)
            record = caught.exception.evidence
            self.assertEqual(record['files_completed'], failed_index)
            self.assertEqual(record['completed_bytes'], sum(e[0]['bytes'] for e in ENTRIES[:failed_index]))
            self.assertIs(record['weights_complete'], failed_index == 18)
            self.assertEqual(record['transfer_state'], 'partial')
            self.assertEqual(len({deadline for _, deadline in calls}), 1)

    def test_child_emits_one_bounded_structured_error_without_traceback(self):
        error = m.urllib.error.HTTPError(SIGNED_URL, 503, SECRET, {'Authorization': SECRET}, None)
        with temporary_root() as root:
            path = request_file(root)
            with patch.object(m.urllib.request, 'build_opener', return_value=SimpleNamespace(open=lambda *a, **k: (_ for _ in ()).throw(error))), patch.object(m.sys, 'stderr', new_callable=io.StringIO) as stderr:
                self.assertEqual(m.download_main(path), 1)
            output = stderr.getvalue()
            self.assertEqual(len(output.splitlines()), 1)
            self.assertLessEqual(len(output.encode('utf-8')), m.MAX_DOWNLOAD_ERROR_BYTES)
            self.assertNotIn('Traceback', output)
            record = m.child_error(output, path)
            self.assertEqual(record['http_status'], 503)
            self.assert_private(record)

    def test_parent_propagates_verified_error_and_rejects_untrusted_fields(self):
        with temporary_root() as root:
            path = request_file(root)
            record = failure_record(1, 3)
            with patch.object(m.subprocess, 'Popen', side_effect=completed_child(stderr=error_line(record))), self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_child(path, {}, root)
            self.assertEqual(caught.exception.evidence, {**record, 'child_reap_confirmed': True})
            altered = []
            for field, value in [('file', {'path': SIGNED_URL}), ('files_completed', True),
                                 ('completed_bytes', -1), ('weights_complete', True),
                                 ('category', SECRET), ('Authorization', SECRET)]:
                changed = copy.deepcopy(record); changed[field] = value; altered.append(changed)
            changed = copy.deepcopy(record); changed['file']['expected_sha256'] = 'f'*64; altered.append(changed)
            for changed in altered:
                with patch.object(m.subprocess, 'Popen', side_effect=completed_child(stderr=error_line(changed))), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download_child(path, {}, root)
                self.assertEqual(caught.exception.evidence['transfer_state'], 'unknown')
                self.assert_private(caught.exception.evidence)

    def test_invalid_request_and_oversized_diagnostics_are_fixed_and_bounded(self):
        with temporary_root() as root, patch.object(m, 'fetch_pair', side_effect=AssertionError('no download')) as fetch:
            path = root/'missing.json'
            with patch.object(m.sys, 'stderr', new_callable=io.StringIO) as stderr:
                self.assertEqual(m.download_main(path), 1)
            record = m.child_error(stderr.getvalue(), path)
            self.assertEqual(record['category'], 'invalid-request')
            self.assertEqual(record['transfer_state'], 'unknown')
            fetch.assert_not_called()
            path = request_file(root)
            oversized = failure_record(); oversized['file'] = {'path': SECRET*1000}
            fetch.side_effect = m.FixtureDownloadError(oversized)
            with patch.object(m.sys, 'stderr', new_callable=io.StringIO) as stderr:
                self.assertEqual(m.download_main(path), 1)
            self.assertLessEqual(len(stderr.getvalue().encode('utf-8')), m.MAX_DOWNLOAD_ERROR_BYTES)
            record = m.child_error(stderr.getvalue(), path)
            self.assertEqual(record['category'], 'oversized-diagnostic')
            self.assert_private(record)
        with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=completed_child(stderr=error_line(oversized))), self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(Path('/unused'), {}, '.')
        self.assert_private(caught.exception.evidence)
        for schema in (True, 1.0):
            record = m.unconfirmed_child_error('invalid-request', None); record['schema'] = schema
            with self.assertRaises(ValueError): m.child_error(error_line(record), Path('/unused'))

    def test_unknown_startup_errors_omit_signed_urls_authorization_and_environment(self):
        stderr = ('Authorization: Bearer '+SECRET+'\nPROXY_PASSWORD='+SECRET+'\n'
                  'RuntimeError: 日本語 '+SIGNED_URL+'\n')
        for raw in (stderr, stderr.encode('utf-8'), b'\xff' + stderr.encode('utf-8')):
            with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=completed_child(stderr=raw)), self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_child(Path('/does-not-exist'), {}, root)
            record = caught.exception.evidence
            self.assertEqual(record['stderr_tail'], 'RuntimeError: [details redacted]')
            self.assertEqual(record['category'], 'child-startup-or-protocol-error')
            self.assertIsNone(record['weights_complete'])
            self.assert_private(record)

    def test_utf8_tail_never_splits_a_code_point_or_exceeds_byte_budget(self):
        value = 'prefix日本語🙂後'
        for limit in range(len(value.encode('utf-8')) + 3):
            for source in (value, value.encode('utf-8')):
                result = m.utf8_tail(source, limit)
                self.assertLessEqual(len(result.encode('utf-8')), limit)
                self.assertTrue(value.endswith(result))
                self.assertNotIn('\ufffd', result)

    def test_aggregate_timeout_preserves_hard_deadline_and_reaps_child(self):
        real_popen = subprocess.Popen
        children = []
        def launch(*args, **kwargs):
            self.assertIn('-I', args[0])
            child = real_popen([sys.executable, '-I', '-B', '-c', 'import time; time.sleep(60)'], **kwargs)
            children.append(child); return child
        self.assertEqual(m.DOWNLOAD_TIMEOUT_SECONDS, 600)
        with temporary_root() as root, patch.object(m, 'DOWNLOAD_TIMEOUT_SECONDS', 0.1), patch.object(m.subprocess, 'Popen', side_effect=launch), self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(request_file(root), None, root)
        self.assertEqual(len(children), 1)
        self.assertIsNotNone(children[0].poll())
        self.assertEqual(caught.exception.evidence['category'], 'aggregate-timeout')
        self.assertEqual(caught.exception.evidence['transfer_state'], 'unknown')
        self.assertTrue(caught.exception.evidence['child_reap_confirmed'])

    def test_failed_download_reaches_report_and_never_starts_inference(self):
        measurement = {'available_ram_bytes':13*m.GIB, 'total_ram_bytes':16*m.GIB, 'free_disk_bytes':20*m.GIB}
        for record in (failure_record(1, 3), failure_record(18), m.unconfirmed_child_error('aggregate-timeout', SECRET)):
            report = {}
            with temporary_root() as root, patch.object(m.subprocess, 'run', return_value=SimpleNamespace(stdout=json.dumps(measurement))), patch.object(m, 'download_child', side_effect=m.FixtureDownloadError(record)), patch.object(m, 'bounded_worker_requests', side_effect=AssertionError('must not infer')) as infer:
                with self.assertRaises(m.FixtureDownloadError):
                    m.run('private', 'worker', {}, root, CATALOG, AUDIO, infer, report=report)
                self.assertEqual(list(root.iterdir()), [])
            infer.assert_not_called()
            self.assertEqual(report['download_failure'], record)
            self.assertIs(report['weights_downloaded'], record['weights_complete'] is True)
            self.assertTrue(report['download_attempted'])
            self.assertTrue(report['fixture_cache_removed'])
            self.assertFalse(report['tested'])

    def test_success_protocol_errors_and_launch_failures_are_sanitized(self):
        for result in (SimpleNamespace(stdout=SECRET, stderr='Authorization: '+SECRET),
                       SimpleNamespace(stdout='x'*10000, stderr=SECRET)):
            with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=completed_child(result.stdout, result.stderr, 0)), self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_child(Path('/unused'), {}, root)
            self.assertIn(caught.exception.evidence['category'], ('child-protocol-error', 'child-output-limit'))
            self.assert_private(caught.exception.evidence)
        with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=OSError(SECRET)), self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(Path('/unused'), {}, root)
        self.assertEqual(caught.exception.evidence['category'], 'child-launch-error')
        self.assert_private(caught.exception.evidence)

    def test_completion_record_requires_exact_fields_integer_types_and_utf8(self):
        good = json.dumps({'downloaded_files': 19, 'model_bytes': m.MODEL_BYTES}).encode('utf-8')
        self.assertTrue(m.download_succeeded(good))
        for bad in [good+good, good+b'\xff', b'[]', b'null', b'x'*257,
                    good.replace(b'19', b'19.0'), good.replace(str(m.MODEL_BYTES).encode(), b'0'),
                    b'{"downloaded_files":0,"downloaded_files":19,"model_bytes":3720689099}',
                    b'{"downloaded_files":19,"model_bytes":3720689099,"extra":true}']:
            with self.subTest(bad=bad[:50]): self.assertFalse(m.download_succeeded(bad))
        with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=completed_child(good, b'', 0)) as popen:
            self.assertIsNone(m.download_child(root/'request.json', {}, root))
            popen.assert_called_once()

    def test_real_over_output_children_are_killed_reaped_and_streams_removed(self):
        real_popen = subprocess.Popen
        for stream in ('stdout', 'stderr'):
            children = []
            def launch(*args, **kwargs):
                code = 'import sys,time;sys.'+stream+'.write("x"*32768);sys.'+stream+'.flush();time.sleep(60)'
                child = real_popen([sys.executable, '-I', '-B', '-u', '-c', code], **kwargs)
                children.append(child); return child
            with self.subTest(stream=stream), temporary_root() as root:
                with patch.object(m.subprocess, 'Popen', side_effect=launch), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download_child(root/'request.json', None, root)
                self.assertEqual(list(root.iterdir()), [])
            self.assertEqual(len(children), 1)
            self.assertIsNotNone(children[0].poll())
            self.assertEqual(caught.exception.evidence['category'], 'child-output-limit')
        with temporary_root() as root, patch.object(m.subprocess, 'Popen', side_effect=completed_child(b'x'*(m.MAX_DOWNLOAD_OUTPUT_BYTES+1), b'', 0)), self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(root/'request.json', {}, root)
        self.assertEqual(caught.exception.evidence['category'], 'child-output-limit')

    def test_poll_or_kill_errors_still_attempt_wait_and_redact_error_text(self):
        for failure in ('poll', 'kill'):
            child = SimpleNamespace(returncode=None, poll=Mock(return_value=None),
                                    kill=Mock(), wait=Mock(return_value=0))
            getattr(child, failure).side_effect = OSError('Authorization: '+SECRET)
            with self.subTest(failure=failure), temporary_root() as root:
                with patch.object(m.subprocess, 'Popen', return_value=child), patch.object(m.time, 'monotonic', side_effect=[0, 601]), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download_child(root/'request.json', {}, root)
                self.assertEqual(list(root.iterdir()), [])
            child.wait.assert_called_once()
            child.kill.assert_called_once()
            self.assertEqual(child.wait.call_args.kwargs, {'timeout': m.DOWNLOAD_REAP_TIMEOUT_SECONDS})
            self.assertEqual(caught.exception.evidence['category'], 'child-process-error')
            self.assertTrue(caught.exception.evidence['child_reap_confirmed'])
            self.assert_private(caught.exception.evidence)

    def test_wait_timeout_or_error_never_claims_successful_reap_or_zero_transfer(self):
        for error in (subprocess.TimeoutExpired([SIGNED_URL], 5), OSError('Authorization: '+SECRET)):
            child = SimpleNamespace(returncode=None, poll=Mock(return_value=None),
                                    kill=Mock(side_effect=OSError(SECRET)), wait=Mock(side_effect=error))
            with self.subTest(error=type(error).__name__), temporary_root() as root:
                with patch.object(m.subprocess, 'Popen', return_value=child), patch.object(m.time, 'monotonic', side_effect=[0, 601]), self.assertRaises(m.FixtureDownloadError) as caught:
                    m.download_child(root/'request.json', {}, root)
            child.kill.assert_called_once()
            child.wait.assert_called_once_with(timeout=m.DOWNLOAD_REAP_TIMEOUT_SECONDS)
            record = caught.exception.evidence
            self.assertEqual(record['category'], 'child-reap-unconfirmed')
            self.assertFalse(record['child_reap_confirmed'])
            self.assertEqual(record['transfer_state'], 'unknown')
            self.assertIsNone(record['weights_complete'])
            self.assert_private(record)

    def test_official_origin_and_empty_proxy_handler_are_unchanged(self):
        payload = b'ok'
        item = {'url': ENTRIES[0][0]['url'], 'bytes': 2, 'sha256': hashlib.sha256(payload).hexdigest()}
        opener = SimpleNamespace(open=lambda *a, **k: Response(payload))
        with temporary_root() as root, patch.object(m.urllib.request, 'ProxyHandler', wraps=m.urllib.request.ProxyHandler) as proxy, patch.object(m.urllib.request, 'build_opener', return_value=opener) as build:
            m.download(item, root/'file', m.time.monotonic()+20)
        proxy.assert_called_once_with({})
        self.assertIsInstance(build.call_args.args[1], m.FixtureRedirects)


class ReadmeDiagnostics(unittest.TestCase):
    def response(self, payload=None, length=None, status=200, extra_headers=()):
        payload = b'x' * m.README_BYTES if payload is None else payload
        response = Response(payload); response.status = status; response.headers = Message()
        if length is not False: response.headers.add_header('Content-Length', str(m.README_BYTES if length is None else length))
        for key, value in extra_headers: response.headers.add_header(key, value)
        response.read = Mock(wraps=response.read)
        return response

    def test_selector_is_required_and_only_two_exact_choices_are_valid(self):
        self.assertEqual(m.request_selector({'selector':'full-inference'}), 'full-inference')
        good={'selector':'readme-diagnostic','diagnostic':dict(m.README_REQUEST)}
        self.assertEqual(m.request_selector(good), 'readme-diagnostic')
        for value in ({}, {'selector':None}, {'selector':True}, {'selector':'anything'},
                      {'selector':'readme-diagnostic'}, {'selector':'full-inference','diagnostic':{}}):
            with self.subTest(value=value), self.assertRaises(ValueError): m.request_selector(value)
        for key in m.README_REQUEST:
            bad=copy.deepcopy(good); bad['diagnostic'][key]=None
            with self.subTest(key=key), self.assertRaises(ValueError):m.request_selector(bad)
        good['diagnostic']['expected_bytes']=float(m.README_BYTES)
        with self.assertRaises(ValueError):m.request_selector(good)
        with patch.object(m.subprocess,'Popen') as launch, self.assertRaises(ValueError):
            m.download_child(Path('/unused'), {}, '.', selector='unknown')
        launch.assert_not_called()

    def test_readme_fetch_is_exact_origin_no_proxy_and_payload_read_cap(self):
        response=self.response(); opener=SimpleNamespace(open=Mock(return_value=response))
        digest=Mock();digest.hexdigest.return_value=m.README_SHA256
        with patch.object(m.urllib.request,'ProxyHandler',wraps=m.urllib.request.ProxyHandler) as proxy, \
             patch.object(m.urllib.request,'build_opener',return_value=opener) as build, \
             patch.object(m.hashlib,'sha256',return_value=digest):
            m.download_readme(m.time.monotonic()+m.README_TIMEOUT_SECONDS)
        proxy.assert_called_once_with({});self.assertIsInstance(build.call_args.args[1],m.FixtureRedirects)
        self.assertIs(build.call_args.args[1].huggingface,True)
        self.assertEqual(opener.open.call_args.args,(m.README_ITEM['url'],))
        self.assertLessEqual(opener.open.call_args.kwargs['timeout'],30)
        self.assertEqual(sum(len(c.args[0]) for c in digest.update.call_args_list),m.README_BYTES)
        self.assertEqual(sum(c.args[0] for c in response.read.call_args_list),m.README_BYTES)
        self.assertEqual(m.README_ITEM,MODELS[0]['files'][0])

    def test_actual_opener_never_reads_redirect_bodies_for_any_supported_status(self):
        import urllib.response
        for code in (301,302,303,307,308):
            requests=[]; bodies=[]; target='https://cdn-lfs.hf.co/pinned-readme'
            class Transport(m.urllib.request.HTTPSHandler):
                def https_open(self,request):
                    requests.append(request)
                    headers=Message()
                    if len(requests)==1:
                        headers['Location']=target;headers['Content-Length']='999999999'
                        body=io.BytesIO(b'');body.read=Mock(side_effect=AssertionError('redirect body must not be read'))
                        bodies.append(body);status=code
                    else:
                        headers['Content-Length']=str(m.README_BYTES)
                        body=io.BytesIO(b'x'*m.README_BYTES);status=200
                    response=urllib.response.addinfourl(body,headers,request.full_url,status)
                    response.msg='fixture';return response
            real_build=m.urllib.request.build_opener
            def build(*handlers):return real_build(*handlers,Transport())
            digest=Mock();digest.hexdigest.return_value=m.README_SHA256
            with self.subTest(code=code),patch.object(m.urllib.request,'build_opener',side_effect=build),patch.object(m.hashlib,'sha256',return_value=digest):
                m.download_readme(m.time.monotonic()+30)
            self.assertEqual([r.full_url for r in requests],[m.README_ITEM['url'],target])
            self.assertEqual(len(bodies),1);self.assertTrue(bodies[0].closed);bodies[0].read.assert_not_called()
            self.assertLessEqual(requests[1].timeout,requests[0].timeout)
            self.assertEqual(sum(len(c.args[0]) for c in digest.update.call_args_list),m.README_BYTES)

    def test_readme_redirect_opener_rejects_hosts_and_loops_without_reading_or_forwarding_secrets(self):
        import urllib.response
        for case in ('credentials','host','http','repeated-loop','distinct-loop','deadline','missing-location'):
            requests=[];bodies=[]
            class Transport(m.urllib.request.HTTPSHandler):
                def https_open(self,request):
                    requests.append(request);headers=Message()
                    target={'credentials':'https://cdn-lfs.hf.co/pinned-readme',
                            'host':'https://unreviewed.example/readme','http':'http://huggingface.co/readme',
                            'repeated-loop':m.README_ITEM['url'],'deadline':'https://cdn-lfs.hf.co/readme',
                            'distinct-loop':'https://huggingface.co/redirect-'+str(len(requests))}.get(case)
                    if target:headers['Location']=target
                    body=io.BytesIO(b'');body.read=Mock(side_effect=AssertionError('redirect body must not be read'))
                    bodies.append(body)
                    code=200 if case=='credentials' and len(requests)==2 else 302
                    response=urllib.response.addinfourl(body,headers,request.full_url,code);response.msg=SECRET
                    return response
            deadline=m.time.monotonic()+(-1 if case=='deadline' else 30)
            handler=m.ReadmeRedirects(deadline)
            opener=m.urllib.request.build_opener(m.urllib.request.ProxyHandler({}),handler,Transport())
            request=m.urllib.request.Request(m.README_ITEM['url'],headers={'Authorization':SECRET,'Proxy-Authorization':SECRET,'Cookie':SECRET,'Host':'wrong-authority','X-Private':SECRET})
            with self.subTest(case=case):
                if case=='credentials':
                    with opener.open(request,timeout=30):pass
                    headers={key.lower():value for key,value in requests[1].header_items()}
                    for key in ('authorization','proxy-authorization','cookie','x-private'):self.assertNotIn(key,headers)
                    self.assertEqual(headers['host'],'cdn-lfs.hf.co')
                else:
                    with self.assertRaises((m.DownloadCheckError,m.urllib.error.HTTPError,TimeoutError)):
                        opener.open(request,timeout=30)
                self.assertTrue(all(body.closed for body in bodies))
                for body in bodies:body.read.assert_not_called()
                expected={'credentials':2,'repeated-loop':3,'distinct-loop':9}.get(case,1)
                self.assertEqual(len(requests),expected)

    def test_readme_rejects_unknown_oversized_compressed_and_ambiguous_lengths_before_body(self):
        for length,extra in [(False,()),(m.README_BYTES+1,()),(m.README_BYTES-1,()),
                             (None,(('Transfer-Encoding','chunked'),)),(None,(('Content-Encoding','gzip'),)),
                             (None,(('Content-Length',str(m.README_BYTES)),))]:
            response=self.response(length=length,extra_headers=extra)
            with self.subTest(length=length,extra=extra), patch.object(m.urllib.request,'build_opener',return_value=SimpleNamespace(open=lambda *a,**k:response)), self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_readme(m.time.monotonic()+30)
            response.read.assert_not_called();self.assertEqual(caught.exception.evidence['category'],'size-mismatch')
            self.assertEqual(caught.exception.evidence['bytes_received'],0)

    def test_readme_short_hash_status_dns_and_http_failures_are_bounded_and_sanitized(self):
        cases=[(self.response(payload=b'abc'),'size-mismatch'),(self.response(),'hash-mismatch'),(self.response(status=206),'http-status')]
        for response,category in cases:
            with patch.object(m.urllib.request,'build_opener',return_value=SimpleNamespace(open=lambda *a,**k:response)),self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_readme(m.time.monotonic()+30)
            self.assertEqual(caught.exception.evidence['category'],category)
            self.assertEqual(caught.exception.evidence['file'],m.README_IDENTITY)
        for error,category,status in [(m.urllib.error.URLError(socket.gaierror(-3,SECRET)),'dns-error',None),
                                      (m.urllib.error.HTTPError(SIGNED_URL,403,SECRET,{'Authorization':SECRET},None),'http-error',403)]:
            with patch.object(m.urllib.request,'build_opener',return_value=SimpleNamespace(open=Mock(side_effect=error))),self.assertRaises(m.FixtureDownloadError) as caught:
                m.download_readme(m.time.monotonic()+30)
            record=caught.exception.evidence
            self.assertEqual(record['category'],category);self.assertEqual(record['http_status'],status)
            self.assertNotIn(SECRET,json.dumps(record));self.assertNotIn('https://',json.dumps(record))
        with patch.object(m.urllib.request,'build_opener') as opener,self.assertRaises(m.FixtureDownloadError):m.download_readme(m.time.monotonic()-1)
        opener.assert_not_called()

    def test_readme_entrypoint_never_accepts_file_overrides_or_calls_pair(self):
        with temporary_root() as root:
            path=root/'readme-request.json';path.write_text(json.dumps({'root':str(root)}),encoding='utf-8')
            with patch.object(m,'download_readme') as readme,patch.object(m,'fetch_pair',side_effect=AssertionError('no pair')) as pair, \
                 patch.object(m.sys,'stdout',new_callable=io.StringIO) as stdout:
                self.assertEqual(m.readme_download_main(path),0)
            readme.assert_called_once();pair.assert_not_called()
            self.assertTrue(m.readme_succeeded(stdout.getvalue().encode('utf-8')))
            self.assertLessEqual(readme.call_args.args[0]-m.time.monotonic(),30)
            path.write_text(json.dumps({'root':str(root),'file':MODELS[0]['files'][5]}),encoding='utf-8')
            with patch.object(m,'download_readme') as readme,patch.object(m.sys,'stderr',new_callable=io.StringIO) as stderr:
                self.assertEqual(m.readme_download_main(path),1)
            readme.assert_not_called();self.assertEqual(m.readme_child_error(stderr.getvalue())['category'],'invalid-request')

    def test_actual_readme_producer_accepts_only_exact_lf_or_crlf_records(self):
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=repr(newline)), temporary_root() as root:
                path=root/'readme-request.json'
                path.write_text(json.dumps({'root':str(root)}),encoding='utf-8')
                with io.BytesIO() as output, io.TextIOWrapper(output,encoding='cp1252',newline=newline) as stdout:
                    with patch.object(m,'download_readme') as readme, \
                         patch.object(m,'fetch_pair',side_effect=AssertionError('no pair')) as pair, \
                         patch.object(m.sys,'stdout',stdout):
                        self.assertEqual(m.readme_download_main(path),0)
                    readme.assert_called_once();pair.assert_not_called()
                    actual=output.getvalue()
                    body=json.dumps({'readme_verified':True,'metadata_bytes':m.README_BYTES}).encode('utf-8')
                    self.assertEqual(actual,body+newline.encode('ascii'))
                    self.assertTrue(m.readme_succeeded(actual))
                    self.assertFalse(m.download_succeeded(actual))
                    for bad in (body,body+b'\r',actual+actual,actual+b'\n',b' '+actual,
                                actual+b'extra',actual.replace(b'true',b'1'),
                                actual.replace(b'57456',b'57457'),
                                actual.replace(b'{',b'{"readme_verified": true, ')):
                        self.assertFalse(m.readme_succeeded(bad))

    def test_readme_child_protocol_cannot_be_mistaken_for_full_pair(self):
        good=(json.dumps({'readme_verified':True,'metadata_bytes':m.README_BYTES})+'\n').encode()
        full=json.dumps({'downloaded_files':19,'model_bytes':m.MODEL_BYTES}).encode()
        self.assertTrue(m.readme_succeeded(good));self.assertFalse(m.download_succeeded(good))
        for bad in (full,good+good,good.replace(b'true',b'1'),good.replace(b'57456',b'57457'),b'x'*9000):
            self.assertFalse(m.readme_succeeded(bad))
            with temporary_root() as root,patch.object(m.subprocess,'Popen',side_effect=completed_child(bad,b'',0)),self.assertRaises(m.FixtureDownloadError):
                m.download_child(root/'request.json',{},root,selector='readme-diagnostic')
        with temporary_root() as root,patch.object(m.subprocess,'Popen',side_effect=completed_child(good,b'',0)) as launch:
            m.download_child(root/'request.json',{},root,selector='readme-diagnostic')
        self.assertIn('--diagnose-readme',launch.call_args.args[0]);self.assertNotIn('--download',launch.call_args.args[0])

    def test_readme_child_has_30_second_total_budget_and_confirms_reap(self):
        self.assertEqual(m.README_TIMEOUT_SECONDS,30);self.assertEqual(m.DOWNLOAD_TIMEOUT_SECONDS,600)
        child=SimpleNamespace(returncode=None,poll=Mock(return_value=None),kill=Mock(),wait=Mock(return_value=0))
        with temporary_root() as root,patch.object(m.subprocess,'Popen',return_value=child),patch.object(m.time,'monotonic',side_effect=[0,31]),self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(root/'request.json',{},root,selector='readme-diagnostic')
        child.kill.assert_called_once();child.wait.assert_called_once_with(timeout=5)
        self.assertEqual(caught.exception.evidence['category'],'aggregate-timeout');self.assertTrue(caught.exception.evidence['child_reap_confirmed'])

    def test_readme_failure_records_reject_untrusted_fields_and_preserve_safe_categories(self):
        record={'schema':1,'category':'dns-error','file':dict(m.README_IDENTITY),'bytes_received':0,'http_status':None,'weights_complete':False,'transfer_state':'not-observed'}
        self.assertEqual(m.readme_child_error(error_line(record)),record)
        for key,value in [('file',{'path':SIGNED_URL}),('bytes_received',m.README_BYTES+1),('weights_complete',True),('http_status',True),('category',SECRET),('url',SIGNED_URL)]:
            bad={**record,key:value}
            with self.assertRaises(ValueError):m.readme_child_error(error_line(bad))
        with temporary_root() as root,patch.object(m.subprocess,'Popen',side_effect=completed_child(stderr=error_line(record))),self.assertRaises(m.FixtureDownloadError) as caught:
            m.download_child(root/'request.json',{},root,selector='readme-diagnostic')
        self.assertEqual(caught.exception.evidence,{**record,'child_reap_confirmed':True})

    def test_readme_success_never_calls_full_download_audio_or_inference_and_cleans(self):
        report={}
        with temporary_root() as root,patch.object(m,'download_child') as child,patch.object(m,'fetch_pair',side_effect=AssertionError('no pair')) as pair, \
             patch.object(m,'run',side_effect=AssertionError('no inference')) as infer:
            m.run_readme_diagnostic({},root,CATALOG,report)
            self.assertEqual(list(root.iterdir()),[])
        child.assert_called_once();self.assertEqual(child.call_args.kwargs['selector'],'readme-diagnostic')
        pair.assert_not_called();infer.assert_not_called()
        self.assertTrue(report['verified']);self.assertTrue(report['fixture_cache_removed'])
        for key in ('weights_downloaded','audio_downloaded','inference_tested','weight_cdn_tested','original_failure_cause_established','content_uploaded'):self.assertFalse(report[key])
        self.assertLessEqual(len(json.dumps(report).encode()),8192)

    def test_unconfirmed_child_retains_owned_request_directories_for_both_selectors(self):
        measurement={'available_ram_bytes':13*m.GIB,'total_ram_bytes':16*m.GIB,'free_disk_bytes':20*m.GIB}
        for readme in (False,True):
            for reaped in (False,True):
                record=m.unconfirmed_child_error('child-reap-unconfirmed',None);record['child_reap_confirmed']=reaped
                report={}
                with self.subTest(readme=readme,reaped=reaped),temporary_root() as root, \
                     patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=json.dumps(measurement))), \
                     patch.object(m,'download_child',side_effect=m.FixtureDownloadError(record)),self.assertRaises(m.FixtureDownloadError):
                    try:
                        if readme:m.run_readme_diagnostic({},root,CATALOG,report)
                        else:m.run('private','worker',{},root,CATALOG,AUDIO,Mock(side_effect=AssertionError('no inference')),report)
                    finally:
                        self.assertEqual(report['fixture_cache_removed'],reaped)
                        self.assertEqual(bool(list(root.iterdir())),not reaped)
                        if not reaped:self.assertIn('fixture_cleanup_blocked',report)


if __name__ == '__main__': unittest.main()
