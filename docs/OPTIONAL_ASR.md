# Optional local transcription engines

The default `whisper.cpp` engine and existing GGML models continue to work without
Python. These additional engines are opt-in. Luma downloads verified private
engine components and models only after an explicit install/download action.
The managed setup design downloads pinned Python and wheels directly from their
original upstream publishers, plus a separately reviewed Luma CPU-only
CTranslate2 wheel for Windows Whisper. It runs its private, version-pinned pip
entirely offline inside staging during Install or Repair, never transcription.
There is no network dependency resolution or source build. You do not install Python, enter pip commands, or
change the system environment. Luma does not read Hugging Face credentials or
silently switch engines. Whole Python/runtime bundles are not republished by Luma.

**Availability (2026-10-09):** the current embedded catalog still marks the four
managed runtime candidates unavailable pending final native evidence, exact pins
and release review. The steps below describe the supported managed setup flow
for an enabled, verified recipe; they do not make a pending component available.
The Windows CPU wheel has no final published size or verified download yet.
See [the build and publication boundaries](ASR_COMPONENTS_BUILD.md).

## One-click managed setup

1. Select an optional engine in Settings or task configuration. The original
   whisper.cpp engine and its Turbo preset remain available without this setup.
2. Select an available compatible component, review download sizes, labelled
   installed-size bounds, exact sources and applicable third-party terms, then
   choose Install. The private runtime is kept in Luma's own data directory; you
   do not need to install Python or enter terminal commands.
3. Download a compatible model separately. Qwen needs both its ASR model and the
   distinct ForcedAligner model. Choose the installed components for this task and
   run the capability check before processing your media.

The initial managed targets are Windows x64 Whisper CPU, Apple Silicon MLX
Whisper/Metal, and Qwen CPU for either platform. They do not include CUDA. Device
and minimum macOS-version checks can reject an incompatible component before
installation. A GPU being present does not make an unsupported backend ready.

Downloads expose progress and cancellation. Complete bytes and SHA-256 values
are pinned in the application catalog, not accepted from an untrusted remote
manifest. Verified inputs are unpacked and assembled privately in staging,
checked and self-tested, then activated atomically. Failed installation keeps
the previous working component.
Repair verifies/reinstalls the selected owned component; removal affects only
Luma-managed copies, not existing external models. Cancel or finish transcription
before replacing/removing components in use. Old component versions are retained
rather than silently deleted while saved tasks may still reference their paths.

No installer requires admin access or changes Gatekeeper/SmartScreen policies.
If the operating system blocks a component, Luma reports the original error;
it does not remove quarantine attributes or bypass a warning. Updater integrity
signatures are distinct from Apple Developer ID signing/notarization.

Windows components obtain the exact Microsoft runtime package directly from
Microsoft after its English and Chinese terms are displayed for acceptance.
Only the verified required DLLs and original notices are extracted into the
private runtime; the downloaded installer is never executed. Install and Repair
use normal Windows certificate-chain/revocation validation in a bounded,
cancellable helper. These checks can require internet access even when all
download bytes are cached. The private pip step remains offline; completely
offline repair is not promised. Failed trust checks do not replace the current
component or change Windows security settings.

Advanced external-runtime paths remain available for existing configurations.
The manual commands below are optional advanced examples, not the standard setup.
Luma does not install, update or repair those external Python/model paths. Managed
Repair applies only to Luma-owned components and keeps its normal verification
and user-assent requirements.

## Choose an engine

| Engine | Device | Local model format |
| --- | --- | --- |
| Whisper accelerated | Apple Silicon `auto` / `metal` | MLX Whisper conversion |
| Whisper accelerated | `cpu`, or Windows/Linux `auto` / `cuda` | CTranslate2 faster-whisper conversion |
| Qwen3-ASR | `cpu` / `cuda` / `auto` | Original qwen-asr ASR model **and** original Qwen3-ForcedAligner |

This table describes adapter capabilities, including advanced external runtimes.
It does not mean managed Linux/CUDA packs are provided; the managed targets are
the CPU/Apple Silicon entries described above.

Automatic selection checks the selected runtime's actual device availability.
Windows alone does not imply CUDA. Explicit unavailable devices fail with a
recovery message. Qwen Metal/MPS, ROCm acceleration, vLLM, FlashAttention setup,
and native Transformers `-hf` model exports are outside this initial adapter.
Qwen on macOS uses CPU and can be slow. On Apple Silicon, explicit Whisper CPU
uses faster-whisper and therefore needs its separate CTranslate2 conversion.

The worker keeps one model configuration warm until a later check/task uses a
changed configuration, you release its memory, start legacy transcription,
cancel, or exit the application. Merely editing settings fields does not unload
it. Changing the language does not reload weights. Results report the loaded device, cold-load seconds, inference
seconds, and whether the model was reused. These are measurements, not speedup
claims. RAM/VRAM remains occupied while that worker stays warm.

## Advanced external-runtime setup examples

The commands below are **user-run setup**, outside Luma. They install software or
download weights only when you choose to run them. Dependency-free unit tests use
fixtures; separate native tests execute the pinned private setup recipe. Use
separate environments to avoid dependency conflicts. Python 3.12 is a practical
starting point. You maintain these external environments yourself; managed Repair
does not modify them.

The adapter was checked against the upstream APIs of `mlx-whisper==0.4.3`,
`faster-whisper==1.2.1`, and `qwen-asr==0.0.6`; actual hardware inference with these
packages still requires validation on your machine. For these external examples,
transitive dependencies are not a reproducible lockfile. Managed components
instead use fixed, hash-pinned wheel/runtime locks. Keep an external working
environment stable after testing.

### Apple Silicon: MLX Whisper

Use an arm64 Python, not a Rosetta x86 interpreter:

```sh
python3.12 -m venv "$HOME/.venvs/luma-mlx"
"$HOME/.venvs/luma-mlx/bin/python" -m pip install "mlx-whisper==0.4.3" huggingface_hub
"$HOME/.venvs/luma-mlx/bin/hf" download mlx-community/whisper-turbo --local-dir "$HOME/Models/luma/whisper-turbo-mlx"
```

In Luma's ASR settings, choose Whisper accelerated, select that environment's
absolute `bin/python` path and the downloaded model directory, and choose `auto`
or `metal`. Keep the conversion's `config.json` and `weights.safetensors` or
`weights.npz` together. The pinned `mlx-whisper==0.4.3` release does **not** load
`model.safetensors`-only directories, even though newer upstream source supports
that filename. For that readiness error, select a compatible conversion such as
the documented `mlx-community/whisper-turbo` snapshot (`weights.npz`). Luma does
not rename, convert, or otherwise modify your model files.

### Windows/Linux: faster-whisper

Linux example:

```sh
python3.12 -m venv "$HOME/.venvs/luma-faster-whisper"
"$HOME/.venvs/luma-faster-whisper/bin/python" -m pip install "faster-whisper==1.2.1" huggingface_hub
"$HOME/.venvs/luma-faster-whisper/bin/hf" download dropbox-dash/faster-whisper-large-v3-turbo --local-dir "$HOME/Models/luma/faster-whisper-turbo"
```

For Windows CPU, prefer Luma's managed component once its verified recipe becomes
available. Its CPU-only CTranslate2 build excludes CUDA/cuDNN and Intel OpenMP/MKL.
The reviewed Windows `4.8.2/1lumacpu` recipe uses `cpu_threads=1` for reliable
model cleanup. CPU transcription may be slower; this is not an acceleration or
throughput promise. The policy is selected only for its active verified managed
interpreter and exact recipe. External/manual Python runtimes, native Whisper,
MLX and Qwen keep their existing behavior.
Final native proof, publication and catalog activation are still required; this
guide does not supply a ready CPU-wheel download. The general upstream Windows
CTranslate2 wheel can include GPU libraries and additional terms even when CPU
execution is selected; it is not the managed CPU recipe. Existing external
runtimes remain usable through advanced paths, but their dependencies and terms
must be checked separately.

This Turbo conversion is maintained by Dropbox (formerly the `mobiuslabsgmbh`
repository), **not an official OpenAI model export**. It is the conversion named
by faster-whisper's own `turbo` model alias; the old owner URL currently redirects
to `dropbox-dash`. Review the model source before downloading. For a smaller
SYSTRAN-maintained alternative, substitute `Systran/faster-whisper-small` and a
separate local `faster-whisper-small` directory in the download command.

Choose the absolute Python executable and local model directory, then `auto` or
`cpu`. CUDA additionally needs compatible NVIDIA drivers and the CUDA/cuDNN
libraries required by your CTranslate2 build. Follow the
[official faster-whisper GPU requirements](https://github.com/SYSTRAN/faster-whisper#gpu)
rather than assuming a generic PyTorch CUDA installation configures CTranslate2.

Keep `config.json`, `model.bin`, `tokenizer.json`, and `vocabulary.json` or
`vocabulary.txt`. Include `preprocessor_config.json` when supplied by the model
(notably large-v3); older official small conversions omit it and use defaults.
The tokenizer is mandatory to prevent upstream's Hub fallback. GGML `.bin` files
from whisper.cpp cannot be used as CTranslate2 models despite the same extension.

### Qwen3-ASR with forced alignment

Use a separate environment. First choose CPU or the correct NVIDIA CUDA PyTorch
build using the [official PyTorch selector](https://pytorch.org/get-started/locally/).
Install it into this same environment before the qwen-asr package. The following
is a Linux/macOS **CPU** example; CUDA users should substitute the selector's
PyTorch command. macOS has no CUDA option.

```sh
python3.12 -m venv "$HOME/.venvs/luma-qwen"
# Linux CPU PyTorch (macOS: use torch==2.9.1 from PyPI instead):
"$HOME/.venvs/luma-qwen/bin/python" -m pip install "torch==2.9.1+cpu" --index-url https://download.pytorch.org/whl/cpu
"$HOME/.venvs/luma-qwen/bin/python" -m pip install "qwen-asr==0.0.6" huggingface_hub
"$HOME/.venvs/luma-qwen/bin/hf" download Qwen/Qwen3-ASR-0.6B --local-dir "$HOME/Models/luma/Qwen3-ASR-0.6B"
"$HOME/.venvs/luma-qwen/bin/hf" download Qwen/Qwen3-ForcedAligner-0.6B --local-dir "$HOME/Models/luma/Qwen3-ForcedAligner-0.6B"
```

For the larger ASR model, download this instead of 0.6B and select its directory.
It uses the **same separate** ForcedAligner-0.6B directory:

```sh
"$HOME/.venvs/luma-qwen/bin/hf" download Qwen/Qwen3-ASR-1.7B --local-dir "$HOME/Models/luma/Qwen3-ASR-1.7B"
```

On Windows create the venv with `py -3.12 -m venv`, then run its
`Scripts\python.exe` / `Scripts\hf.exe` as in the faster-whisper example. Use the
Windows CPU/CUDA command from the PyTorch selector. Upstream qwen-asr's Japanese
tokenizer dependency may need platform-specific build support if no wheel exists.
If a compatible external environment cannot be installed, use an available
managed component or native whisper.cpp instead. Luma does not install or repair
this user-created environment. This does not limit Install/Repair for enabled
Luma-managed components.

Select Qwen3-ASR, the absolute Python executable, the ASR directory, and the
**separate forced-aligner directory**. Download full snapshots with tokenizer,
preprocessor, chat template, safetensors weights and every indexed shard. Luma
requires safetensors for Qwen and disables remote model code. The original model
names above are intentional: native Transformers `-hf` exports use a different API.

Qwen timed subtitles support these forced-aligner languages: Chinese (`zh`),
English (`en`), Cantonese (`yue`), French (`fr`), German (`de`), Italian (`it`),
Japanese (`ja`), Korean (`ko`), Portuguese (`pt`), Russian (`ru`), Spanish (`es`).
Qwen's ASR supports more languages, but that does **not** make them supported for
timed subtitles. Unsupported explicit or detected languages fail rather than
receiving invented timestamps.

## Storage and memory planning

Approximate model storage from the upstream snapshots, before Python packages,
package/download caches, temporary audio, or outputs:

| Example | Approximate model files |
| --- | --- |
| faster-whisper small | 0.49 GB |
| faster-whisper large-v3-turbo conversion | 1.6 GB |
| faster-whisper large-v3 | 3.1 GB |
| MLX whisper-turbo | 1.6 GB |
| Qwen3-ASR-0.6B + ForcedAligner-0.6B | 3.7 GB |
| Qwen3-ASR-1.7B + ForcedAligner-0.6B | 6.5 GB |

These decimal-GB estimates are planning examples, not measured free disk space
or RAM/VRAM requirements. Runtime dependencies can add several GB, particularly
CUDA/PyTorch; model/cache copies can add more. Leave substantial headroom.
Inference RAM/VRAM depends on dtype, audio length, and runtime overhead. CPU Qwen
loads **both ASR and forced-aligner weights concurrently as float32**, so RAM
use can considerably exceed the snapshot size. For the published BF16 snapshots,
the 0.6B pair is roughly 7.5 GB of loaded weights alone (about twice its 3.7 GB
storage), and the 1.7B pair is roughly 13 GB, before Python/PyTorch, activations,
audio, and operating-system memory. These are dtype-and-pair-specific planning
figures, not guarantees. **Do not treat Qwen CPU as safe on a typical 8 GB device.**
The CPU probe estimates weight-only FP32 bytes from both local safetensors headers
when available and always warns that it has not established actual memory fit.

Luma's readiness check reports **measured bytes of required files currently
present** in the selected model/aligner directories. It does not measure free
disk space or predict peak memory. It validates paths, expected metadata/files,
all indexed shards, runtime imports, and device availability without loading model
weights. A passing probe is not proof of successful inference or a memory fit.
It detects missing/empty files and Git LFS pointers, but does not checksum every
weight or detect every form of corruption.

## Offline behavior and subtitle quality safeguards

- Worker requests accept absolute local paths only, never model IDs or URLs.
- HF/Transformers offline and no-telemetry flags are forced before imports;
  implicit Hub credentials and remote model code are disabled.
- Python network and child-process audit events are blocked. This is defense in
  depth, not an operating-system sandbox for arbitrary native dependencies.
  Install only runtimes/model files you trust.
- Prepared mono PCM16 16 kHz WAV is decoded with `wave` and NumPy. Arrays are
  passed to backends, so the worker does not launch ffmpeg or decoder subprocesses.
  Existing Rust-owned audio preparation remains responsible for ffmpeg.
- Qwen uses the official Transformers wrapper with local forced alignment,
  batch size one and no vLLM/multiprocessing. GPU dtype is BF16 when available,
  otherwise FP16; CPU is FP32.
- Timings must be finite, ordered, non-overlapping, positive-length, and inside
  audio duration (only 1 ms rounding tolerance). Invalid or missing timestamps
  fail; there is no equal-duration chunk fallback.
- Qwen aligner tokens are matched against the **entire** original transcript.
  Dropped, extra, or reordered speech text fails. Original transcript slices keep
  punctuation, accents, whitespace and CJK text. Cues group real aligned spans
  around sentence boundaries, pauses, approximately 6 seconds, and character
  budgets (84 Latin / 36 wide-character text). A single indivisible long token
  can exceed those grouping targets; it is not assigned invented sub-timings.
- faster-whisper progress derives from completed segment timestamps divided by
  audio duration. MLX/Qwen expose stage updates and 15-second liveness messages
  without fabricated percentages. Completion timing includes alignment and cue
  validation. Cancellation discards the worker and its warm model.

## Validation boundary (2026-10-09)

- **Windows CPU candidate proof failed:** [run 37943815924](https://github.com/csic21/luma-subtitle/actions/runs/37943815924)
  produced two identical 25,058,630-byte wheels from fresh builds at the same
  fixed native path, and static closure passed for 156 PE files/1,039 imports.
  The native verifier then timed out after 900 seconds without stage evidence;
  imports, model loading, inference and the final loaded-module inventory cannot
  yet be localized or claimed successful. The failed candidate's exact hash is
  recorded in [the build guide](ASR_COMPONENTS_BUILD.md); these bytes are not a
  final installed-runtime size or approved catalog pin. Publication remains
  blocked. Fixed-path repeatability is not path-independent or cross-machine
  reproducibility, and the Linux result below does not validate this Windows wheel.
- **Native Apple Silicon setup proof passed:** MLX and Qwen private offline
  installation reproduced exactly, relocated imports passed, and the actual
  Rust installer exercised install, repair, cancellation and removal. The MLX
  runner also executed a real Metal tensor. These checks do not perform Whisper
  or Qwen speech-model inference and are not speed or quality benchmarks.
- **Real Linux CPU functional smoke passed:** Python 3.12.14,
  faster-whisper 1.2.1, CTranslate2 4.8.2, and the official
  `Systran/faster-whisper-tiny` snapshot
  `d90ca5fe260221311c53c58e660288d3deb8d356` processed the public
  [11-second JFK sample](https://github.com/ggml-org/whisper.cpp/blob/master/samples/jfk.wav).
  The actual Rust process manager exercised setup probe, cold transcription,
  warm reuse, the shared timestamp validator/SRT parser/writer, export
  round-trip, cancellation during inference, and cold-worker recovery.
- **Real Qwen dependency/API checks passed:** qwen-asr 0.0.6,
  Transformers 4.57.6 and PyTorch 2.9.1+cpu imported under the worker's offline
  guard. Actual wrapper signatures, devices and alignment dataclasses were
  checked without loading model weights. Metadata-only probe fixtures do not
  prove Qwen model readiness or inference success.
- **Not established:** Qwen inference/memory fit, Metal/CUDA inference,
  representative transcription accuracy, speed or peak-memory comparisons,
  and a complete native desktop UI/queue smoke. These engines remain
  experimental and opt-in. The small Linux functional smoke is not a benchmark.

The real worker test is opt-in and never installs or downloads anything. After
preparing a trusted Python environment, local CTranslate2 model, the public WAV
and a longer repeated WAV for cancellation, set these absolute paths:

```sh
export LUMA_ASR_TEST_PYTHON=/absolute/venv/bin/python
export LUMA_ASR_TEST_MODEL=/absolute/faster-whisper-tiny
export LUMA_ASR_TEST_AUDIO=/absolute/jfk.wav
export LUMA_ASR_TEST_LONG_AUDIO=/absolute/jfk-repeat.wav
export LUMA_ASR_TEST_OUTPUT=/absolute/isolated-test-output
cargo test --manifest-path src-tauri/Cargo.toml --locked \
  real_optional_worker_transcribes_exports_reuses_and_cancels -- --ignored --nocapture
```

The app regression jobs use dependency-free adapter fixtures and real small
Python subprocess lifecycle tests on Windows/macOS. Separate engine-proof jobs
download exact pinned runtime inputs and run private offline installation;
model inference is separately identified and resource-gated.

## JSON-lines protocol and verification

Input: one object per line, up to 1 MiB, with string/integer `id`, `op` (`probe` or
`transcribe`), `engine`, `model_path`, `aligner_path`, and `device`. Transcription
also requires `audio_path`; `language` is an ISO code or `auto`. `python_path` is
owned by the Rust launcher and is not part of the worker request.

Events always include `id`:

- `progress`: `message`, optional `progress` fraction 0–1, `backend`, `device`.
- `probe`: `ready`, `backend`, `device`, `model_bytes`, `aligner_bytes`,
  `total_bytes`, `capabilities`, `warnings`; failures add `code` and `error`.
  Unsupported/invalid configurations can leave backend/device null and bytes 0.
- `result`: `segments` with integral `start_ms`, `end_ms`, `text`; `backend`,
  `device`, `reused`, numeric `load_seconds`, numeric `inference_seconds`.
- `error`: `code` and actionable `message`. Recoverable request errors leave the
  worker available; an irrecoverable partial runtime initialization discards the
  process. Cancellation/shutdown is handled by the Rust parent.

Only JSON protocol records use stdout; Python and native library diagnostics go
to stderr. Run adapter/protocol tests without optional dependencies:

```sh
python3 -B scripts/test_asr_worker.py
```

Tests use fake optional packages and tiny local metadata/PCM fixtures. They cover
device selection, warm reuse, probe-without-load, local-only API arguments,
complete alignment coverage, CJK punctuation, invalid times, and protocol error
recovery. They do not claim tested model accuracy, real hardware speed, or peak
memory. Before relying on a new backend, run a short known speech sample twice,
verify cold/warm timing and device reporting, inspect subtitle text/timestamps,
and cancel a longer sample to verify cleanup on your platform.

Upstream references: [Qwen package API](https://github.com/QwenLM/Qwen3-ASR),
[Qwen forced-aligner implementation](https://github.com/QwenLM/Qwen3-ASR/blob/main/qwen_asr/inference/qwen3_forced_aligner.py),
[MLX Whisper](https://github.com/ml-explore/mlx-examples/tree/main/whisper),
[faster-whisper](https://github.com/SYSTRAN/faster-whisper),
[Qwen 0.6B files](https://huggingface.co/Qwen/Qwen3-ASR-0.6B/tree/main),
[Qwen 1.7B files](https://huggingface.co/Qwen/Qwen3-ASR-1.7B/tree/main),
[aligner files](https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B/tree/main),
[Whisper small files](https://huggingface.co/Systran/faster-whisper-small/tree/main),
[Whisper large-v3 files](https://huggingface.co/Systran/faster-whisper-large-v3/tree/main),
[faster-whisper's model aliases](https://github.com/SYSTRAN/faster-whisper/blob/master/faster_whisper/utils.py),
[Turbo conversion files](https://huggingface.co/dropbox-dash/faster-whisper-large-v3-turbo/tree/main),
[MLX turbo files](https://huggingface.co/mlx-community/whisper-turbo/tree/main).
