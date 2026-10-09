# Private ASR component builds

These are opt-in, application-managed components, separate from the native
whisper.cpp app release. End users do not install Python, run pip, change PATH,
use an administrator account, or install a compiler. Model downloads are a
separate explicit choice and are never part of engine assembly. Whole-runtime ZIP
publication is retired; the active CI uploads only proof metadata. The local ZIP
format below is retained as a deterministic comparison and security-test fixture.

## Pinned component set

Component version: `1.2.0-r1`. Separate proposed immutable release tag:
`asr-components-1.2.0-r1` in `csic21/luma-subtitle`, published as a prerelease with
`make_latest:false`. This is not an application updater release. The application
release tag, stable latest release, and updater manifest are unchanged.

| Pack | Native platform | Minimum OS | Core packages | Wheel download bytes |
| --- | --- | --- | --- | ---: |
| faster-whisper-cpu-windows-x64 | Windows x64 | Windows 10 | faster-whisper 1.2.1, CTranslate2 4.8.2 | 85,004,906 |
| mlx-whisper-metal-macos-arm64 | Apple Silicon | macOS 14 | mlx-whisper 0.4.3, MLX/MLX Metal 0.29.3, PyTorch 2.9.1 | 198,349,117 |
| qwen3-asr-cpu-windows-x64 | Windows x64 | Windows 10 | qwen-asr 0.0.6, PyTorch 2.9.1 CPU, Transformers 4.57.6 | 363,486,390 |
| qwen3-asr-cpu-macos-arm64 | Apple Silicon | macOS 14 | qwen-asr 0.0.6, PyTorch 2.9.1, Transformers 4.57.6 | 293,589,133 |

The last column is the exact sum of the selected upstream wheel sizes. It is
**not** the final compressed archive size or installed footprint. Native build
manifests record those measured quantities, hashes, and file counts. Runtime
archives add 22,011,023 download bytes on Windows or 25,013,243 on macOS before
repacking. Initial compressed planning estimates are roughly 100–160 MB for
faster-whisper, 200–280 MB for MLX, and 300–450 MB for Qwen, excluding weights.
Do not display estimates as measured available disk space or memory requirements.

Qwen CPU remains experimental and high-memory. This recipe never downloads its
multi-GB ASR/aligner weights. Both weights are needed for timed subtitles, and both
are loaded as FP32 on CPU; the 0.6B pair alone requires approximately 7.5 GB of
weight memory before activations and overhead. A passing import check is not an
inference or memory-fit result. Native whisper.cpp remains the existing GPU route.

MLX's macOS 14 minimum is separate from the base app's lower OS requirement.
Metal availability is checked on the actual machine. Hosted CI without a Metal
device records `metal_available:false, metal_tested:false`; CPU tensor execution
is never reported as Metal validation. No MLX model inference is claimed here.

## Runtime and package provenance

The private interpreter is CPython **3.12.15**, from the official Astral
[python-build-standalone 20261003 release](https://github.com/astral-sh/python-build-standalone/releases/tag/20261003).
`packs.json` pins each install-only-stripped archive's URL, SHA-256 and byte count.
These vendor distributions are [designed to be portable and self-contained](https://github.com/astral-sh/python-build-standalone).
The app uses an absolute path to the interpreter within its component directory.
It does not register Python, create a venv tied to a CI path, or fall back to a
system interpreter.

All package inputs are exact, platform-specific wheels uploaded by their
publishers to PyPI. `scripts/asr-components/locks/<pack>.json` records filename,
version, HTTPS origin, exact bytes and SHA-256. Build jobs consume only those
records. There is no runtime resolver, pip install, moving `latest` URL, source
build, model download, or package repair. The `.txt` and `.in` files document the
maintainer resolution; they are not executed by the app or build workflow.

The separately published official PyPI Windows `torch==2.9.1` wheel has been
inspected: 110,940,568 bytes, SHA-256
`81a285002d7b8cfd3fdf1b98aa8df138d41f1a8334fd9ea37511517cedf43083`.
Its `torch/version.py` reports `2.9.1+cpu`, `cuda=None`, `hip=None`, `xpu=None`,
and its archive contains no CUDA DLL/PYD files. Native smoke must additionally
check the actual imported runtime. This source is distinct from the official
CPU-index wheel with a similar name. The inaccessible `download-r2.pytorch.org`
artifacts are not used, rewritten, proxied, or retried by this recipe.

No CUDA pack is provided. NVIDIA's CUDA/cuDNN redistribution terms, driver
requirements, matching CTranslate2/PyTorch versions and substantially larger
payloads require a separate source/license/size assessment. Installing a CPU pack
does not make CUDA available. No new signing credentials are requested.

## Normalized archive contract

Each release asset is `luma-asr-<pack-id>-1.2.0-r1.zip`. The archive has no enclosing
folder and contains only ordinary files with safe relative paths; upstream
internal symlinks are copied as ordinary files. Entry points are:

- Windows: `python.exe`, with `Lib/site-packages/`
- macOS: `bin/python3`, with `lib/python3.12/site-packages/`

`component.json`, `self_test.py`, `LICENSES.json`, `THIRD_PARTY_NOTICES.txt`, and
runtime license files are included. The application continues to supply its
reviewed `worker.py`, preserving the existing process lifecycle and protocol.
The worker is invoked with `-I -B -u -X utf8` in verification; application launch
uses isolated mode as well. `-B` only suppresses generated bytecode during tests.

No pip/ensurepip, generated console launchers, CI venv paths, bytecode or test logs
are shipped. A wheel's library and runtime data files are retained; CLI launcher
and developer header installation schemes are not used. ZIP entries are sorted,
have fixed timestamps and normalized modes, and use the pinned private Python's
zlib. Native CI builds each component twice into fresh directories and compares
all output bytes. Reproducibility is an observed CI requirement, not an assumed
property. Assets at or above 2,000,000,000 bytes are rejected.

The per-pack manifest contains immutable source SHA, component identity, runtime
entry point, OS minimum, exact installed byte/file counts, and archive URL/bytes/
SHA-256. The manifest is printed with its own SHA-256 in the build log. It becomes
an installer trust input only after approved publication and catalog pinning.

## Licenses and sources

The ZIP preserves the actual notices shipped in each wheel, including notices
outside `.dist-info`, and records their paths and package/source identities in
`LICENSES.json`. Original wheel metadata is preserved. These components contain
multiple licenses; the engine's MIT or Apache license is not a blanket license
for dependencies or bundled native libraries.

Astral install-only archives omit the full distribution's third-party license
set. We extracted `PYTHON.json` and all `python/licenses/*` from each exact full
vendor archive, checked its archive digest, and committed these notices under
`scripts/asr-components/licenses/`. `licenses.lock.json` pins every retained file
and its upstream full-archive source. Every build rechecks these hashes and copies
the notices. This includes CPython and the vendor's OpenSSL, zlib, bzip2, libffi,
SQLite and other native-library notices. The full runtime metadata identifies
linked components and license paths.

PyPI source distributions, where published, are recorded with exact URLs and
hashes. PyTorch, torchvision and torchaudio source revisions are separately
recorded because those binary releases do not supply PyPI source distributions.
The corresponding-source obligations of native dependencies still apply to
redistributors; retaining an inventory does not waive them. In particular review
PyAV/FFmpeg, numerical libraries, PyTorch's bundled notices, and runtime native
libraries before publishing. Do not strip licenses or imply OpenAI/Alibaba/MLX
endorses the combined distribution. No model licenses are included under this
runtime license grant; model cards are preserved with separate model downloads.

### Reviewed supplemental notices and CPU-only trimming

The build supplements the vendor's missing macOS zlib-ng 2.2.4 notice from its
exact official source and preserves the CTranslate2 4.8.2 MIT license, oneDNN
3.1.1 license/third-party notices, and the exact Intel OpenMP 2025.3.0 license and
third-party notices. All supplemental files and their sources are hash-locked;
`licenses/SUPPLEMENTAL-SOURCES.json` explains their origins. The official Intel
PyPI wheel contains a byte-identical CTranslate2 OpenMP DLL, which establishes the
notice provenance but does not by itself establish redistribution clearance.

The upstream CTranslate2 Windows recipe at
`d44d2d069eb88c7b7804da864c10c201501cb4a9` builds with `WITH_CUDNN=OFF` yet copies
`cudnn64_9.dll` into the wheel. `pruning.json` now omits that exact unused 266,288-byte
file from the CPU-only component. The builder pins the CTranslate2 DLL identity,
parses its normal PE imports, rejects any CUDA/cuDNN import and any unreviewed
delay-import directory, and checks the omitted file's exact bytes/hash before
removing it. `INSTALLATION_CHANGES.json` records this operation. Native CPU
imports and real Tiny cold/warm inference must pass again after this change.
The Intel OpenMP DLL remains a direct dependency and is not removed or replaced.

Intel's exact redist designation and the applicability of nested notices remain
unresolved. Preserving those notices is **not** a blanket declaration that the
DLL is open source or freely redistributable under the app's GPL. The Qwen packs'
python-soxr and soundfile bundled native libraries also require corresponding
source/notices review. These are publication holds alongside PyAV below.

### Publication blocker: PyAV bundled libraries

The selected PyAV 19.0.1 Windows wheel was inspected. Its `av.libs/` includes
FFmpeg DLLs, libx264, libx265, libiconv and GCC runtime libraries, while its included
license directory contains only PyAV's BSD notice and authors. Those top-level
metadata do **not** describe all bundled native-library terms. The publisher must
supply the exact dependent licenses and corresponding-source fulfillment before
publishing affected packs; source package links for PyAV alone do not resolve this.
See the [upstream wheel-license report](https://github.com/PyAV-Org/PyAV/issues/2270)
and [PyAV's FFmpeg build project](https://github.com/PyAV-Org/pyav-ffmpeg).
This is a release blocker, not a runtime import failure. Native build/test work may
continue, but current reports are not redistribution clearance.

## Build and verification

The read-only `.github/workflows/asr-components.yml` runs on native Windows x64
and macOS arm64 runners. PR jobs check out `pull_request.head.sha` explicitly.
It is also callable with an exact `source_sha` and component `tag`. Every job is
named `Build component <pack-id>`. The primary proof assembles the exact inputs
twice with the shared offline installer, compares output bytes, tests relocation
and imports, and then runs the genuine Rust direct-recipe lifecycle test for
install, repair, cancellation rollback and removal. The legacy whole-runtime ZIP
reference build and its duplicate installer run are no longer active CI steps.
Archive/security unit tests remain required, alongside final aggregate native
verification for the exact candidate source.

Each job uploads only `asr-offline-pip-proof-<pack-id>`, containing
`<pack-id>.offline-pip-proof.json`. No runtime binaries, model weights or input
caches are uploaded. A missing recipe or failed native dependency check is not
reported as a successful end-user installation.

The build host's Python handles build orchestration only. The test extracts the
ZIP, verifies actual uncompressed bytes/counts, moves it to another directory with
spaces and non-ASCII characters, and launches its private interpreter. HOME and
caches are clean; PATH has no executables; PYTHONHOME and PYTHONPATH are poisoned;
user site is disabled. `sys.prefix`, all Python search paths, imported package
paths, and actually loaded native libraries must remain inside the component or
OS-owned library directories. The self-test blocks Python network and subprocess
audit events before importing ML packages. This is defense in depth, not an OS
sandbox for arbitrary native code.

Evidence explicitly distinguishes:

- Private runtime import/API checks and tiny tensor operations
- Offline JSON-lines path rejection, clean EOF shutdown, idle termination/reaping,
  and a fresh-worker recovery request (not Qwen inference cancellation)
- Actual faster-whisper Tiny CPU cold/warm inference with a pinned public
  11-second JFK fixture, checking text, timestamp validity and warm reuse
- MLX device/tensor capability, with explicit Metal booleans; no model inference
- Qwen imports/API only; no ASR/aligner weights, inference, memory fit or speed claim

The MLX Tiny fixture metadata read was paused; this workflow intentionally does
not fetch it, substitute Turbo weights, or report a model smoke. The existing
reviewed SYSTRAN Tiny fixture manifest and a pinned whisper.cpp JFK sample are
used only by the faster-whisper CI test, never bundled into user engine archives.

Run dependency-free packaging/harness tests locally:

```sh
python -B -m unittest discover -s scripts/asr-components -p 'test_*.py' -v
```

Native verification remains required before publishing. No macOS Developer ID
signature/notarization or Windows SmartScreen reputation is claimed. The build
never removes quarantine, bypasses Gatekeeper/SmartScreen, changes PATH or requires
administrator permissions. A native CI import pass does not establish first-launch
behavior on every clean user machine.

## Publication boundary

Whole-runtime binary publication is retired. The historical release/controller
contract below is not an active delivery route; any future reviewed CPU-only
dependency publication requires its own explicit approval and source closure.

The build workflow has read-only repository permissions and never uploads release
assets, creates tags, modifies the app release, or updates the application catalog.
A separately reviewed publication controller must verify the exact successful PR
source/run, approved archive and manifest hashes, and rebuilt evidence before any
release mutation. Published component tags/assets must be immutable; mismatches
must fail rather than overwrite. A failed or untested pack remains unavailable.

## Direct-upstream assembly proof (activation requires final review)

The separate `assemble.py` entrypoint is shared with the planned application
installer. It is install-time assembly only; transcription never invokes pip.
The verified downloader owns every network request. The bootstrap uses the exact
private PBS interpreter and its bundled **pip 26.2.1**, launched with
`-I -S -B -u -X utf8`. It manually adds only that interpreter's known private site
directory, without processing startup `.pth` or site customization hooks.

Every input wheel is rechecked for exact bytes/hash, and the wheelhouse must
contain exactly the approved files. Requirements are direct local file URLs with
one exact SHA-256 per wheel. Pip runs with no index, dependency resolution, source
builds, bytecode compilation or cache. The bootstrap clears inherited environment
and all `PIP_*` options, sets `PIP_CONFIG_FILE` to the platform's null device, and
asserts that the pinned pip loads no global/user/site configuration even in
isolated mode. An audit guard is installed before pip imports and denies network
and child-process operations. The caller owns the only child process, imposes a
timeout and kills/reaps it on cancellation or error.

Installation uses the fresh private interpreter's explicit `--prefix`, preserving
standard wheel `.data/data` locations under the runtime root. The complete old
bootstrap setuptools package is removed before replacement. Generated launchers are
removed. Generated local `direct_url.json` records are verified against their
approved wheel identity and removed; the exact original upstream URLs/hashes are
preserved in deterministic `ASSEMBLY.json`, without CI or user filesystem paths.
Only RECORD rows for deliberately removed artifacts are dropped, so launcher and
local-URL hashes cannot retain staging-dependent content. The reviewed setuptools
startup `.pth` is also removed; startup hooks are not needed by transcription.
The native proof assembles twice and compares bytes, then runs the same relocated
private runtime and functional smoke used by the reference build. Only the JSON
proof report is uploaded. Until those native checks pass, this is an implemented
proof path, not verified end-user installation support.

The eventual UI must show exact applicable proprietary terms before a user's
explicit install click. Consent cannot authorize a use excluded by a vendor's
license. In particular, the original CUDA/cuDNN-containing CTranslate2 wheel is
not a direct-source CPU delivery candidate; a separately verified CPU-only build
is under consideration. Proprietary/native redistribution and source-closure
holds still prohibit shipping the reference ZIPs.

`measure_recipe_caps.py` reads only the already verified local archive cache.
`recipe-caps.json` labels each input count as measured or a conservative enforced
cap. It does not fetch missing archives. The two Qwen candidates use an independent
4 GiB/100,000-file final-tree limit; that is not a measured installed size.

`generate_recipes.py` combines those bounds with exact Python notices, upstream
engine license texts, declared dependency inventory and applicable proprietary
terms into an explicitly named review-only output. CI generates it twice under
`RUNNER_TEMP`, compares exact bytes, logs its SHA-256 and passes that path to the
native proof. The redundant generated text is not checked into the repository.
The generator never modifies the production catalog and
preserves an unavailable reason for every candidate. The original Windows
CTranslate2 wheel is excluded entirely. Engine text provenance and matching source
version evidence are locked in `engine-terms.lock.json`; this does not establish
complete native dependency license coverage or validate runtime operation.

Windows smoke rejects numbered optional Visual C++ runtime DLLs loaded outside
the private component, even if the hosted runner has installed them in System32.
Guaranteed Windows OS libraries such as `msvcp_win.dll` are classified separately.
The pinned PBS package does not itself supply every optional DLL used by the
upstream engine wheels; clean-user readiness remains blocked until app-local
dependencies and their exact upstream terms are reviewed and tested.


## Managed Windows Nagisa initialization

The reviewed Qwen dependency pins Nagisa 0.2.11 and DyNet38 2.2. Its default
initializer passes an absolute model path to a native narrow-string file API,
which fails in a Unicode Windows installation directory. The app-owned helper
`src-tauri/src/asr/nagisa_compat.py` is embedded from one source into the managed
worker and installer self-test; it is never loaded from a user-selected path.
Manual Python environments are unchanged.

Before any worker activity threads, the helper checks the exact private package
versions, bounded metadata, every retained RECORD file and expected module origins.
During official package initialization only, it supplies the supported relative
`Tagger` file arguments for one default construction from Python's Unicode-aware
working directory. It restores the original constructor, import loader and cwd in
`finally`; a partial failure is fatal to that worker process. Wheel bytes and the
upstream public API remain unchanged.

Native Windows proof must compare actual Japanese tokens and POS tags from an
ordinary ASCII-path initialization with the adapted Unicode-path initialization,
and prove restoration after a deliberate post-import error. This check uses only
Nagisa's already bundled data. It does not establish Qwen ASR inference, memory
fit, or closure of the separate Windows app-local CRT requirement.
