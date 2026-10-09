import ast
import json
from pathlib import Path
import unittest
from unittest.mock import patch
from fixture_paths import temporary_root
import prove_offline_pip as proof


class ProofTextEncodingTests(unittest.TestCase):
    def test_generated_utf8_reports_round_trip_under_cp1252_host_default(self):
        # U+5B50 contains UTF-8 byte0x90, which cp1252 cannot decode. Exercise the
        # actual orchestration reader, with only builds/children replaced by
        # tiny local fixtures. No engine, model or subprocess is involved.
        value={'label':'子 日本語 é 测试','path':'runtime 子/model 日本語'}
        with self.assertRaises(UnicodeDecodeError):json.dumps(value,ensure_ascii=False).encode('utf-8').decode('cp1252')
        with temporary_root(prefix='proof 子 ') as root:
            output=root/'output';output.mkdir();cache=root/'cache';cache.mkdir()
            candidates=root/'recipes.json';proof.dump(candidates,{'runtimes':[{'id':'fixture','label':value['label'],'recipe':{}}]})
            worker=root/'worker.py';worker.write_text('# source 子\n',encoding='utf-8')
            def build(pack,directory,cache,source,**kwargs):
                (directory/'staging'/pack).mkdir(parents=True)
                proof.dump(directory/'staging'/pack/'ASSEMBLY.json',value)
                proof.dump(directory/(pack+'.manifest.json'),value)
                return {'archive':{'sha256':'a'*64,'bytes':1}}
            def owned(command,**kwargs):proof.dump(output/'first/fixture.smoke.json',value)
            original=Path.read_text
            def cp1252_default(path,*args,**kwargs):
                if not args and 'encoding' not in kwargs:kwargs['encoding']='cp1252'
                return original(path,*args,**kwargs)
            argv=['proof','--pack','fixture','--source-sha','b'*40,'--output',str(output),'--cache',str(cache),
                  '--worker',str(worker),'--recipe-candidates',str(candidates)]
            with patch.object(proof.sys,'argv',argv),patch.object(proof,'build',side_effect=build), \
                 patch.object(proof,'owned',side_effect=owned),patch.object(proof,'prove_native_installer',return_value={'tested':False}), \
                 patch.object(Path,'read_text',cp1252_default),patch('builtins.print'):
                proof.main()
            report=json.loads((output/'fixture.offline-pip-proof.json').read_text(encoding='utf-8'))
            self.assertEqual(report['assembly'],value);self.assertEqual(report['runtime_smoke'],value)
            self.assertFalse((output/'first').exists());self.assertFalse((output/'second').exists())

    def test_adjacent_proof_text_io_always_names_its_encoding(self):
        for name in ('prove_offline_pip.py','build.py','smoke.py','windows_crt_proof.py','real_worker_fixture.py','resolve_locks.py'):
            source=Path(__file__).with_name(name).read_text(encoding='utf-8')
            for node in ast.walk(ast.parse(source)):
                if (isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute)
                        and node.func.attr in ('read_text','write_text')):
                    encodings=[keyword.value for keyword in node.keywords if keyword.arg=='encoding']
                    self.assertEqual(len(encodings),1,f'{name}:{node.lineno} must not use the host code page')
                    self.assertIn(ast.literal_eval(encodings[0]),('utf-8','utf-8-sig'))


if __name__=='__main__':unittest.main()
