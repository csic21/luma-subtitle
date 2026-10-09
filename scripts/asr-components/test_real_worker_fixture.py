import copy
import hashlib
import json
from pathlib import Path
import unittest
from unittest.mock import patch
import wave
from fixture_paths import temporary_root
import real_worker_fixture as f
import prove_offline_pip as proof


def pin(data): return {'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}


class RealWorkerFixtureTests(unittest.TestCase):
    def test_original_reference_and_other_engines_cannot_enable_hook(self):
        self.assertFalse(f.final_cpu_recipe({'id':'faster-whisper-cpu-windows-x64'}))
        self.assertFalse(f.final_cpu_recipe({'id':'qwen3-asr-cpu-windows-x64','recipe':{}}))
        wheel={'name':'ctranslate2','version':'4.8.2',
               'filename':'ctranslate2-4.8.2-cp312-cp312-win_amd64.whl',
               'url':'https://files.pythonhosted.org/upstream-reference.whl'}
        self.assertFalse(f.final_cpu_recipe({'id':'faster-whisper-cpu-windows-x64','recipe':{'wheels':[wheel]}}))

    def test_final_gate_requires_exact_origin_reviewed_hash_and_crt(self):
        wheel={'name':'ctranslate2','version':'4.8.2','filename':f.CPU_FILENAME,'url':f.CPU_URL,'bytes':123,'sha256':'a'*64}
        candidate={'id':'faster-whisper-cpu-windows-x64','unavailable_reason':'review pending',
                   'recipe':{'windows_crt':'msvc-14.44.35211-x64','wheels':[wheel]}}
        with temporary_root() as root, patch.object(f,'ROOT',root):
            (root/'locks').mkdir();(root/'locks/faster-whisper-cpu-windows-x64.json').write_text(json.dumps({'wheels':[wheel]}))
            self.assertTrue(f.final_cpu_recipe(candidate))
            for field,value in [('sha256','b'*64),('bytes',124)]:
                bad=copy.deepcopy(candidate);bad['recipe']['wheels'][0][field]=value
                with self.assertRaises(ValueError):f.final_cpu_recipe(bad)
            bad=copy.deepcopy(candidate);bad['recipe']['windows_crt']='other'
            with self.assertRaises(ValueError):f.final_cpu_recipe(bad)
            for field,value in [('url','https://unreviewed.invalid/wheel.whl'),('filename','ctranslate2-upstream.whl')]:
                bad=copy.deepcopy(candidate);bad['recipe']['wheels'][0][field]=value
                self.assertFalse(f.final_cpu_recipe(bad))

    def test_cached_copy_never_downloads_or_accepts_modified_bytes(self):
        with temporary_root() as root:
            item=pin(b'tiny');source=root/item['sha256'];source.write_bytes(b'tiny')
            f.copy_cached(item,root,root/'copy');self.assertEqual((root/'copy').read_bytes(),b'tiny')
            source.write_bytes(b'evil')
            with self.assertRaises(ValueError):f.copy_cached(item,root,root/'other')

    def test_prepared_cancellation_audio_is_bounded_streamed_source_repetition(self):
        with temporary_root() as root:
            cache=root/'cache';cache.mkdir();metadata=root/'metadata';metadata.mkdir()
            audio=root/'audio.wav';pcm=b'\x01\x00'*176000
            with wave.open(str(audio),'wb') as output:
                output.setparams((1,2,16000,0,'NONE','not compressed'));output.writeframes(pcm)
            a=pin(audio.read_bytes());a['path']='jfk.wav'
            model=pin(b'fixture');model['path']='model.bin'
            (cache/a['sha256']).write_bytes(audio.read_bytes());(cache/model['sha256']).write_bytes(b'fixture')
            (metadata/'fixtures.json').write_text(json.dumps({'audio':a,'faster_whisper_tiny':{'version':'reviewed','files':[model]}}))
            with patch.object(f,'ROOT',metadata):env,report=f.prepare(cache,root/'fixtures é 测试')
            repeated=Path(env['LUMA_ASR_TEST_LONG_AUDIO'])
            self.assertEqual(report['cancellation_audio']['bytes'],35_200_044)
            self.assertEqual(report['cancellation_audio']['duration_ms'],1_100_000)
            self.assertFalse(report['new_downloads'])
            with wave.open(str(repeated),'rb') as source:
                for _ in range(100):self.assertEqual(source.readframes(176000),pcm)
                self.assertEqual(source.readframes(1),b'')
            self.assertEqual(f.sha256(repeated),report['cancellation_audio']['sha256'])

    def test_native_failure_cleans_copied_inputs_and_partial_fixtures(self):
        for where in ('prepare','cargo'):
            with self.subTest(where=where), temporary_root() as root:
                cache=root/'cache';cache.mkdir();output=root/'output';output.mkdir()
                python=pin(b'python');(cache/python['sha256']).write_bytes(b'python')
                candidate={'id':'fixture','recipe':{'python':python,'wheels':[]}}
                candidates=root/'candidates.json';candidates.write_text(json.dumps({'runtimes':[candidate]}))
                def prepare(cache,directory):
                    directory.mkdir();(directory/'partial-model').write_bytes(b'fixture')
                    if where=='prepare':raise RuntimeError('expected preparation failure')
                    return {},{'fixture':True}
                with patch.object(f,'final_cpu_recipe',return_value=True), patch.object(f,'prepare',side_effect=prepare), patch.object(proof,'owned',side_effect=RuntimeError('expected cargo failure')):
                    with self.assertRaisesRegex(RuntimeError,'expected'):
                        proof.prove_native_installer('fixture',output,cache,candidates)
                self.assertEqual(list(output.iterdir()),[])
                self.assertTrue((cache/python['sha256']).exists())

    def test_repeat_rejects_unreviewed_format_or_duration(self):
        with temporary_root() as root:
            audio=root/'bad.wav'
            with wave.open(str(audio),'wb') as output:
                output.setparams((1,2,16000,0,'NONE','not compressed'));output.writeframes(b'\0'*32)
            with self.assertRaises(ValueError):f.repeat_audio(audio,root/'out.wav')


if __name__=='__main__':unittest.main()
