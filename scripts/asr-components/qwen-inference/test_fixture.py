import copy
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
import hashlib
import io
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixture_paths import temporary_root

spec = importlib.util.spec_from_file_location('qwen_smoke_draft', Path(__file__).with_name('fixture.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
CATALOG = json.loads((Path(__file__).resolve().parents[3] / 'src-tauri/resources/asr/catalog.json').read_text())

class Gates(unittest.TestCase):
    def test_redirect_hosts_remain_official_and_https(self):
        for url in ['https://huggingface.co/file', 'https://cdn-lfs.hf.co/file', 'https://cas-bridge.xethub.hf.co/signed?x=1']:
            self.assertTrue(m.allowed_download_url(url, True))
        for url in ['http://huggingface.co/file', 'https://huggingface.co.attacker.test/file',
                    'https://attacker.test/file', 'https://user:secret@huggingface.co/file',
                    'https://huggingface.co:8443/file', 'https://raw.githubusercontent.com/file']:
            self.assertFalse(m.allowed_download_url(url, True))
        self.assertTrue(m.allowed_download_url('https://raw.githubusercontent.com/file', False))
        self.assertFalse(m.allowed_download_url('https://huggingface.co/file', False))
        with self.assertRaises(ValueError):
            m.FixtureRedirects(True).redirect_request(None, None, 302, '', {}, 'http://huggingface.co/file')

    def test_download_enforces_exact_bytes_and_digest(self):
        payload = b'fixture bytes'
        class Response(io.BytesIO):
            status = 200
        for size, digest, expected in [(len(payload), hashlib.sha256(payload).hexdigest(), True),
                                       (1, hashlib.sha256(payload).hexdigest(), False),
                                       (len(payload), 'a'*64, False)]:
            with temporary_root() as tmp:
                item = {'url': 'https://huggingface.co/Qwen/reviewed', 'bytes': size, 'sha256': digest}
                opener = SimpleNamespace(open=lambda *a, **kw: Response(payload))
                with patch.object(m.urllib.request, 'build_opener', return_value=opener):
                    if expected:
                        m.download(item, tmp/'file', m.time.monotonic()+30)
                        self.assertEqual((tmp/'file').read_bytes(), payload)
                    else:
                        with self.assertRaises(ValueError): m.download(item, tmp/'file', m.time.monotonic()+30)

    def test_download_child_has_a_hard_deadline(self):
        with patch.object(m.subprocess, 'run', return_value=SimpleNamespace(stdout=json.dumps({'downloaded_files':19,'model_bytes':m.MODEL_BYTES}))) as run:
            m.download_child(Path('/owned/request.json'), {}, '/owned')
            self.assertEqual(run.call_args.kwargs['timeout'], 600)
            self.assertTrue(run.call_args.kwargs['check'])
            self.assertIn('-I', run.call_args.args[0])

    def test_failed_inference_preserves_measurement_and_removes_cache(self):
        measurement={'available_ram_bytes':13*m.GIB,'total_ram_bytes':16*m.GIB,'free_disk_bytes':20*m.GIB}
        audio=json.loads((Path(__file__).resolve().parents[1]/'fixtures.json').read_text())['audio']
        report={}
        with temporary_root() as tmp, patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=json.dumps(measurement))), patch.object(m,'download_child'):
            with self.assertRaises(RuntimeError):
                m.run('private','worker',{},tmp,CATALOG,audio,lambda *a,**kw: (_ for _ in ()).throw(RuntimeError('fixture failure')), report=report)
            self.assertEqual(report['resources_before_download'], measurement)
            self.assertTrue(report['fixture_cache_removed'])
            self.assertEqual(list(tmp.iterdir()), [])

    def test_owned_worker_output_is_capped_and_timeout_is_reaped(self):
        real_popen=m.subprocess.Popen
        for code, timeout, limit, error in [("print('x'*2048)", 5, 128, ValueError),
                                             ('import time;time.sleep(60)', 0.1, 1024, TimeoutError)]:
            with temporary_root() as tmp:
                worker=tmp/'worker.py';worker.write_text(code,encoding='utf-8')
                children=[]
                def launch(*args,**kwargs):
                    child=real_popen(*args,**kwargs);children.append(child);return child
                with patch.object(m.subprocess,'Popen',side_effect=launch), self.assertRaises(error):
                    m.bounded_worker_requests(sys.executable,worker,[],None,tmp,timeout=timeout,max_output_bytes=limit)
                self.assertEqual(len(children),1)
                self.assertIsNotNone(children[0].poll())

    def test_owned_worker_preserves_utf8_protocol(self):
        with temporary_root() as tmp:
            worker=tmp/'worker.py'
            worker.write_text("import json;print(json.dumps({'text':'日本語'},ensure_ascii=False))",encoding='utf-8')
            self.assertEqual(m.bounded_worker_requests(sys.executable,worker,[],None,tmp), [{'text':'日本語'}])

    def test_exact_thresholds(self):
        for ram,disk,result in [(13*m.GIB,12*m.GIB,True),(13*m.GIB-1,12*m.GIB,False),(13*m.GIB,12*m.GIB-1,False)]:
            self.assertIs(m.resource_gate({'available_ram_bytes':ram,'total_ram_bytes':16*m.GIB,'free_disk_bytes':disk})[0],result)
    def test_invalid_measurements(self):
        for item in [{},{'available_ram_bytes':True,'total_ram_bytes':16*m.GIB,'free_disk_bytes':20*m.GIB},{'available_ram_bytes':20*m.GIB,'total_ram_bytes':16*m.GIB,'free_disk_bytes':20*m.GIB}]:
            self.assertFalse(m.resource_gate(item)[0])
    def test_under_threshold_never_downloads(self):
        measurement={'available_ram_bytes':7*m.GIB,'total_ram_bytes':8*m.GIB,'free_disk_bytes':50*m.GIB}
        with patch.object(m.subprocess,'run',return_value=SimpleNamespace(stdout=json.dumps(measurement))), patch.object(m,'download',side_effect=AssertionError('must not fetch')) as download:
            result=m.run('private-python','worker',{},'/tmp',None,None,None)
            self.assertFalse(result['tested']);self.assertFalse(result['weights_downloaded']);download.assert_not_called()
    def test_fixed_pair_only(self):
        self.assertEqual(sum(f['bytes'] for x in m.reviewed_models(CATALOG) for f in x['files']),m.MODEL_BYTES)
        for field,value in [('version','moving-main'),('id','qwen3-asr-1-7b')]:
            catalog=copy.deepcopy(CATALOG);model=next(x for x in catalog['models'] if x['id']=='qwen3-asr-0-6b');model[field]=value
            with self.assertRaises(ValueError):m.reviewed_models(catalog)
    def test_bad_layout_source_hash_and_size(self):
        for field,value in [('path','../escape'),('url','https://unapproved.example/weights'),('sha256','bad'),('bytes',1)]:
            catalog=copy.deepcopy(CATALOG);model=next(x for x in catalog['models'] if x['id']=='qwen3-asr-0-6b');model['files'][0][field]=value
            with self.assertRaises(ValueError):m.reviewed_models(catalog)

if __name__=='__main__':unittest.main()
