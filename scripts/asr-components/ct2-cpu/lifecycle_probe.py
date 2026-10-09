#!/usr/bin/env python3
"""Diagnostic-only lifecycle comparison; execute with the exact private PBS."""
import argparse
import gc
import importlib
import importlib.metadata
import importlib.util
import inspect
import io
import json
import math
import os
from pathlib import Path
import runpy
import sys
import time
from types import SimpleNamespace
import faulthandler


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + '\n', encoding='utf-8', newline='\n')
    temporary.replace(path)


def validate_result(frame, reused):
    if (frame.get('event') != 'result' or frame.get('device') != 'cpu'
            or frame.get('backend') != 'faster-whisper' or frame.get('reused') is not reused
            or not isinstance(frame.get('segments'), list)):
        raise ValueError('Unexpected diagnostic inference result')
    previous = 0
    for segment in frame['segments']:
        if not previous <= segment['start_ms'] < segment['end_ms'] <= 11001:
            raise ValueError('Invalid diagnostic segment timestamps')
        previous = segment['end_ms']
    if 'country' not in ' '.join(s['text'] for s in frame['segments']).lower():
        raise ValueError('Diagnostic transcript does not match the pinned fixture')
    for key in ('load_seconds', 'inference_seconds'):
        value = frame.get(key)
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ValueError('Invalid observed diagnostic timing')
    return {key: frame[key] for key in ('id', 'segments', 'reused', 'load_seconds', 'inference_seconds')}


def thread_override(constructor, journal):
    if 'cpu_threads' not in inspect.signature(constructor).parameters:
        raise ValueError('Pinned faster-whisper constructor no longer exposes cpu_threads')
    def wrapped(*args, **kwargs):
        if 'cpu_threads' in kwargs:
            raise ValueError('Original worker unexpectedly supplied cpu_threads')
        journal.record('diagnostic.cpu_threads', 'explicit', {'value': 1})
        return constructor(*args, **dict(kwargs, cpu_threads=1))
    return wrapped


def retained_factory(worker_class, holder, journal):
    def create():
        if not holder:
            holder.append(worker_class())
            journal.record('worker.ownership', 'retained')
        return holder[0]
    return create


def main():
    parser = argparse.ArgumentParser()
    for name in ('root', 'worker', 'original-verifier', 'model', 'switch-model', 'audio', 'reports'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--case', choices=('eof', 'unload', 'switch'), required=True)
    parser.add_argument('--threads', choices=('default', 'one'), required=True)
    args = parser.parse_args()
    args.reports.mkdir(parents=True, exist_ok=False)
    root = args.root.resolve()
    spec = importlib.util.spec_from_file_location('original_verifier', args.original_verifier)
    original = importlib.util.module_from_spec(spec); spec.loader.exec_module(original)
    journal = original.StageJournal(args.reports / 'stages.json')
    report = {'schema': 1, 'kind': 'ct2-lifecycle-diagnostic', 'publication_authorized': False,
              'passed': False, 'case': args.case, 'cpu_threads': 'default' if args.threads == 'default' else 1,
              'performance_claim': False, 'case_timeout_seconds': 120, 'aggregate_timeout_seconds': 900,
              'cpu_threads_argument': 0 if args.threads == 'default' else 1,
              'upstream_default_thread_rule': 'min(4, hardware_concurrency); OMP_NUM_THREADS absent',
              'os_cpu_count_observed': os.cpu_count(), 'single_process_thread_claim': False,
              'results': [], 'normal_exit_observed_by_parent': False}
    trace = (args.reports / 'traceback.log').open('w', encoding='utf-8')
    faulthandler.dump_traceback_later(90, repeat=False, file=trace)
    journal.record('bootstrap', 'started')
    holder = []
    try:
        if (sys.platform != 'win32' or sys.version_info[:3] != (3, 12, 15)
                or not sys.flags.isolated or not sys.flags.dont_write_bytecode
                or Path(sys.prefix).resolve() != root):
            raise RuntimeError('Expected exact isolated private Windows PBS')
        worker = runpy.run_path(str(args.worker))
        worker['configure_offline'](); sys.addaudithook(worker['offline_audit'])
        for name in ('ctranslate2', 'faster_whisper', 'av', 'numpy', 'onnxruntime', 'tokenizers'):
            with journal.stage('import.' + name):
                module = importlib.import_module(name)
            if not Path(module.__file__).resolve().is_relative_to(root):
                raise RuntimeError('Diagnostic import escaped private runtime')
        import ctranslate2
        import faster_whisper
        if ctranslate2.__version__ != '4.8.2' or importlib.metadata.version('faster-whisper') != '1.2.1':
            raise RuntimeError('Unexpected diagnostic inference versions')
        if ctranslate2.get_cuda_device_count() != 0 or not {'float32', 'int8'}.issubset(ctranslate2.get_supported_compute_types('cpu')):
            raise RuntimeError('Expected complete CPU-only inference backend')
        if args.threads == 'one':
            faster_whisper.WhisperModel = thread_override(faster_whisper.WhisperModel, journal)
        serve = worker['serve']; worker_class = worker['Worker']
        if args.case != 'eof':
            serve.__globals__['Worker'] = retained_factory(worker_class, holder, journal)
        # Observe GC separately without replacing its implementation or the
        # native model assignment/destructor in the original Worker.unload.
        def observed_collect(*a, **kw):
            with journal.stage('python.gc.collect'):
                return gc.collect(*a, **kw)
        worker_class.unload.__globals__['gc'] = SimpleNamespace(collect=observed_collect)
        common = {'engine': 'whisper-accelerated', 'device': 'cpu', 'model_path': str(args.model),
                  'audio_path': str(args.audio), 'language': 'en'}
        requests = [dict(common, id='probe', op='probe'), dict(common, id='cold', op='transcribe'),
                    dict(common, id='warm', op='transcribe')]
        class Capture(original.ProtocolCapture):
            def write(self, value):
                count = super().write(value)
                # Original serve emits exactly one complete JSON frame/write.
                frame = json.loads(value)
                if frame.get('event') == 'error':
                    raise RuntimeError('Original worker returned an inference error: ' + str(frame.get('code')))
                if frame.get('event') == 'probe' and frame.get('ready') is not True:
                    raise RuntimeError('Original worker probe failed')
                if frame.get('event') == 'result':
                    expected = frame.get('id') == 'warm'
                    report['results'].append(validate_result(frame, expected))
                    report['last_result_elapsed_seconds'] = time.monotonic() - started
                    # Audit while the actual live Worker still owns its model.
                    system = Path(next(v for k, v in os.environ.items() if k.upper() == 'SYSTEMROOT')).resolve()
                    with journal.stage('loaded_modules.validation'):
                        report['loaded_modules'] = original.validate_loaded_paths(original.loaded_modules(), root, system)
                    write_json(args.reports / 'results.json', report)
                return count
        output = Capture(journal)
        started = time.monotonic()
        journal.record('bootstrap', 'completed')
        with journal.stage('worker.serve.' + args.case):
            serve(io.StringIO(''.join(json.dumps(r) + '\n' for r in requests)), output)
        if args.case == 'switch':
            # A second existing, hash-identical fixture path triggers the real
            # original model-key change and Worker.unload inside transcribe.
            switch = dict(common, model_path=str(args.switch_model), id='switch', op='transcribe')
            with journal.stage('worker.model_switch'):
                serve(io.StringIO(json.dumps(switch) + '\n'), output)
        if holder:
            with journal.stage('worker.explicit_unload'):
                holder[0].unload()
            if holder[0].model is not None or holder[0].key is not None or holder[0].loaded_runtime is not None:
                raise RuntimeError('Original Worker.unload did not clear its state')
            with journal.stage('worker.release_owner'):
                holder.clear()
                gc.collect()
        expected_ids = ['cold', 'warm', 'switch'] if args.case == 'switch' else ['cold', 'warm']
        if [r['id'] for r in report['results']] != expected_ids:
            raise RuntimeError('Diagnostic result sequence is incomplete')
        journal.record('cleanup', 'completed')
        report['passed'] = True
    except BaseException as error:
        report.update(passed=False, error=type(error).__name__ + ': ' + str(error))
        journal.record('diagnostic', 'failed', {'exception_type': type(error).__name__})
        raise
    finally:
        write_json(args.reports / 'result.json', report)
        faulthandler.cancel_dump_traceback_later()
        trace.close()
    # The parent also requires process returncode=0; this file alone cannot
    # turn an interpreter-finalization hang into a passing lifecycle result.


if __name__ == '__main__':
    main()
