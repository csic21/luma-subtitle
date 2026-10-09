import hashlib
import base64
from contextlib import contextmanager
import io
import json
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch
from types import SimpleNamespace
from fixture_paths import temporary_root

import windows_crt_proof as crt
from windows_native_inventory import closure
from decode_crt_terms import decode, compact
from generate_recipes import REPO, verified_term


class WindowsProofTests(unittest.TestCase):
    def test_native_workflow_test_batches_fail_on_each_command(self):
        for name in ('ci.yml', 'release.yml'):
            workflow = (REPO / '.github/workflows' / name).read_text(encoding='utf-8')
            block = workflow.split('- name: Managed ASR packaging and publication guard tests', 1)[1].split('- name:', 1)[0]
            self.assertIn('shell: bash', block, 'Python failures must not be masked by a later Node success')
        cpu = (REPO / '.github/workflows/asr-ct2-cpu.yml').read_text(encoding='utf-8')
        for name in ('Test input locks and binary inventory logic', 'Test the fixed non-executing CRT extraction helper'):
            block = cpu.split('- name: ' + name, 1)[1].split('- name:', 1)[0]
            self.assertIn('shell: pwsh', block)
            lines = block.splitlines()
            for index, line in enumerate(lines):
                if line.strip().startswith('python '):
                    self.assertEqual(lines[index + 1].strip(), 'if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }')

    def test_qwen_keeps_reviewed_private_openmp_but_not_host_fallback(self):
        def image(path, normal=(), delay=()): return {'path': path, 'normal': normal, 'delay': delay}
        torch = image('torch/torch_cpu.dll', ['libiomp5md.dll', 'kernel32.dll'], ['msvcp140.dll'])
        files = [torch, image('torch/libiomp5md.dll'), image('msvcp140.dll')]
        self.assertTrue(closure(files)['passed'])
        missing = closure(files[:-1]); self.assertFalse(missing['passed'])
        self.assertEqual(missing['blocked_dependencies'][0]['resolution'], 'missing_private_crt')
        self.assertFalse(closure([torch, image('msvcp140.dll')])['passed'])
        for gpu in ('cudnn64_9.dll', 'cufft64_11.dll', 'curand64_10.dll', 'cusolver64_11.dll', 'cusparse64_12.dll', 'nvJitLink_120_0.dll', 'torch_cuda.dll', 'c10_cuda.dll'):
            self.assertFalse(closure(files + [image(gpu)])['passed'], gpu)

    def test_private_pyd_imports_do_not_expand_windows_api_exemptions(self):
        def image(path, imports=()):
            return {'path': path, 'normal': list(imports), 'delay': []}
        for name in ('libtorchaudio.pyd', 'api-ms-win-example.pyd', 'ext-ms-win-example.pyd'):
            importer = image('torchaudio/_torchaudio.pyd', [name])
            missing = closure([importer]); self.assertFalse(missing['passed'])
            self.assertEqual(missing['blocked_dependencies'][0]['resolution'], 'unresolved')
            self.assertTrue(closure([importer, image('torchaudio/' + name)])['passed'])
        for name in ('api-ms-win-core-synch-l1-2-0.dll', 'ext-ms-win-ntuser-window-l1-1-0.dll'):
            result = closure([image('extension.pyd', [name])])
            self.assertTrue(result['passed'])
            self.assertEqual(result['dependencies'][0]['resolution'], 'windows_api_set')

    def test_crt_copy_preserves_companions_and_never_overwrites(self):
        with temporary_root() as tmp:
            base = tmp; runtime = base / 'runtime'; runtime.mkdir(); source = base / 'source'; source.mkdir()
            def pin(data): return {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
            (runtime / 'vcruntime140.dll').write_bytes(b'companion')
            (source / 'msvcp140.dll').write_bytes(b'original')
            (source / 'license-en.rtf').write_bytes(b'notice')
            (base / 'direct-crt-signatures.json').write_text('{"passed":true}')
            lock = {'source_url': 'https://fixture.invalid', 'installer': {'sha256': 'a'*64},
                    'dlls': {'msvcp140.dll': pin(b'original')}, 'notices': {'license-en.rtf': pin(b'notice')}}
            contract = base / 'contract.json'; contract.write_text(json.dumps(lock))
            companion = pin(b'companion')
            with patch.object(crt, 'CONTRACT', contract), patch.object(crt, 'COMPANIONS', {'vcruntime140.dll': (companion['bytes'], companion['sha256'])}):
                crt.install(runtime, source)
                self.assertEqual((runtime / 'msvcp140.dll').read_bytes(), b'original')
                self.assertEqual((runtime / 'vcruntime140.dll').read_bytes(), b'companion')
                receipt = (runtime / 'licenses' / crt.ID / 'provenance.json').read_text()
                self.assertNotIn(str(base), receipt)
                with self.assertRaisesRegex(ValueError, 'overwrite'): crt.install(runtime, source)

    def test_font_aware_chinese_decode_does_not_use_rtf_header_codepage(self):
        raw = br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0\fnil\fcharset0 Tahoma;}{\f1\fnil\fcharset134 SimSun;}{\f2\fnil\fcharset2 Symbol;}}\f1\'c8\'ed\'bc\'fe\par}"
        plain, normalized = decode(raw)
        self.assertEqual(compact(plain), '软件')
        self.assertIn('\\u-28817?', normalized)

    def test_full_crt_terms_preserve_reviewed_display_and_raw_identities(self):
        directory = REPO / 'src-tauri/resources/asr/terms'
        sources = json.loads((directory / 'sources.json').read_text(encoding='utf-8'))
        expected = {'microsoft-vc-runtime-en': 'f815ace86c91d3dadc62ed85292b7c79066447f2277774803df87b1d1a9ecfc8',
                    'microsoft-vc-runtime-zh-cn': 'bf17656105173f49b79623f1508c4c8eddf9b09e78a6ea79ee5aad99015eb38e'}
        for name, digest in expected.items():
            source = next(item for item in sources['terms'] if item['id'] == name)
            term = verified_term(source, directory)
            self.assertEqual(term['sha256'], digest)
            self.assertEqual(term['source_encoding'], 'rtf')
            self.assertIn(source['version'], term['text'])
            self.assertNotIn('\ufffd', term['text'])

    def test_real_app_gui_header_and_exact_helper_output_are_required(self):
        with temporary_root() as tmp:
            path = tmp / 'app.exe'; data = bytearray(512)
            data[:2] = b'MZ'; struct.pack_into('<I', data, 0x3c, 128); data[128:132] = b'PE\0\0'
            struct.pack_into('<H', data, 132, 0x8664); struct.pack_into('<H', data, 128 + 24 + 68, 2)
            path.write_bytes(data); self.assertEqual(crt.gui_subsystem(path), 2)
            struct.pack_into('<H', data, 128 + 24 + 68, 3); path.write_bytes(data)
            with self.assertRaisesRegex(ValueError, 'GUI subsystem'): crt.gui_subsystem(path)
        lock = json.loads(crt.CONTRACT.read_text(encoding='utf-8'))
        result = {'schema': 1, 'role': 'installer', 'mode': 'online', 'sha256': lock['installer']['sha256'], 'signer_sha256': crt.SIGNERS['installer']}
        output = '\nLUMA_CRT_SIGNATURE ' + json.dumps(result) + '\n'
        self.assertEqual(crt.signature_result(output, 'installer', 'online', lock), result)
        for invalid in (output + output, 'unexpected\n' + output, output.replace('"online"', '"cache-only"')):
            with self.assertRaises(ValueError): crt.signature_result(invalid, 'installer', 'online', lock)

    def test_helper_child_is_reaped_and_output_bounded(self):
        with temporary_root() as tmp:
            code, out, err = crt.helper_process([sys.executable, '-I', '-B', '-c', 'print("fixture")'], None, tmp)
            self.assertEqual((code,out.strip(),err), (0,'fixture',''))
            with self.assertRaisesRegex(ValueError, 'output bound'):
                crt.helper_process([sys.executable, '-I', '-B', '-c', 'print("x" * 20000)'], None, tmp)

    def test_cab_diagnostics_retain_exit_and_exact_non_utf8_bytes(self):
        with temporary_root() as tmp:
            code, out, err = crt.helper_process_bytes([sys.executable, '-I', '-B', '-c',
                'import sys; sys.stdout.buffer.write(b"\\xffCAB\\r\\n"); sys.stderr.buffer.write(b"\\xfeerror"); sys.exit(7)'], None, tmp)
            self.assertEqual((code, out, err), (7, b'\xffCAB\r\n', b'\xfeerror'))

    def test_cab_path_probe_requires_local_disk_paths_and_clean_environment(self):
        for path in (r'C:\Managed runtime é 测试', r'\\?\C:\Managed runtime é 测试'):
            self.assertEqual(str(crt.verbatim_disk_path(path)), r'\\?\C:\Managed runtime é 测试')
        for path in (r'\\server\share\cab', r'\\?\UNC\server\share\cab', r'C:cab', 'relative', r'C:\x\..\cab'):
            with self.assertRaises(ValueError): crt.verbatim_disk_path(path)
        work, temporary, system = Path('/private/work'), Path('/private/temp'), Path('/system')
        # Exercise both returned spellings on every host. Windows os.environ
        # uppercases keys, while the filtered result is an ordinary dictionary.
        for spelling in ('SystemRoot', 'SYSTEMROOT'):
            with self.subTest(system_root_key=spelling), patch.object(crt.os, 'environ',
                    {spelling: r'C:\Windows', 'PATH': 'poisoned', 'PYTHONPATH': 'poisoned', 'TEMP': 'poisoned'}):
                env = crt.cab_environment(work, temporary, system)
            self.assertIn(spelling, env)
            normalized = {key.upper(): value for key, value in env.items()}
            self.assertEqual(normalized['SYSTEMROOT'], r'C:\Windows'); self.assertNotIn('PYTHONPATH', normalized)
            self.assertNotIn('poisoned', str(env)); self.assertEqual(normalized['TEMP'], str(temporary))
            self.assertEqual(normalized['PATH'], ';'.join(map(str, (work, work / 'bin', system))))

    def test_cab_path_probe_records_legacy_failure_and_requires_all_fixed_hashes(self):
        def pin(data, member=None):
            value = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
            if member: value['member'] = member
            return value
        payloads = {'a12': b'min', 'msvcp140.dll_amd64': b'dll1', 'msvcp140_1.dll_amd64': b'dll2', 'u4': b'en', 'u28': b'zh'}
        lock = {'containers': {'attached.cab': pin(b'attached'), 'ux.cab': pin(b'ux')},
                'minimum_cab': pin(payloads['a12'], 'a12'),
                'dlls': {name: pin(payloads[name + '_amd64'], name + '_amd64') for name in ('msvcp140.dll', 'msvcp140_1.dll')},
                'notices': {name: pin(payloads[member], member) for name, member in (('license-en.rtf', 'u4'), ('license-zh-CN.rtf', 'u28'))}}
        with temporary_root() as tmp:
            source = tmp / 'source'; source.mkdir(); (source / 'attached.cab').write_bytes(b'attached'); (source / 'ux.cab').write_bytes(b'ux')
            contract = tmp / 'contract.json'; contract.write_text(json.dumps(lock))
            held = []; broken = [None]; calls = []
            @contextmanager
            def hold(path):
                held.append(path)
                try: yield
                finally: held.pop()
            def process(command, env, cwd):
                self.assertEqual(len(held), 1); self.assertEqual(Path.cwd(), parent_cwd)
                calls.append(command); member = command[2][3:]
                if Path(command[1]).is_absolute(): return 2, b'Cannot open input file.\xff', b'legacy diagnostic'
                self.assertTrue(all(argument.isascii() for argument in command[1:]))
                self.assertEqual(held[0], cwd / command[1])
                if broken[0] == 'exit': return 5, b'', b'fixed extraction failed'
                (cwd / command[3] / member).write_bytes(b'changed' if broken[0] == 'hash' else payloads[member])
                if broken[0] == 'extra': (cwd / command[3] / 'unexpected').write_bytes(b'bad')
                return 0, b'extracted', b''
            parent_cwd = Path.cwd()
            spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda module: None))
            audit = SimpleNamespace(system_expand=lambda: tmp / 'System32' / 'expand.exe')
            with patch.object(crt.sys, 'platform', 'win32'), patch.object(crt, 'CONTRACT', contract), \
                 patch.object(crt, 'verbatim_disk_path', side_effect=lambda path: path), patch.object(crt, 'held_cabinet', side_effect=hold), \
                 patch.object(crt.importlib.util, 'spec_from_file_location', return_value=spec), patch.object(crt.importlib.util, 'module_from_spec', return_value=audit), \
                 patch.object(crt, 'helper_process_bytes', side_effect=process), patch('sys.stdout', new_callable=io.StringIO):
                report = crt.prove_cab_paths(source, tmp, 'fixture-sha')
                self.assertTrue(report['passed']); self.assertEqual(len(report['results']), 6)
                self.assertEqual(report['results'][0]['exit_code'], 2)
                self.assertEqual(base64.b64decode(report['results'][0]['stdout_base64']), b'Cannot open input file.\xff')
                self.assertEqual({result['verified_sha256'] for result in report['results'][1:]}, {pin(data)['sha256'] for data in payloads.values()})
                self.assertEqual(json.loads((tmp / 'direct-crt-cab-paths.json').read_text())['source_sha'], 'fixture-sha')
                for failure, message in [('exit', 'Fixed relative CAB extraction failed'), ('hash', 'expected regular file|exact hash'), ('extra', 'unexpected outputs')]:
                    broken[0] = failure
                    with self.assertRaisesRegex((ValueError, RuntimeError), message): crt.prove_cab_paths(source, tmp, 'fixture-sha')

    def test_rust_cab_argv_and_status_guards_remain_in_the_owned_process_path(self):
        source = (REPO / 'src-tauri/src/asr_components/direct_crt.rs').read_text(encoding='utf-8')
        self.assertIn('cabinet.parent() != Some(work)', source); self.assertIn('cabinet_pin == SLICES[1].1 && pin == MINIMUM', source)
        self.assertIn('command.args(&arguments).current_dir(work)', source)
        self.assertIn('setup_process::run_owned_status(command, "Windows CAB extraction"', source)
        self.assertIn('expansion_failure(status, &output, cabinet_pin, pin)', source)
        self.assertNotIn('Check local Windows application-control policy', source)
        self.assertIn('open_verified(&path, pin, cancel)?', source)


if __name__ == '__main__': unittest.main()
