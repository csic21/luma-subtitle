"""Optional, local-only ASR worker. Stdout is exclusively the JSON-lines protocol.

Optional dependencies are imported only for the selected engine. The application
owns this process and kills/reaps it on cancellation. No child processes are used.
See docs/OPTIONAL_ASR.md for the protocol and intentionally separate setup steps.
"""

from __future__ import annotations

import gc
from contextlib import contextmanager
import importlib
import json
import math
import os
from pathlib import Path
import platform
import sys
import threading
import time
import unicodedata
import wave

# LUMA_NAGISA_COMPAT_SOURCE


SAMPLE_RATE = 16000
MAX_REQUEST_BYTES = 1024 * 1024
ALIGNER_LANGUAGES = {
    "zh": "Chinese", "en": "English", "yue": "Cantonese", "fr": "French",
    "de": "German", "it": "Italian", "ja": "Japanese", "ko": "Korean",
    "pt": "Portuguese", "ru": "Russian", "es": "Spanish",
}


class WorkerError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def configure_offline():
    # Override inherited values before importing any HF/Transformers packages.
    for name in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_DATASETS_OFFLINE",
                 "HF_HUB_DISABLE_TELEMETRY", "HF_HUB_DISABLE_IMPLICIT_TOKEN",
                 "DO_NOT_TRACK"):
        os.environ[name] = "1"
    os.environ["TOKENIZERS_PARALLELISM"] = "false"
    # The official wrapper opportunistically imports vLLM. This worker only uses
    # Transformers, and must never initialize a vLLM process pool.
    sys.modules["vllm"] = None


def offline_audit(event, _args):
    if event in {"socket.connect", "socket.getaddrinfo", "socket.bind",
                 "subprocess.Popen", "os.system", "os.posix_spawn", "os.fork"}:
        raise WorkerError("offline_only", "This ASR worker cannot use the network or "
                          "start child processes. Complete local setup separately.")


def optional_import(name, package):
    try:
        return importlib.import_module(name)
    except Exception as exc:
        raise WorkerError("runtime_unavailable", f"Cannot import {name}: {exc}. "
                          f"Install or repair {package} in the selected Python environment; "
                          "see docs/OPTIONAL_ASR.md. Nothing was downloaded.") from exc


def local_path(value, label, directory=True):
    if not isinstance(value, str) or not value.strip() or "\x00" in value:
        raise WorkerError("local_path_required", f"Choose an existing local {label}.")
    if "://" in value or value.startswith("data:"):
        raise WorkerError("local_path_required", f"{label} must be a local path, not a URL or model ID.")
    path = Path(value).expanduser()
    # Absolute paths remove dependence on the worker's launch directory.
    if not path.is_absolute() or not (path.is_dir() if directory else path.is_file()):
        raise WorkerError("local_path_required", f"{label} does not exist at an absolute local path: {value}")
    return path.resolve()


def faster_whisper_model_argument(value):
    """Preserve model identity while avoiding CT2's verbatim-path slash join.

    CT2 4.8.2 uses UTF-16 file I/O on Windows, but ModelFileReader joins its
    directory and filename with '/'. Win32 does not normalize that separator
    inside a verbatim namespace. Only a verified, short local-drive spelling is
    adapted here; configuration, cache keys and managed leases stay canonical.
    """
    if sys.platform != "win32":
        return value
    prefix = "\\\\?\\"
    if not value.startswith((prefix, "\\\\.\\")):
        return value
    ordinary = value[len(prefix):] if value.startswith(prefix) else ""
    message = ("CTranslate2 cannot use this Windows model path safely. Choose the same complete model "
               "in a shorter local-drive directory with ordinary file names; extended UNC/device paths "
               "and long paths are unsupported by this adapter. No model files were moved or changed.")
    if (len(ordinary) < 3 or not ordinary[0].isascii() or not ordinary[0].isalpha()
            or ordinary[1:3] != ":\\"):
        raise WorkerError("unsupported_model_path", message)
    reserved = {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"}
    reserved.update(stem + digit for stem in ("COM", "LPT") for digit in "123456789¹²³")
    parts = ordinary[3:].split("\\")
    if any(not part or part in (".", "..") or part.endswith((".", " "))
           or part.split(".", 1)[0].upper() in reserved
           or any(ord(char) < 32 or char in '<>:"/|?*' for char in part) for part in parts):
        raise WorkerError("unsupported_model_path", message)
    # Include the longest filename used by the reviewed Whisper loader. Do not
    # depend on machine-wide long-path policy or silently truncate a path.
    if len((ordinary + "\\preprocessor_config.json").encode("utf-16-le")) // 2 >= 260:
        raise WorkerError("unsupported_model_path", message)
    try:
        canonical = Path(value).resolve(strict=True)
        candidate = Path(ordinary)
        resolved = str(candidate.resolve(strict=True))
        if not resolved.startswith(prefix):
            resolved = prefix + resolved
        if Path(resolved).resolve(strict=True) != canonical or not candidate.samefile(canonical):
            raise WorkerError("model_path_identity_changed", "The Windows model path resolves to a different directory. Retry after checking the selected local model.")
    except OSError as exc:
        raise WorkerError("unsupported_model_path", message) from exc
    return ordinary


def required_file(root, name):
    relative = Path(name)
    if relative.is_absolute() or ".." in relative.parts:
        raise WorkerError("invalid_model", "Model index contains an unsafe shard path.")
    path = root / relative
    if not path.is_file() or path.stat().st_size == 0:
        raise WorkerError("incomplete_model", f"Missing or empty model file: {path}. "
                          "Download the complete matching model directory separately.")
    with path.open("rb") as handle:
        if handle.read(128).startswith(b"version https://git-lfs.github.com/spec/v1"):
            raise WorkerError("incomplete_model", f"{path} is a Git LFS pointer, not model data. "
                              "Download the actual model files separately.")
    return path


def read_json_file(root, name):
    path = required_file(root, name)
    if path.stat().st_size > 8 * 1024 * 1024:
        raise WorkerError("invalid_model", f"Model metadata is unexpectedly large: {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (ValueError, UnicodeError) as exc:
        raise WorkerError("invalid_model", f"Invalid JSON model metadata: {path}") from exc
    if not isinstance(data, dict):
        raise WorkerError("invalid_model", f"Expected an object in {path}.")
    return data


def model_files(root, backend, aligner=False):
    config = read_json_file(root, "config.json")
    names = ["config.json"]
    if backend == "faster-whisper":
        # Without tokenizer.json faster-whisper may try the Hub even with a
        # local model directory. Require it, rather than allowing that fallback.
        names += ["model.bin", "tokenizer.json"]
        vocabulary = next((name for name in ("vocabulary.json", "vocabulary.txt") if (root / name).is_file()), None)
        if vocabulary is None:
            raise WorkerError("incomplete_model", "CTranslate2 model is missing vocabulary.json or vocabulary.txt.")
        names.append(vocabulary)
        # Older official tiny/base/small conversions use the built-in feature
        # defaults and intentionally omit this file. Large-v3 supplies it.
        if (root / "preprocessor_config.json").is_file():
            names.append("preprocessor_config.json")
    elif backend == "mlx-whisper":
        if not all(key in config for key in ("n_mels", "n_audio_ctx", "n_vocab")):
            raise WorkerError("invalid_model", "Choose an MLX-converted Whisper directory for Metal.")
        # The pinned mlx-whisper 0.4.3 release predates upstream main's support
        # for model.safetensors. Match the released loader, not unreleased main.
        weights = next((name for name in ("weights.safetensors", "weights.npz")
                        if (root / name).is_file()), None)
        if weights is None:
            if (root / "model.safetensors").is_file():
                raise WorkerError("unsupported_model_layout", "The pinned mlx-whisper 0.4.3 runtime cannot load a "
                                  "model.safetensors-only directory. Choose a compatible MLX conversion containing "
                                  "weights.safetensors or weights.npz, such as the documented whisper-turbo model. "
                                  "Luma has not renamed or changed your model files.")
            raise WorkerError("incomplete_model", "MLX model needs weights.safetensors or weights.npz for mlx-whisper 0.4.3.")
        names.append(weights)
    else:
        if config.get("model_type") != "qwen3_asr":
            raise WorkerError("invalid_model", "Choose an original qwen-asr model directory, "
                              "not a native Transformers '-hf' export or Whisper model.")
        is_aligner = "timestamp_token_id" in config and "timestamp_segment_time" in config
        if aligner != is_aligner:
            raise WorkerError("invalid_model", "The ASR and forced-aligner directories are not interchangeable. "
                              "Select Qwen3-ASR for model_path and Qwen3-ForcedAligner for aligner_path.")
        names += ["preprocessor_config.json", "tokenizer_config.json"]
        if (root / "tokenizer.json").is_file():
            names.append("tokenizer.json")
        else:
            names += ["vocab.json", "merges.txt"]
        tokenizer_config = read_json_file(root, "tokenizer_config.json")
        if (root / "chat_template.json").is_file():
            names.append("chat_template.json")
        elif (root / "chat_template.jinja").is_file():
            names.append("chat_template.jinja")
        elif not tokenizer_config.get("chat_template"):
            raise WorkerError("incomplete_model", "Qwen model is missing its local chat template.")
        if (root / "model.safetensors").is_file():
            names.append("model.safetensors")
        else:
            index = read_json_file(root, "model.safetensors.index.json")
            mapping = index.get("weight_map")
            if not isinstance(mapping, dict) or not mapping or not all(isinstance(v, str) for v in mapping.values()):
                raise WorkerError("invalid_model", "Invalid safetensors shard index.")
            names += ["model.safetensors.index.json", *set(mapping.values())]
        if (root / "generation_config.json").is_file():
            names.append("generation_config.json")
    files = {required_file(root, name).resolve() for name in names}
    return sum(path.stat().st_size for path in files)



def qwen_cpu_weight_bytes(root):
    """Estimate loaded FP32 parameter bytes from bounded safetensors metadata.

    This is a weight-only lower bound, never an inference peak-RAM prediction.
    The ASR and aligner estimates must be added because both remain resident.
    """
    root = Path(root)
    try:
        if (root / "model.safetensors").is_file():
            names = ["model.safetensors"]
        else:
            index = read_json_file(root, "model.safetensors.index.json")
            names = sorted(set(index["weight_map"].values()))
        integer_sizes = {"BOOL": 1, "U8": 1, "I8": 1, "I16": 2, "U16": 2,
                         "I32": 4, "U32": 4, "I64": 8, "U64": 8}
        float_types = {"F64", "F32", "F16", "BF16", "F8_E4M3", "F8_E5M2"}
        total = 0
        for name in names:
            path = required_file(root, name)
            with path.open("rb") as handle:
                header_length = int.from_bytes(handle.read(8), "little")
                if not 1 <= header_length <= 16 * 1024 * 1024 or header_length + 8 > path.stat().st_size:
                    return None
                header = json.loads(handle.read(header_length))
            if not isinstance(header, dict):
                return None
            for key, tensor in header.items():
                if key == "__metadata__":
                    continue
                shape, dtype = tensor.get("shape"), tensor.get("dtype")
                if not isinstance(shape, list) or len(shape) > 8 or any(type(dim) is not int or dim < 0 for dim in shape):
                    return None
                count = math.prod(shape)
                if count > 1_000_000_000_000:
                    return None
                size = 4 if dtype in float_types else integer_sizes.get(dtype)
                if size is None:
                    return None
                total += count * size
        return total if total > 0 else None
    except (OSError, ValueError, TypeError, KeyError, AttributeError, RecursionError, OverflowError, WorkerError):
        return None


def qwen_cpu_memory_warning(config):
    asr_bytes = qwen_cpu_weight_bytes(config["model"])
    aligner_bytes = qwen_cpu_weight_bytes(config["aligner"])
    total = asr_bytes + aligner_bytes if asr_bytes is not None and aligner_bytes is not None else None
    estimate = (f"Weight-only FP32 estimate from both local safetensors headers: {total / (1024 ** 3):.2f} GiB. "
                if total is not None else "Weight-only RAM could not be estimated from the local safetensors headers. ")
    warning = ("Qwen CPU loads BOTH ASR and forced-aligner weights concurrently as float32. " + estimate
               + "Runtime, audio and activations need additional RAM. Do not assume this fits an 8 GB device. "
                 "This check does not load weights or verify memory fit.")
    return total, warning


def backend_name(engine, device):
    if engine == "qwen3-asr":
        return "qwen3-asr-transformers"
    if engine != "whisper-accelerated":
        raise WorkerError("invalid_engine", "Engine must be whisper-accelerated or qwen3-asr.")
    apple = platform.system() == "Darwin" and platform.machine().lower() in {"arm64", "aarch64"}
    if device == "metal" and not apple:
        raise WorkerError("device_unavailable", "MLX Metal requires native Apple Silicon macOS and an arm64 Python.")
    return "mlx-whisper" if apple and device in {"auto", "metal"} else "faster-whisper"


def runtime(backend, requested):
    warnings = []
    if backend == "mlx-whisper":
        mx = optional_import("mlx.core", "mlx-whisper")
        module = optional_import("mlx_whisper.transcribe", "mlx-whisper")
        if not getattr(getattr(mx, "metal", None), "is_available", lambda: False)():
            raise WorkerError("device_unavailable", "MLX cannot access Metal. Use native arm64 Python on Apple Silicon "
                              "or select CPU with a CTranslate2 model.")
        if not callable(getattr(getattr(module, "ModelHolder", None), "get_model", None)):
            raise WorkerError("runtime_incompatible", "Installed mlx-whisper lacks the supported model cache API. "
                              "Use the documented mlx-whisper version.")
        return {"device": "metal", "module": module, "core": mx, "compute_type": "float16", "warnings": warnings}
    if backend == "faster-whisper":
        if requested == "metal":
            raise WorkerError("device_unavailable", "faster-whisper supports CPU/CUDA; Metal needs MLX on Apple Silicon.")
        ct = optional_import("ctranslate2", "faster-whisper")
        module = optional_import("faster_whisper", "faster-whisper")
        cuda = False
        if requested != "cpu":
            try:
                cuda = ct.get_cuda_device_count() > 0 and bool(ct.get_supported_compute_types("cuda"))
            except Exception as exc:
                warnings.append(f"CUDA capability check failed: {exc}")
        if requested == "cuda" and not cuda:
            raise WorkerError("device_unavailable", "CTranslate2 cannot use CUDA. Check the NVIDIA driver/CUDA libraries "
                              "for your installed runtime, or choose CPU.")
        device = "cuda" if cuda and requested != "cpu" else "cpu"
        types = ct.get_supported_compute_types(device)
        preferences = ("float16", "int8_float16", "float32") if device == "cuda" else ("int8", "int8_float32", "float32")
        compute = next((item for item in preferences if item in types), None)
        if compute is None:
            raise WorkerError("device_unavailable", f"No supported CTranslate2 compute type on {device}.")
        return {"device": device, "module": module, "compute_type": compute, "warnings": warnings}
    if requested == "metal":
        raise WorkerError("device_unavailable", "Qwen3-ASR Metal is not supported in this version. Choose CPU or CUDA.")
    if globals().get("LUMA_MANAGED_QWEN_RUNTIME", False):
        luma_configure_numba_workqueue()
        luma_prepare_nagisa()
        luma_probe_numba_workqueue()
    torch = optional_import("torch", "qwen-asr and PyTorch")
    module = optional_import("qwen_asr", "qwen-asr")
    if globals().get("LUMA_MANAGED_QWEN_RUNTIME", False):
        luma_check_numba_workqueue(require_initialized=True)
    hip = bool(getattr(getattr(torch, "version", None), "hip", None))
    cuda = bool(torch.cuda.is_available()) and not hip if requested != "cpu" else False
    if hip and requested == "auto":
        warnings.append("ROCm acceleration is not supported in this version; using CPU.")
    if requested == "cuda" and not cuda:
        raise WorkerError("device_unavailable", "PyTorch cannot use CUDA. Select a CUDA-enabled PyTorch environment "
                          "and compatible NVIDIA driver, or choose CPU.")
    device = "cuda" if cuda else "cpu"
    dtype = (torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16) if cuda else torch.float32
    if not callable(getattr(getattr(module, "Qwen3ASRModel", None), "from_pretrained", None)):
        raise WorkerError("runtime_incompatible", "Installed qwen-asr lacks Qwen3ASRModel.from_pretrained. "
                          "Use the documented qwen-asr version.")
    return {"device": device, "module": module, "core": torch, "dtype": dtype, "warnings": warnings}


def configuration(request):
    device = request.get("device", "auto")
    if device not in {"auto", "cpu", "cuda", "metal"}:
        raise WorkerError("invalid_device", "Device must be auto, cpu, cuda, or metal.")
    engine = request.get("engine")
    backend = backend_name(engine, device)
    if engine == "qwen3-asr" and device == "metal":
        raise WorkerError("device_unavailable", "Qwen3-ASR Metal is not supported in this version. Choose CPU or CUDA.")
    model = local_path(request.get("model_path"), "model directory")
    model_bytes = model_files(model, backend)
    aligner = None
    aligner_bytes = 0
    if engine == "qwen3-asr":
        aligner = local_path(request.get("aligner_path"), "Qwen forced-aligner directory")
        aligner_bytes = model_files(aligner, backend, aligner=True)
    return {"engine": engine, "backend": backend, "requested_device": device,
            "model": str(model), "aligner": str(aligner) if aligner else None,
            "model_bytes": model_bytes, "aligner_bytes": aligner_bytes}


def qwen_language(language):
    if language in (None, "", "auto"):
        return None
    if language not in ALIGNER_LANGUAGES:
        raise WorkerError("unsupported_language", "Qwen timed subtitles support only "
                          + ", ".join(ALIGNER_LANGUAGES) + ". Choose Whisper for other languages.")
    return ALIGNER_LANGUAGES[language]


def read_audio(path, numpy):
    audio = local_path(path, "prepared WAV audio file", directory=False)
    try:
        with wave.open(str(audio), "rb") as handle:
            if (handle.getnchannels(), handle.getsampwidth(), handle.getframerate(), handle.getcomptype()) != (1, 2, SAMPLE_RATE, "NONE"):
                raise WorkerError("invalid_audio", "Expected prepared mono PCM16 WAV at 16000 Hz. Re-run audio preparation.")
            count = handle.getnframes()
            if count <= 0:
                raise WorkerError("invalid_audio", "Prepared audio is empty. Choose a file with speech.")
            frames = handle.readframes(count)
            if len(frames) != count * 2:
                raise WorkerError("invalid_audio", "Prepared WAV is truncated. Re-run audio preparation.")
    except (wave.Error, EOFError) as exc:
        raise WorkerError("invalid_audio", "Cannot read prepared PCM WAV. Re-run audio preparation.") from exc
    return numpy.frombuffer(frames, dtype="<i2").astype(numpy.float32) / 32768.0, count / SAMPLE_RATE


def value(item, name):
    return item.get(name) if isinstance(item, dict) else getattr(item, name, None)


def timed_span(start, end, duration, previous_end=0):
    try:
        if isinstance(start, (bool, str)) or isinstance(end, (bool, str)):
            raise ValueError()
        start, end = float(start), float(end)
        if not (math.isfinite(start) and math.isfinite(end) and 0 <= start < end <= duration + 0.001):
            raise ValueError()
        start_ms, end_ms = round(start * 1000), min(round(end * 1000), round(duration * 1000))
        if start_ms < previous_end or end_ms <= start_ms:
            raise ValueError()
        return start_ms, end_ms
    except (TypeError, ValueError, OverflowError) as exc:
        raise WorkerError("invalid_timestamps", "The engine returned missing, overlapping, zero-length, or out-of-range "
                          "timestamps. No approximate timings were created. Try Whisper or a different local model.") from exc


def whisper_segments(items, duration, progress=None):
    segments = []
    previous_end = 0
    for item in items:
        text = value(item, "text")
        if not isinstance(text, str):
            raise WorkerError("invalid_result", "Whisper returned a segment without text.")
        if not text.strip():
            continue
        start, end = timed_span(value(item, "start"), value(item, "end"), duration, previous_end)
        segments.append({"start_ms": start, "end_ms": end, "text": text.strip()})
        previous_end = end
        if progress:
            progress(min(1.0, end / (duration * 1000)))
    return segments


@contextmanager
def stage_heartbeat(emit, backend, device):
    """Keep an opaque inference stage visible without inventing percentages."""
    stop = threading.Event()

    def pulse():
        while not stop.wait(15):
            emit({"event": "progress", "message": "Transcription is still running",
                  "backend": backend, "device": device})

    thread = threading.Thread(target=pulse, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join()


def kept_chars(text):
    # Match the official aligner's speech-token filtering. Compatibility/case
    # normalization only maps equivalent text; original output is always sliced
    # from the transcript, preserving punctuation, accents, and CJK spacing.
    chars, positions = [], []
    for index, char in enumerate(text):
        for normalized in unicodedata.normalize("NFKC", char).casefold():
            if normalized == "'" or unicodedata.category(normalized)[0] in "LN":
                chars.append(normalized)
                positions.append(index)
    return "".join(chars), positions


def qwen_segments(result, duration):
    text = value(result, "text")
    if not isinstance(text, str):
        raise WorkerError("invalid_result", "Qwen returned no original transcript text.")
    stamps = value(result, "time_stamps")
    if not text.strip():
        if stamps is not None and list(stamps):
            raise WorkerError("alignment_mismatch", "Qwen returned aligned words for an empty transcript.")
        return []
    languages = [part.strip().lower() for part in str(value(result, "language") or "").split(",")]
    if not languages or any(part not in {name.lower() for name in ALIGNER_LANGUAGES.values()} for part in languages):
        raise WorkerError("unsupported_language", "The detected language is not supported by the Qwen forced aligner. "
                          "Choose Whisper or set one supported language explicitly.")
    if stamps is None:
        raise WorkerError("missing_alignment", "Qwen returned text without forced-alignment timestamps. "
                          "Select a complete local Qwen3-ForcedAligner model or use Whisper.")
    items = list(stamps)
    normalized, positions = kept_chars(text)
    units, cursor, previous_end = [], 0, 0
    for item in items:
        word = value(item, "text")
        token = kept_chars(word)[0] if isinstance(word, str) else ""
        if not token or normalized[cursor:cursor + len(token)] != token:
            raise WorkerError("alignment_mismatch", "Aligned words do not match the original Qwen transcript. "
                              "No text was discarded and no timings were guessed. Try another model or Whisper.")
        start, end = timed_span(value(item, "start_time"), value(item, "end_time"), duration, previous_end)
        units.append({"start_ms": start, "end_ms": end, "offset": cursor})
        cursor += len(token)
        previous_end = end
    if not units or cursor != len(normalized):
        raise WorkerError("alignment_mismatch", "Forced alignment does not cover the complete Qwen transcript. "
                          "No partial subtitles were accepted. Try Whisper or a different local model.")
    for index, unit in enumerate(units):
        offset = 0 if index == 0 else positions[unit["offset"]]
        next_offset = len(text) if index + 1 == len(units) else positions[units[index + 1]["offset"]]
        if next_offset <= offset:
            raise WorkerError("alignment_mismatch", "An aligned token boundary falls inside a normalized character.")
        unit["text"] = text[offset:next_offset]
    # Group real word/character spans into readable cues. Never divide the audio
    # duration into equal chunks. Each cue uses its first/last aligned timestamps.
    segments = []
    current = None
    for unit in units:
        cjk = any(unicodedata.east_asian_width(char) in {"W", "F"} for char in unit["text"])
        limit = 36 if cjk or (current and current["cjk"]) else 84
        split = current and (
            unit["start_ms"] - current["end_ms"] >= 800
            or unit["end_ms"] - current["start_ms"] > 6000
            or len(current["text"] + unit["text"]) > limit
            or any(char in current["tail"] for char in ".!?。！？")
        )
        if split:
            segments.append({key: current[key] for key in ("start_ms", "end_ms", "text")})
            current = None
        if current is None:
            current = {"start_ms": unit["start_ms"], "end_ms": unit["end_ms"],
                       "text": unit["text"], "cjk": cjk, "tail": unit["text"]}
        else:
            current.update(end_ms=unit["end_ms"], text=current["text"] + unit["text"],
                           cjk=current["cjk"] or cjk, tail=unit["text"])
    if current:
        segments.append({key: current[key] for key in ("start_ms", "end_ms", "text")})
    if "".join(segment["text"] for segment in segments) != text:
        raise WorkerError("alignment_mismatch", "Cue grouping did not preserve the complete original transcript.")
    return segments


class Worker:
    def __init__(self):
        self.key = None
        self.model = None
        self.loaded_runtime = None

    def unload(self):
        if self.loaded_runtime and self.loaded_runtime.get("backend") == "mlx-whisper":
            holder = self.loaded_runtime["module"].ModelHolder
            holder.model = None
            holder.model_path = None
        self.key = self.model = self.loaded_runtime = None
        gc.collect()

    def probe(self, request):
        report = {"id": request.get("id"), "event": "probe", "ready": False,
                  "backend": None, "device": None, "model_bytes": 0, "aligner_bytes": 0, "total_bytes": 0,
                  "capabilities": {"offline": True, "word_timestamps": True,
                                   "supported_languages": sorted(ALIGNER_LANGUAGES) if request.get("engine") == "qwen3-asr" else None},
                  "warnings": []}
        try:
            config = configuration(request)
            report.update(backend=config["backend"], model_bytes=config["model_bytes"],
                          aligner_bytes=config["aligner_bytes"], total_bytes=config["model_bytes"] + config["aligner_bytes"])
            info = runtime(config["backend"], config["requested_device"])
            optional_import("numpy", "numpy and the selected ASR runtime")
            report.update(ready=True, device=info["device"], warnings=info["warnings"] + [
                "Package/device and local-file checks passed. Model weights were not loaded; inference remains unverified."])
            if config["engine"] == "qwen3-asr" and info["device"] == "cpu":
                estimate, warning = qwen_cpu_memory_warning(config)
                report["estimated_weight_memory_bytes"] = estimate
                report["warnings"].append(warning)
            if info["device"] == "cpu":
                report["warnings"].append("CPU inference can be slow, especially for Qwen3-ASR.")
        except Exception as exc:
            report.update(ready=False, error=error_message(exc), code=getattr(exc, "code", "probe_failed"))
        return report

    def transcribe(self, request, emit):
        config = configuration(request)
        language = request.get("language", "auto")
        if language is not None and not isinstance(language, str):
            raise WorkerError("invalid_language", "Language must be an ISO code or auto.")
        selected_language = qwen_language(language) if config["engine"] == "qwen3-asr" else (None if language in (None, "auto", "") else language)
        key = (config["engine"], config["model"], config["aligner"], config["requested_device"])
        reused = key == self.key and self.model is not None
        numpy = optional_import("numpy", "numpy and the selected ASR runtime")
        audio, duration = read_audio(request.get("audio_path"), numpy)
        load_seconds = 0.0
        if not reused:
            self.unload()
            started = time.perf_counter()
            info = runtime(config["backend"], config["requested_device"])
            info["backend"] = config["backend"]
            emit({"event": "progress", "message": "Loading local model", "backend": config["backend"], "device": info["device"]})
            try:
                if config["backend"] == "mlx-whisper":
                    info["core"].set_default_device(info["core"].gpu)
                    model = info["module"].ModelHolder.get_model(config["model"], info["core"].float16)
                elif config["backend"] == "faster-whisper":
                    model = info["module"].WhisperModel(faster_whisper_model_argument(config["model"]), device=info["device"],
                                                         compute_type=info["compute_type"], local_files_only=True, num_workers=1)
                    actual = getattr(getattr(model, "model", None), "device", None)
                    if actual != info["device"]:
                        raise WorkerError("device_mismatch", "CTranslate2 loaded on an unexpected device. Select an explicit device and retry.")
                else:
                    kwargs = {"device_map": "cuda:0" if info["device"] == "cuda" else "cpu",
                              "dtype": info["dtype"], "local_files_only": True,
                              "trust_remote_code": False, "use_safetensors": True}
                    model = info["module"].Qwen3ASRModel.from_pretrained(
                        config["model"], forced_aligner=config["aligner"], forced_aligner_kwargs=dict(kwargs),
                        max_inference_batch_size=1, max_new_tokens=4096, **kwargs)
                    actual = str(getattr(model, "device", ""))
                    aligner_device = str(getattr(getattr(model, "forced_aligner", None), "device", ""))
                    if any(device.split(":")[0] != info["device"] for device in (actual, aligner_device)):
                        raise WorkerError("device_mismatch", "Qwen ASR/aligner loaded on an unexpected device. Select an explicit device and retry.")
                self.model, self.key, self.loaded_runtime = model, key, info
            except Exception:
                self.unload()
                raise
            load_seconds = time.perf_counter() - started
        info = self.loaded_runtime
        emit({"event": "progress", "message": "Transcribing with warm model" if reused else "Transcribing audio",
              "backend": config["backend"], "device": info["device"]})
        started = time.perf_counter()
        if config["backend"] == "mlx-whisper":
            with stage_heartbeat(emit, config["backend"], info["device"]):
                result = info["module"].transcribe(audio, path_or_hf_repo=config["model"], language=selected_language,
                                                     verbose=None, word_timestamps=True)
            segments = whisper_segments(result["segments"], duration)
        elif config["backend"] == "faster-whisper":
            raw_segments, _ = self.model.transcribe(audio, language=selected_language, word_timestamps=True,
                                                       vad_filter=False, beam_size=5)
            segments = whisper_segments(raw_segments, duration, lambda progress: emit({
                "event": "progress", "message": "Transcribing audio", "progress": progress,
                "backend": config["backend"], "device": info["device"]}))
        else:
            with stage_heartbeat(emit, config["backend"], info["device"]):
                results = self.model.transcribe(audio=(audio, SAMPLE_RATE), language=selected_language, return_time_stamps=True)
            if not isinstance(results, (list, tuple)) or len(results) != 1:
                raise WorkerError("invalid_result", "Qwen returned an unexpected number of results for one audio file.")
            if globals().get("LUMA_MANAGED_QWEN_RUNTIME", False):
                luma_check_numba_workqueue(require_initialized=True)
            segments = qwen_segments(results[0], duration)
        return {"id": request.get("id"), "event": "result", "segments": segments,
                "backend": config["backend"], "device": info["device"], "reused": reused,
                "load_seconds": load_seconds, "inference_seconds": time.perf_counter() - started}

    def handle(self, request, emit):
        request_id = request.get("id") if isinstance(request, dict) else None
        try:
            if not isinstance(request, dict) or not isinstance(request_id, (str, int)) or isinstance(request_id, bool):
                raise WorkerError("invalid_request", "Each request must be an object with a string or integer id.")
            operation = request.get("op", "transcribe")
            if operation == "probe":
                return self.probe(request)
            if operation != "transcribe":
                raise WorkerError("invalid_request", "op must be probe or transcribe.")
            return self.transcribe(request, lambda event: emit({**event, "id": request_id}))
        except Exception as exc:
            return {"id": request_id, "event": "error", "code": getattr(exc, "code", "inference_failed"),
                    "message": error_message(exc)}


def error_message(exc):
    if isinstance(exc, WorkerError):
        return str(exc)
    message = str(exc)[:2000]
    recovery = ("Close other GPU applications or use a smaller model/CPU and retry."
                if isinstance(exc, MemoryError) or "out of memory" in message.lower()
                else "Check the selected Python runtime and complete local model files, then retry. "
                     "See docs/OPTIONAL_ASR.md; the existing whisper.cpp engine is also available.")
    return f"{type(exc).__name__}: {message}. {recovery}"


def serve(source, output):
    worker = Worker()

    def emit(event):
        output.write(json.dumps(event, ensure_ascii=False, allow_nan=False, separators=(",", ":")) + "\n")
        output.flush()

    while True:
        line = source.readline(MAX_REQUEST_BYTES + 1)
        if not line:
            return
        if len(line.encode("utf-8")) > MAX_REQUEST_BYTES:
            # Drain this frame to preserve framing for the next request.
            while line and not line.endswith("\n"):
                line = source.readline(MAX_REQUEST_BYTES + 1)
            emit({"id": None, "event": "error", "code": "invalid_request", "message": "Request exceeds the 1 MiB protocol limit."})
            continue
        try:
            request = json.loads(line, parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
        except (ValueError, UnicodeError):
            emit({"id": None, "event": "error", "code": "invalid_request", "message": "Expected one valid JSON object per line."})
            continue
        emit(worker.handle(request, emit))


def main():
    configure_offline()
    # Preserve the protocol descriptor, then redirect Python AND native library
    # stdout writes to stderr. A library's print/progress output cannot break JSON.
    protocol = os.fdopen(os.dup(sys.stdout.fileno()), "w", encoding="utf-8", buffering=1)
    os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
    sys.addaudithook(offline_audit)
    try:
        serve(sys.stdin, protocol)
    finally:
        protocol.close()


if __name__ == "__main__":
    main()
