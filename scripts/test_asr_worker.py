"""Dependency-free adapter/protocol tests. No models, downloads, or GPU needed.

Run from the repository root: python3 -B scripts/test_asr_worker.py
"""

import importlib.util
import io
import json
import os
from pathlib import Path, PureWindowsPath
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch
import wave

sys.dont_write_bytecode = True
WORKER_PATH = Path(__file__).resolve().parents[1] / "src-tauri/src/asr/worker.py"
spec = importlib.util.spec_from_file_location("asr_worker", WORKER_PATH)
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)


class Array(list):
    def astype(self, _dtype):
        return self

    def __truediv__(self, scalar):
        return Array(value / scalar for value in self)


NUMPY = NS(float32="float32", frombuffer=lambda data, dtype: Array(struct.unpack("<" + "h" * (len(data) // 2), data)))


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        # The worker canonicalizes local paths. macOS /var symlinks and Windows
        # short temp-directory names must have the same form in API assertions.
        self.root = Path(self.temp.name).resolve()
        self.model = self.root / "model"
        self.aligner = self.root / "aligner"
        self.model.mkdir()
        self.aligner.mkdir()
        self.audio = self.root / "audio.wav"
        with wave.open(str(self.audio), "wb") as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(16000)
            out.writeframes(struct.pack("<hh", -32768, 32767) + b"\0\0" * 31998)

    def write(self, root, name, data):
        (root / name).write_text(json.dumps(data) if isinstance(data, dict) else data, encoding="utf-8")

    def faster_files(self):
        for name in ("config.json", "preprocessor_config.json", "tokenizer.json"):
            self.write(self.model, name, {})
        self.write(self.model, "model.bin", "local model bytes")
        self.write(self.model, "vocabulary.txt", "local vocabulary")

    def qwen_files(self):
        for root, aligner in ((self.model, False), (self.aligner, True)):
            config = {"model_type": "qwen3_asr"}
            if aligner:
                config.update(timestamp_token_id=151705, timestamp_segment_time=80)
            self.write(root, "config.json", config)
            for name in ("preprocessor_config.json", "tokenizer_config.json", "vocab.json", "chat_template.json"):
                self.write(root, name, {})
            self.write(root, "merges.txt", "# local merges")
            self.write(root, "model.safetensors", "fake test weights")

    def request(self, engine="whisper-accelerated", **kwargs):
        return {"id": "test", "op": "transcribe", "engine": engine, "model_path": str(self.model),
                "aligner_path": str(self.aligner), "device": "cpu", "language": "auto", "audio_path": str(self.audio), **kwargs}

    def faster_modules(self, cuda=False):
        self.faster_files()
        model = NS(model=NS(device="cuda" if cuda else "cpu"),
                   transcribe=Mock(return_value=([NS(start=0.1, end=1.5, text=" hello ")], NS())))
        factory = Mock(return_value=model)
        modules = {"numpy": NUMPY, "ctranslate2": NS(get_cuda_device_count=lambda: int(cuda),
                    get_supported_compute_types=lambda device: {"float16", "float32"} if device == "cuda" else {"int8", "float32"}),
                   "faster_whisper": NS(WhisperModel=factory)}
        return modules, factory, model

    def test_ct2_constructor_spelling_does_not_change_cache_identity(self):
        modules,factory,_=self.faster_modules()
        engine=w.Worker()
        with patch.object(w,'optional_import',side_effect=lambda name,_:modules[name]), \
             patch.object(w,'faster_whisper_model_argument',return_value='D:\\same verified model') as adapt:
            engine.transcribe(self.request(),lambda event:None)
            engine.transcribe(self.request(),lambda event:None)
        self.assertEqual(factory.call_args.args[0],'D:\\same verified model')
        self.assertEqual(engine.key[1],str(self.model.resolve()))
        self.assertEqual(factory.call_count,1);self.assertEqual(adapt.call_count,1)

    def test_managed_ct2_cpu_constructor_and_warning_use_explicit_source_policy(self):
        for platform, policy, cuda, expected in (
                ('win32', 'luma-cpu-seq-1', False, {'cpu_threads': 1}),
                ('win32', None, False, {}), ('win32', 'other', False, {}),
                ('darwin', 'luma-cpu-seq-1', False, {}),
                ('linux', 'luma-cpu-seq-1', False, {}),
                ('win32', 'luma-cpu-seq-1', True, {})):
            with self.subTest(platform=platform, policy=policy, cuda=cuda):
                modules, factory, _ = self.faster_modules(cuda=cuda)
                with patch.object(w.sys, 'platform', platform), \
                     patch.object(w, 'LUMA_MANAGED_CT2_CPU_POLICY', policy, create=True), \
                     patch.object(w, 'optional_import', side_effect=lambda name, _: modules[name]), \
                     patch.dict(os.environ, LUMA_MANAGED_CT2_CPU_POLICY='luma-cpu-seq-1', CPU_THREADS='8'):
                    request = self.request(device='cuda' if cuda else 'cpu')
                    worker = w.Worker(); probe = worker.probe(request)
                    worker.transcribe(request, lambda event: None)
                    worker.transcribe(request, lambda event: None)
                kwargs = factory.call_args.kwargs
                self.assertEqual({key: value for key, value in kwargs.items() if key == 'cpu_threads'}, expected)
                self.assertEqual(kwargs['num_workers'], 1); self.assertTrue(kwargs['local_files_only'])
                self.assertEqual(factory.call_count, 1)
                self.assertEqual(w.MANAGED_CT2_CPU_WARNING in probe['warnings'], bool(expected))

    def ct2_windows_argument(self,value,*,same=True,other=False):
        class FakePath:
            def __init__(self,path):self.path=path
            def resolve(self,strict=False):
                self_assert.assertTrue(strict)
                path='D:\\different model' if other and not self.path.startswith('\\\\?\\') else self.path
                return PureWindowsPath(path)
            def samefile(self,canonical):
                self_assert.assertTrue(str(canonical).startswith('\\\\?\\'))
                return same
        self_assert=self
        with patch.object(w.sys,'platform','win32'),patch.object(w,'Path',side_effect=FakePath):
            return w.faster_whisper_model_argument(value)

    def test_ct2_extended_drive_adapter_preserves_unicode_and_checks_identity(self):
        normal='D:\\model 子 日本語 é'
        self.assertEqual(self.ct2_windows_argument('\\\\?\\'+normal),normal)
        for kwargs in ({'same':False},{'other':True}):
            with self.assertRaises(w.WorkerError) as caught:self.ct2_windows_argument('\\\\?\\'+normal,**kwargs)
            self.assertEqual(caught.exception.code,'model_path_identity_changed')

    def test_ct2_normal_paths_and_non_windows_inputs_are_unchanged(self):
        values=['D:\\model 子','\\\\server\\share\\model','/local/model']
        with patch.object(w.sys,'platform','win32'),patch.object(w,'Path') as filesystem:
            for value in values:self.assertEqual(w.faster_whisper_model_argument(value),value)
            filesystem.assert_not_called()
        with patch.object(w.sys,'platform','darwin'):
            value='\\\\?\\D:\\model';self.assertEqual(w.faster_whisper_model_argument(value),value)

    def test_ct2_namespace_or_ambiguous_path_is_never_reinterpreted(self):
        bad=['\\\\?\\UNC\\server\\share\\model','\\\\.\\D:\\model','\\\\?\\Volume{fixture}\\model',
             '\\\\?\\D:model','\\\\?\\D:\\model.','\\\\?\\D:\\model ',
             '\\\\?\\D:\\model:stream','\\\\?\\D:\\NUL.txt','\\\\?\\D:\\COM¹',
             '\\\\?\\D:\\one\\..\\two','\\\\?\\D:\\one/two','\\\\?\\D:\\\\model']
        for value in bad:
            with self.subTest(value=value),self.assertRaises(w.WorkerError) as caught:self.ct2_windows_argument(value)
            self.assertEqual(caught.exception.code,'unsupported_model_path')

    def test_ct2_limit_counts_utf16_units_and_longest_loader_filename(self):
        suffix='\\preprocessor_config.json'
        count=259-len('D:\\')-len(suffix)
        normal='D:\\'+'a'*count
        self.assertEqual(self.ct2_windows_argument('\\\\?\\'+normal),normal)
        with self.assertRaises(w.WorkerError):self.ct2_windows_argument('\\\\?\\'+normal+'a')
        # Replacing two ASCII units with one astral character preserves the bound;
        # replacing just one adds a UTF-16 unit and must fail.
        allowed='D:\\'+'a'*(count-2)+'😀'
        self.assertEqual(self.ct2_windows_argument('\\\\?\\'+allowed),allowed)
        with self.assertRaises(w.WorkerError):self.ct2_windows_argument('\\\\?\\'+'D:\\'+'a'*(count-1)+'😀')

    @unittest.skipUnless(sys.platform=='win32','real native Windows extended and Unicode path identities')
    def test_ct2_real_windows_extended_unicode_directory_roundtrip(self):
        directory=self.root/'model 子 日本語 é';directory.mkdir()
        normal=str(directory.resolve(strict=True))
        if normal.startswith('\\\\?\\'):normal=normal[4:]
        extended='\\\\?\\'+normal
        result=w.faster_whisper_model_argument(extended)
        self.assertEqual(result,normal)
        self.assertTrue(Path(result).samefile(extended))
        self.assertEqual(w.faster_whisper_model_argument(normal),normal)

    def qwen_modules(self, cuda=False):
        self.qwen_files()
        model = NS(device="cuda:0" if cuda else "cpu", forced_aligner=NS(device="cuda:0" if cuda else "cpu"),
                   transcribe=Mock(return_value=[NS(language="English", text="Hello, world!",
                       time_stamps=[NS(text="Hello", start_time=0.1, end_time=0.5), NS(text="world", start_time=0.6, end_time=1.2)])]))
        factory = Mock(return_value=model)
        modules = {"numpy": NUMPY, "torch": NS(float32="float32", float16="float16", bfloat16="bfloat16",
                    cuda=NS(is_available=lambda: cuda, is_bf16_supported=lambda: True)),
                   "qwen_asr": NS(Qwen3ASRModel=NS(from_pretrained=factory))}
        return modules, factory, model

    def invoke(self, worker, request, modules):
        with patch.object(w, "optional_import", side_effect=lambda name, package: modules[name]):
            return worker.handle(request, lambda event: None)

    def test_faster_cpu_and_warm_reuse(self):
        modules, factory, model = self.faster_modules()
        worker = w.Worker()
        cold = self.invoke(worker, self.request(), modules)
        warm = self.invoke(worker, self.request(language="en"), modules)
        self.assertEqual(cold["event"], "result", cold)
        self.assertFalse(cold["reused"])
        self.assertTrue(warm["reused"])
        self.assertEqual(warm["load_seconds"], 0)
        self.assertEqual(cold["device"], "cpu")
        self.assertEqual(cold["segments"], [{"start_ms": 100, "end_ms": 1500, "text": "hello"}])
        factory.assert_called_once_with(str(self.model), device="cpu", compute_type="int8", local_files_only=True, num_workers=1)
        self.assertIsInstance(model.transcribe.call_args.args[0], Array)
        self.assertFalse(model.transcribe.call_args.kwargs["vad_filter"])

    def test_changed_config_reloads(self):
        modules, factory, _ = self.faster_modules()
        worker = w.Worker()
        self.invoke(worker, self.request(), modules)
        next_model = self.root / "another-model"
        next_model.mkdir()
        for path in self.model.iterdir():
            (next_model / path.name).write_bytes(path.read_bytes())
        result = self.invoke(worker, self.request(model_path=str(next_model)), modules)
        self.assertFalse(result["reused"])
        self.assertEqual(factory.call_count, 2)

    def test_cuda_requires_runtime_support_not_os(self):
        modules, factory, _ = self.faster_modules(cuda=False)
        with patch.object(w.platform, "system", return_value="Windows"):
            result = self.invoke(w.Worker(), self.request(device="cuda"), modules)
            automatic = self.invoke(w.Worker(), self.request(device="auto"), modules)
        self.assertEqual(result["code"], "device_unavailable")
        self.assertEqual(automatic["device"], "cpu")
        self.assertEqual(factory.call_count, 1)

    def test_available_cuda_is_selected(self):
        modules, factory, _ = self.faster_modules(cuda=True)
        with patch.object(w.platform, "system", return_value="Linux"):
            result = self.invoke(w.Worker(), self.request(device="auto"), modules)
        self.assertEqual(result["device"], "cuda")
        self.assertEqual(factory.call_args.kwargs["compute_type"], "float16")

    def test_device_mismatch_not_reported_as_accelerated(self):
        modules, factory, model = self.faster_modules(cuda=True)
        model.model.device = "cpu"
        result = self.invoke(w.Worker(), self.request(device="cuda"), modules)
        self.assertEqual(result["code"], "device_mismatch")

    def test_probe_does_not_load_weights(self):
        modules, factory, _ = self.faster_modules()
        result = self.invoke(w.Worker(), self.request(op="probe"), modules)
        self.assertTrue(result["ready"], result)
        self.assertEqual(result["event"], "probe")
        self.assertEqual(result["model_bytes"], sum(path.stat().st_size for path in self.model.iterdir()))
        self.assertEqual(result["total_bytes"], result["model_bytes"])
        self.assertIn("unverified", " ".join(result["warnings"]))
        factory.assert_not_called()

    def test_probe_checks_numpy(self):
        modules, factory, _ = self.faster_modules()
        del modules["numpy"]
        result = self.invoke(w.Worker(), self.request(op="probe"), modules)
        self.assertFalse(result["ready"])
        factory.assert_not_called()

    def test_missing_tokenizer_cannot_trigger_hub_fallback(self):
        modules, factory, _ = self.faster_modules()
        (self.model / "tokenizer.json").unlink()
        result = self.invoke(w.Worker(), self.request(op="probe"), modules)
        self.assertFalse(result["ready"])
        self.assertEqual(result["code"], "incomplete_model")
        factory.assert_not_called()

    def test_official_small_conversion_without_preprocessor_is_supported(self):
        modules, factory, _ = self.faster_modules()
        (self.model / "preprocessor_config.json").unlink()
        result = self.invoke(w.Worker(), self.request(op="probe"), modules)
        self.assertTrue(result["ready"], result)
        factory.assert_not_called()

    def test_qwen_api_passes_local_aligner_and_seconds_are_milliseconds(self):
        modules, factory, model = self.qwen_modules()
        worker = w.Worker()
        result = self.invoke(worker, self.request("qwen3-asr", language="en"), modules)
        warm = self.invoke(worker, self.request("qwen3-asr", language="en"), modules)
        self.assertEqual(result["event"], "result", result)
        self.assertEqual(result["segments"], [{"start_ms": 100, "end_ms": 1200, "text": "Hello, world!"}])
        self.assertTrue(warm["reused"])
        factory.assert_called_once()
        kwargs = factory.call_args.kwargs
        self.assertEqual(kwargs["forced_aligner"], str(self.aligner))
        self.assertEqual(kwargs["forced_aligner_kwargs"]["device_map"], "cpu")
        self.assertTrue(kwargs["forced_aligner_kwargs"]["local_files_only"])
        self.assertTrue(kwargs["local_files_only"])
        self.assertFalse(kwargs["trust_remote_code"])
        self.assertTrue(kwargs["use_safetensors"])
        self.assertEqual(kwargs["dtype"], "float32")
        self.assertEqual(kwargs["max_inference_batch_size"], 1)
        self.assertEqual(model.transcribe.call_args.kwargs["language"], "English")
        self.assertTrue(model.transcribe.call_args.kwargs["return_time_stamps"])
        self.assertIsInstance(model.transcribe.call_args.kwargs["audio"][0], Array)
        self.assertEqual(model.transcribe.call_args.kwargs["audio"][1], 16000)

    def test_qwen_cuda_uses_runtime_and_checks_aligner_device(self):
        modules, factory, model = self.qwen_modules(cuda=True)
        result = self.invoke(w.Worker(), self.request("qwen3-asr", device="auto"), modules)
        self.assertEqual(result["device"], "cuda")
        self.assertEqual(factory.call_args.kwargs["dtype"], "bfloat16")
        model.forced_aligner.device = "cpu"
        mismatch = self.invoke(w.Worker(), self.request("qwen3-asr", device="cuda"), modules)
        self.assertEqual(mismatch["code"], "device_mismatch")

    def test_qwen_metal_rejected_without_runtime_import(self):
        with patch.object(w, "optional_import") as importer:
            result = w.Worker().handle(self.request("qwen3-asr", device="metal"), lambda event: None)
        self.assertEqual(result["code"], "device_unavailable")
        importer.assert_not_called()

    def test_rocm_is_not_misreported_as_nvidia_cuda(self):
        modules, factory, model = self.qwen_modules(cuda=True)
        modules["torch"].version = NS(hip="6.3")
        model.device = model.forced_aligner.device = "cpu"
        result = self.invoke(w.Worker(), self.request("qwen3-asr", device="auto"), modules)
        self.assertEqual(result["device"], "cpu")
        rejected = self.invoke(w.Worker(), self.request("qwen3-asr", device="cuda"), modules)
        self.assertEqual(rejected["code"], "device_unavailable")

    def test_qwen_language_and_alignment_required(self):
        modules, factory, _ = self.qwen_modules()
        unsupported = self.invoke(w.Worker(), self.request("qwen3-asr", language="ar"), modules)
        self.assertEqual(unsupported["code"], "unsupported_language")
        missing = self.invoke(w.Worker(), self.request("qwen3-asr", aligner_path=""), modules)
        self.assertEqual(missing["code"], "local_path_required")
        factory.assert_not_called()

    def test_qwen_model_and_aligner_are_not_interchangeable(self):
        self.qwen_files()
        with self.assertRaisesRegex(w.WorkerError, "not interchangeable"):
            w.model_files(self.model, "qwen3-asr-transformers", aligner=True)

    def test_sharded_models_require_all_safe_local_shards(self):
        self.qwen_files()
        (self.model / "model.safetensors").unlink()
        self.write(self.model, "model.safetensors.index.json", {"weight_map": {"a": "model-1.safetensors", "b": "model-2.safetensors"}})
        self.write(self.model, "model-1.safetensors", "first")
        with self.assertRaisesRegex(w.WorkerError, "model-2"):
            w.model_files(self.model, "qwen3-asr-transformers")
        self.write(self.model, "model-2.safetensors", "second")
        self.assertGreater(w.model_files(self.model, "qwen3-asr-transformers"), 0)
        self.write(self.model, "model.safetensors.index.json", {"weight_map": {"a": "../escape"}})
        with self.assertRaisesRegex(w.WorkerError, "unsafe"):
            w.model_files(self.model, "qwen3-asr-transformers")

    def test_missing_weights_and_lfs_pointers_fail_readiness(self):
        self.faster_files()
        self.write(self.model, "model.bin", "version https://git-lfs.github.com/spec/v1\noid sha256:fake")
        result = w.Worker().probe(self.request(op="probe"))
        self.assertFalse(result["ready"])
        self.assertIn("Git LFS pointer", result["error"])

    def test_mlx_cache_loaded_once_on_native_apple_silicon(self):
        self.write(self.model, "config.json", {"n_mels": 80, "n_audio_ctx": 1500, "n_vocab": 51865})
        self.write(self.model, "weights.npz", "local weights")
        holder = NS(model=None, model_path=None, get_model=Mock(return_value=object()))
        module = NS(ModelHolder=holder, transcribe=Mock(return_value={"segments": [{"text": "hello", "start": 0.1, "end": 1.0}]}))
        core = NS(metal=NS(is_available=lambda: True), set_default_device=Mock(), gpu="gpu", float16="float16")
        modules = {"numpy": NUMPY, "mlx.core": core, "mlx_whisper.transcribe": module}
        worker = w.Worker()
        with patch.object(w.platform, "system", return_value="Darwin"), patch.object(w.platform, "machine", return_value="arm64"):
            probe = self.invoke(worker, self.request(device="auto", op="probe"), modules)
            holder.get_model.assert_not_called()
            result = self.invoke(worker, self.request(device="auto"), modules)
            warm = self.invoke(worker, self.request(device="auto"), modules)
        self.assertTrue(probe["ready"], probe)
        self.assertEqual(result["device"], "metal")
        self.assertEqual(result["backend"], "mlx-whisper")
        self.assertTrue(warm["reused"])
        holder.get_model.assert_called_once_with(str(self.model), "float16")
        self.assertIsInstance(module.transcribe.call_args.args[0], Array)

    def test_pinned_mlx_rejects_model_safetensors_only_without_mutating_files(self):
        self.write(self.model, "config.json", {"n_mels": 80, "n_audio_ctx": 1500, "n_vocab": 51865})
        self.write(self.model, "model.safetensors", "unsupported filename in release 0.4.3")
        before = {path.name: path.read_bytes() for path in self.model.iterdir()}
        with patch.object(w.platform, "system", return_value="Darwin"), patch.object(w.platform, "machine", return_value="arm64"), patch.object(w, "optional_import") as importer:
            probe = w.Worker().probe(self.request(device="metal", op="probe"))
        self.assertFalse(probe["ready"])
        self.assertEqual(probe["code"], "unsupported_model_layout")
        self.assertIn("mlx-whisper 0.4.3", probe["error"])
        self.assertIn("weights.safetensors or weights.npz", probe["error"])
        self.assertEqual(before, {path.name: path.read_bytes() for path in self.model.iterdir()})
        importer.assert_not_called()

    def test_pinned_mlx_accepts_weights_safetensors(self):
        self.write(self.model, "config.json", {"n_mels": 80, "n_audio_ctx": 1500, "n_vocab": 51865})
        self.write(self.model, "weights.safetensors", "supported weights")
        self.assertEqual(w.model_files(self.model, "mlx-whisper"), sum(path.stat().st_size for path in self.model.iterdir()))

    def test_qwen_preserves_cjk_punctuation_without_spaces(self):
        text = "你好，世界！今天很好。"
        words = "你好世界今天很好"
        stamps = [NS(text=word, start_time=index * 0.2, end_time=index * 0.2 + 0.15) for index, word in enumerate(words)]
        result = w.qwen_segments(NS(text=text, language="Chinese", time_stamps=stamps), 2.0)
        self.assertEqual([s["text"] for s in result], ["你好，世界！", "今天很好。"])
        self.assertEqual("".join(s["text"] for s in result), text)
        self.assertEqual(result[1]["start_ms"], 800)

    def test_qwen_japanese_and_korean_preserve_original_text(self):
        for text, language, words in (("私は猫です。元気？", "Japanese", ["私", "は", "猫", "です", "元気"]),
                                      ("안녕, 세상! 반가워요.", "Korean", ["안녕", "세상", "반가워요"])):
            with self.subTest(language=language):
                stamps = [NS(text=word, start_time=index * 0.3, end_time=index * 0.3 + 0.2) for index, word in enumerate(words)]
                result = w.qwen_segments(NS(text=text, language=language, time_stamps=stamps), 2.0)
                self.assertEqual("".join(s["text"] for s in result), text)

    def test_qwen_preserves_accents_quotes_and_apostrophes(self):
        text = '“Café, l\'été!” Oui.'
        stamps = [NS(text=word, start_time=index * 0.3, end_time=index * 0.3 + 0.2) for index, word in enumerate(["Café", "l'été", "Oui"])]
        result = w.qwen_segments(NS(text=text, language="French", time_stamps=stamps), 2.0)
        self.assertEqual("".join(s["text"] for s in result), text)

    def test_cue_limits_use_actual_word_times(self):
        text = " ".join(["longword"] * 20)
        stamps = [NS(text="longword", start_time=index, end_time=index + 0.4) for index in range(20)]
        result = w.qwen_segments(NS(text=text, language="English", time_stamps=stamps), 21)
        self.assertGreater(len(result), 2)
        self.assertTrue(all(s["end_ms"] - s["start_ms"] <= 6000 for s in result))
        self.assertTrue(all(len(s["text"]) <= 84 for s in result))
        self.assertEqual("".join(s["text"] for s in result), text)
        self.assertEqual(result[-1]["end_ms"], 19400)

    def test_alignment_missing_dropped_reordered_and_extra_words_fail(self):
        cases = [None, [], [NS(text="hello", start_time=0, end_time=0.5)],
                 [NS(text="world", start_time=0, end_time=0.5), NS(text="hello", start_time=0.5, end_time=1)],
                 [NS(text="hello", start_time=0, end_time=0.5), NS(text="world", start_time=0.5, end_time=1), NS(text="extra", start_time=1, end_time=1.5)]]
        for stamps in cases:
            with self.subTest(stamps=stamps), self.assertRaises(w.WorkerError):
                w.qwen_segments(NS(text="hello world", language="English", time_stamps=stamps), 2)

    def test_invalid_timestamps_never_fabricated(self):
        for start, end in [(None, 1), (0, None), (-1, 1), (1, 1), (2, 1), (0, float("nan")),
                           (float("inf"), 1), (0, 2.1), (True, 1), ("0", 1)]:
            with self.subTest(start=start, end=end), self.assertRaises(w.WorkerError):
                w.timed_span(start, end, 2)
        with self.assertRaises(w.WorkerError):
            w.whisper_segments([NS(text="one", start=0, end=1), NS(text="two", start=0.9, end=1.5)], 2)
        with self.assertRaises(w.WorkerError):
            w.qwen_segments(NS(text="hi", language="English", time_stamps=[NS(text="hi", start_time=0, end_time=0)]), 2)

    def test_empty_audio_transcription_is_successful_empty_result(self):
        self.assertEqual(w.qwen_segments(NS(text="", language="", time_stamps=None), 2), [])
        self.assertEqual(w.whisper_segments([], 2), [])

    def test_wave_is_validated_and_decoded_without_ffmpeg(self):
        array, duration = w.read_audio(str(self.audio), NUMPY)
        self.assertEqual(duration, 2)
        self.assertEqual(array[0], -1)
        self.assertAlmostEqual(array[1], 32767 / 32768)
        with wave.open(str(self.audio), "wb") as out:
            out.setnchannels(2)
            out.setsampwidth(2)
            out.setframerate(44100)
            out.writeframes(b"\0" * 32)
        with self.assertRaisesRegex(w.WorkerError, "PCM16"):
            w.read_audio(str(self.audio), NUMPY)

    def test_runtime_errors_are_actionable(self):
        with patch.object(w.importlib, "import_module", side_effect=ImportError("missing dependency")):
            with self.assertRaisesRegex(w.WorkerError, "selected Python environment"):
                w.optional_import("qwen_asr", "qwen-asr")
        self.assertIn("smaller model", w.error_message(MemoryError("out of memory")))

    def test_remote_paths_rejected(self):
        for path in ("https://host/model", "Qwen/Qwen3-ASR-0.6B", "data:audio/wav;base64,AAA"):
            with self.subTest(path=path), self.assertRaises(w.WorkerError):
                w.local_path(path, "model")

    def test_audit_blocks_network_and_descendants(self):
        for event in ("socket.connect", "socket.getaddrinfo", "subprocess.Popen", "os.fork", "os.posix_spawn"):
            with self.subTest(event=event), self.assertRaises(w.WorkerError):
                w.offline_audit(event, ())
        w.offline_audit("open", ())

    def test_offline_flags_override_inherited_configuration(self):
        with patch.dict(os.environ, {"HF_HUB_OFFLINE": "0"}), patch.dict(sys.modules):
            w.configure_offline()
            self.assertEqual(os.environ["HF_HUB_OFFLINE"], "1")
            self.assertEqual(os.environ["HF_HUB_DISABLE_IMPLICIT_TOKEN"], "1")
            self.assertEqual(os.environ["TOKENIZERS_PARALLELISM"], "false")
            self.assertIsNone(sys.modules["vllm"])

    def test_json_lines_recovers_after_invalid_input(self):
        source = io.StringIO('invalid\n[]\n{"id":1,"op":"invalid"}\n{"id":"probe","op":"probe","engine":"qwen3-asr","device":"metal"}\n')
        output = io.StringIO()
        w.serve(source, output)
        events = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(len(events), 4)
        self.assertEqual(events[-1]["event"], "probe")
        self.assertFalse(events[-1]["ready"])
        self.assertEqual(events[-1]["id"], "probe")

    def test_protocol_progress_keeps_id(self):
        modules, _, _ = self.faster_modules()
        events = []
        with patch.object(w, "optional_import", side_effect=lambda name, package: modules[name]):
            result = w.Worker().handle(self.request(), events.append)
        self.assertEqual(result["event"], "result")
        self.assertTrue(events)
        self.assertTrue(all(event["id"] == "test" for event in events))
        self.assertTrue(all(event["event"] == "progress" for event in events))
        self.assertEqual(events[-1]["progress"], 0.75)

    def test_segment_progress_is_monotonic_and_uses_real_audio_time(self):
        progress = []
        segments = [NS(text="one", start=0.1, end=0.3), NS(text="two", start=0.4, end=1.5)]
        w.whisper_segments(segments, 2, progress.append)
        self.assertEqual(progress, [0.15, 0.75])

    def test_qwen_cpu_memory_estimate_counts_float32_asr_and_aligner(self):
        def write_weights(root, shape, dtype):
            header = json.dumps({"weight": {"dtype": dtype, "shape": shape, "data_offsets": [0, 8]}}).encode()
            (root / "model.safetensors").write_bytes(len(header).to_bytes(8, "little") + header + b"\0" * 8)
        write_weights(self.model, [2, 3], "BF16")
        write_weights(self.aligner, [4], "F16")
        estimate, warning = w.qwen_cpu_memory_warning({"model":str(self.model), "aligner":str(self.aligner)})
        self.assertEqual(estimate, 40)
        self.assertIn("BOTH", warning)
        self.assertIn("8 GB", warning)
        self.assertIn("additional RAM", warning)

    def test_qwen_cpu_memory_estimate_is_unknown_for_unreadable_weight_headers(self):
        estimate, warning = w.qwen_cpu_memory_warning({"model":str(self.model), "aligner":str(self.aligner)})
        self.assertIsNone(estimate)
        self.assertIn("could not be estimated", warning)
        self.assertIn("does not load weights", warning)

    def test_deep_weight_header_cannot_bypass_cpu_memory_warning(self):
        modules, _, _ = self.qwen_modules()
        header = b'{"weight":' + b'[' * 10000 + b'0' + b']' * 10000 + b'}'
        (self.model / "model.safetensors").write_bytes(len(header).to_bytes(8, "little") + header)
        with patch.object(w, "optional_import", side_effect=lambda name, package: modules[name]):
            report = w.Worker().probe(self.request(engine="qwen3-asr"))
        self.assertTrue(report["ready"])
        self.assertIsNone(report["estimated_weight_memory_bytes"])
        self.assertTrue(any("8 GB" in warning for warning in report["warnings"]))
        self.assertNotIn("error", report)

    def test_probe_error_after_import_never_leaves_ready_true(self):
        modules, _, _ = self.qwen_modules()
        with patch.object(w, "optional_import", side_effect=lambda name, package: modules[name]), \
                patch.object(w, "qwen_cpu_memory_warning", side_effect=RuntimeError("injected metadata error")):
            report = w.Worker().probe(self.request(engine="qwen3-asr"))
        self.assertFalse(report["ready"])
        self.assertEqual(report["code"], "probe_failed")

    def test_isolated_script_protocol_smoke(self):
        result = subprocess.run([sys.executable, "-I", "-u", "-X", "utf8", str(WORKER_PATH)],
                                input='{"id":"test","op":"probe","engine":"qwen3-asr","device":"metal"}\n',
                                capture_output=True, text=True, timeout=15, check=True)
        event = json.loads(result.stdout)
        self.assertEqual(event["event"], "probe")
        self.assertFalse(event["ready"])
        self.assertEqual(event["code"], "device_unavailable")


if __name__ == "__main__":
    unittest.main(verbosity=2)
