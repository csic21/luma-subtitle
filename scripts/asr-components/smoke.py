#!/usr/bin/env python3
"""Test extracted pack using only its private interpreter in a poisoned env."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile
from build import ROOT, ALLOWED, dump, fetch, safe_name, sha256


def extract(archive, destination):
    destination.mkdir(parents=True)
    with zipfile.ZipFile(archive) as z:
        names = set()
        for entry in z.infolist():
            path = safe_name(entry.filename)
            if entry.filename in names or entry.is_dir() or (entry.external_attr >> 16) & 0o170000 != 0o100000:
                raise ValueError('Archive must contain distinct regular files only')
            names.add(entry.filename)
            target = destination.joinpath(*path.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with z.open(entry) as f, target.open('wb') as out:
                shutil.copyfileobj(f, out)
            target.chmod((entry.external_attr >> 16) & 0o777)


def clean_environment(home):
    home.mkdir(parents=True)
    keep = {'SystemRoot', 'WINDIR', 'COMSPEC', 'TEMP', 'TMP', 'SYSTEMDRIVE'}
    env = {k: v for k, v in os.environ.items() if k in keep}
    env.update({'HOME': str(home), 'USERPROFILE': str(home), 'APPDATA': str(home / 'AppData'),
                'LOCALAPPDATA': str(home / 'Local'), 'PATH': str(home / 'no-executables'),
                'PYTHONPATH': str(home / 'poison'), 'PYTHONHOME': str(home / 'not-python'),
                'PYTHONNOUSERSITE': '1', 'HF_HOME': str(home / 'empty-hf-cache'),
                'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1', 'HF_HUB_DISABLE_TELEMETRY': '1',
                'HF_HUB_DISABLE_IMPLICIT_TOKEN': '1', 'DO_NOT_TRACK': '1', 'LANG': 'C.UTF-8',
                'NUMBA_CACHE_DIR': str(home / 'numba-cache')})
    poison = home / 'poison'; poison.mkdir()
    (poison / 'sitecustomize.py').write_text("raise RuntimeError('inherited Python path leaked into component')\n")
    return env


def worker_requests(executable, worker, requests, env, cwd, timeout=600):
    proc = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(worker)],
                          input=''.join(json.dumps(r) + '\n' for r in requests), text=True,
                          encoding='utf-8', capture_output=True, env=env, cwd=cwd, timeout=timeout)
    if proc.returncode:
        raise RuntimeError(f'Private worker failed ({proc.returncode}): {proc.stderr[-12000:]}')
    try:
        frames = [json.loads(line) for line in proc.stdout.splitlines() if line]
    except ValueError as exc:
        raise AssertionError('Worker stdout was not clean JSON lines') from exc
    if not frames:
        raise AssertionError('Worker emitted no protocol response')
    return frames


def terminate_idle_worker(command, env, cwd):
    idle = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, env=env, cwd=cwd)
    try:
        # communicate() would close stdin and turn this into an EOF test.
        idle.wait(timeout=0.2)
        idle.communicate(timeout=15)
        raise AssertionError('Idle worker exited without EOF or termination')
    except subprocess.TimeoutExpired:
        idle.kill(); idle.communicate(timeout=15)
        assert idle.returncode is not None


def main():
    p = argparse.ArgumentParser(); p.add_argument('--manifest', type=Path, required=True); p.add_argument('--worker', type=Path, required=True); p.add_argument('--cache', type=Path, required=True)
    a = p.parse_args(); manifest = json.loads(a.manifest.read_text()); output = a.manifest.parent
    archive = output / manifest['archive']['url'].rsplit('/', 1)[-1]
    assert sha256(archive) == manifest['archive']['sha256'] and archive.stat().st_size == manifest['archive']['bytes']
    with tempfile.TemporaryDirectory(prefix='luma-asr-clean-') as temp:
        work = Path(temp); unpacked = work / 'first-location'; extract(archive, unpacked)
        assert sum(p.stat().st_size for p in unpacked.rglob('*') if p.is_file()) == manifest['installed_bytes']
        assert len([p for p in unpacked.rglob('*') if p.is_file()]) == manifest['max_files']
        relocated = work / 'Relocated private runtime é 测试'; unpacked.rename(relocated)
        env = clean_environment(work / 'clean-home'); executable = relocated / manifest['entrypoint']
        result = subprocess.run([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(relocated / 'self_test.py')],
                                capture_output=True, text=True, encoding='utf-8', env=env, cwd=work, timeout=240)
        if result.returncode:
            raise RuntimeError(f'Private self-test failed ({result.returncode}):\n{result.stdout}\n{result.stderr}')
        imports = json.loads(result.stdout.splitlines()[-1])
        requests = [{'id': 'offline-url', 'op': 'probe', 'engine': manifest['engine'], 'device': manifest['device'], 'model_path': 'https://invalid.example/never-download'}]
        frames = worker_requests(executable, a.worker.resolve(), requests, env, work)
        response = frames[-1]
        assert response['id'] == 'offline-url' and response['event'] == 'probe' and response['ready'] is False
        assert response['code'] == 'local_path_required', response
        terminate_idle_worker([str(executable), '-I', '-B', '-u', '-X', 'utf8', str(a.worker.resolve())], env, work)
        recovered = worker_requests(executable, a.worker.resolve(), requests, env, work)
        assert recovered[-1]['code'] == 'local_path_required'
        inference = {'tested': False, 'reason': 'Qwen imports/API only; model weights and memory fit are untested.' if manifest['backend'] == 'qwen3-asr' else 'MLX tiny fixture access is paused; no model inference performed.'}
        if manifest['backend'] == 'faster-whisper':
            fixture = json.loads((ROOT / 'fixtures.json').read_text()); a.cache.mkdir(parents=True, exist_ok=True)
            # Build-time, reviewed public fixture downloads. Workers stay offline.
            ALLOWED.update({'huggingface.co', 'raw.githubusercontent.com'})
            model = work / 'fixture-model'; model.mkdir()
            for item in fixture['faster_whisper_tiny']['files']:
                target = model.joinpath(*safe_name(item['path']).parts); target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(fetch(item, a.cache), target)
            audio = work / 'jfk.wav'; shutil.copyfile(fetch(fixture['audio'], a.cache), audio)
            common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(model), 'audio_path': str(audio), 'language': 'en'}
            frames = worker_requests(executable, a.worker.resolve(), [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'), dict(common, id='warm', op='transcribe')], env, work)
            by_id = {f['id']: f for f in frames if f.get('event') in {'probe', 'result', 'error'}}
            assert by_id['probe']['ready'], by_id
            for key in ('cold', 'warm'):
                result = by_id[key]
                assert result['event'] == 'result' and result['device'] == 'cpu', result
                assert 'country' in ' '.join(s['text'] for s in result['segments']).lower(), result
                previous = 0
                for segment in result['segments']:
                    assert previous <= segment['start_ms'] < segment['end_ms'] <= 11001
                    previous = segment['end_ms']
            assert not by_id['cold']['reused'] and by_id['warm']['reused']
            inference = {'tested': True, 'backend': 'faster-whisper', 'device': 'cpu', 'cold_and_warm': True,
                         'segments': by_id['cold']['segments'], 'model_revision': fixture['faster_whisper_tiny']['version'],
                         'audio_sha256': fixture['audio']['sha256']}
        report = {'schema': 1, 'pack_id': manifest['id'], 'source_sha': manifest['source_sha'],
                  'archive_sha256': manifest['archive']['sha256'], 'relocated': True, 'isolated': True,
                  'system_python_used': False, 'system_packages_used': False, 'offline_protocol_tested': True,
                  'worker_test': {'passed': True, 'checks': ['json-lines', 'offline-path-rejection', 'clean-eof-shutdown', 'idle-termination', 'recovery']},
                  'imports': imports, 'inference': inference,
                  'limitations': ['No CUDA validation or CUDA redistribution.', 'No Developer ID signing, notarization, Gatekeeper bypass, or clean-GUI-machine validation.']}
        dump(output / f'{manifest["id"]}.smoke.json', report)
        print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == '__main__':
    main()
