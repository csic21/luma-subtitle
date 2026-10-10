# Managed ASR setup and CPU-wheel builds

Optional ASR setup is separate from the native whisper.cpp application release.
The shipping design is **direct upstream download, followed by private offline
assembly on the user's machine**. Luma does not publish assembled Python/runtime
ZIPs. The only separate binary publication is Luma's narrow
CPU-only CTranslate2 wheel, with its exact source, notices and proof metadata.

Users explicitly review the selected component's sources, sizes and applicable
terms before Install or Repair. They do not install system Python, run terminal
pip commands, change PATH, use an administrator account or install a compiler.
Luma's pinned private pip runs only during managed setup, never transcription.
ASR/forced-aligner model weights are separate, explicit downloads and are never
part of engine assembly or public component artifacts. An upstream package's
bundled tokenizer data, such as Nagisa's, is part of that package's locked input.

**Activation status (2026-10-10):** the activation catalog enables all four
optional experimental recipes. Luma's CPU-only CT2 wheel, sources, notices and
proof are published at the versioned
[`asr-ct2-cpu-4.8.2-1` release](https://github.com/csic21/luma-subtitle/releases/tag/asr-ct2-cpu-4.8.2-1)
and their public bytes match the reviewed artifacts. The shipping lock pins every
input by size and SHA-256; publication follows a no-overwrite policy, not a claim
of platform-enforced release immutability. Source `370c1e3` passed the CPU source
proof and all three other native setup jobs. Final activation-source native
assembly, receipt/lease lifecycle, app checks and release review remain gates
before merge and application release. No final-source native pass or complete
speech-model coverage is implied by these local catalog changes.

## Current component targets and size boundaries

The managed recipe version is `1.2.0-r1`. This identifies private setup recipes,
not an active whole-runtime release tag.

| Target | Native platform | Minimum OS | Core packages | Locked upstream wheel bytes only |
| --- | --- | --- | --- | ---: |
| faster-whisper-cpu-windows-x64 | Windows x64 | Windows 10 | faster-whisper 1.2.1, Luma CPU CTranslate2 4.8.2 | 90,841,467 |
| mlx-whisper-metal-macos-arm64 | Apple Silicon | macOS 14 | mlx-whisper 0.4.3, MLX/MLX Metal 0.29.3, PyTorch 2.9.1 | 198,349,117 |
| qwen3-asr-cpu-windows-x64 | Windows x64 | Windows 10 | qwen-asr 0.0.6, PyTorch 2.9.1 CPU, Transformers 4.57.6 | 363,486,390 |
| qwen3-asr-cpu-macos-arm64 | Apple Silicon | macOS 14 | qwen-asr 0.0.6, PyTorch 2.9.1, Transformers 4.57.6 | 293,589,133 |

The numeric column sums the exact selected upstream wheel sizes in the existing
locks. It excludes Python, the Windows CRT prerequisite, model weights, temporary
staging/cache copies and installed expansion. Python adds 22,011,023 download
bytes on Windows or 25,013,243 on macOS. The fixed direct-Microsoft package adds
25,635,768 bytes where required. These are individual input sizes, **not** final
installed totals, free-space measurements, compressed runtime assets or memory
requirements. The Windows Whisper total including Python and the direct Microsoft package is
138,488,258 bytes. The rejected vendor CT2 wheel is not a recipe input.

`measure_recipe_caps.py` reads only already verified local archives; it does not
fetch missing inputs. `recipe-caps.json` labels measured archive member counts
separately from conservative enforced extraction/final-tree caps. The two Qwen
candidates use a 4 GiB/100,000-file final-tree ceiling; this is not a measured
installed footprint. The published CPU wheel was size/hash checked and measured without execution:
43 regular members, 25,051,858 expanded bytes and two native binaries. Unchanged
Python/21-wheel measurements are inherited from the reviewed prior caps with
identical locked identities, not freshly measured in the activation workspace.
The CPU archive-input aggregate is 272,310,667 bytes / 5,477 files. Its final-tree
cap is 277,111,469 bytes / 5,909 files, including conservative assembly allowances.
The previous aggregate replacement-RECORD allowance is retained with its surplus,
and the new wheel adds a measured 25,498-byte RECORD allowance. The JSON records
this provenance explicitly; final caps are not actual installed usage.

Qwen CPU remains experimental and high-memory. Both ASR and aligner weights are
loaded as FP32 on CPU; the 0.6B pair alone needs approximately 7.5 GB of weight
memory before activations and overhead. Imports do not establish inference or
memory fit. Native whisper.cpp remains the existing GPU route. MLX's macOS 14
minimum is separate from the base app's lower OS requirement; a real Metal device
must be checked on the target machine.

## Input provenance and trust

The private interpreter is CPython **3.12.15** from the official Astral
[python-build-standalone 20261003 release](https://github.com/astral-sh/python-build-standalone/releases/tag/20261003).
`packs.json` pins its install-only-stripped URL, SHA-256 and byte count. The app
uses an absolute private interpreter path, with no system-Python fallback or
registration and no venv tied to a CI path.

For upstream packages, `scripts/asr-components/locks/<pack>.json` records each
publisher's exact platform wheel, version, HTTPS origin, bytes and SHA-256.
Python comes directly from Astral, wheels such as PyAV and PyTorch directly from
their original publisher distribution, and the CRT package directly from
Microsoft. Luma does not mirror, proxy or republish these runtime inputs. The
exception is the separately reviewed Luma CPU CT2 wheel, whose immutable URL,
bytes and hash are pinned after its own guarded publication. The original
Windows CT2 wheel is excluded from the current managed recipe.

The verified application downloader owns all artifact downloads. Compiled catalog
pins and reviewed recipes are the trust inputs; an untrusted remote manifest
cannot replace them. There is no network dependency resolution, source build or
moving `latest` URL during setup. The `.txt`/`.in` maintainer resolution files are
not executed by the app. Model downloads remain separate and preserve model cards.

The official PyPI Windows `torch==2.9.1` input is 110,940,568 bytes, SHA-256
`81a285002d7b8cfd3fdf1b98aa8df138d41f1a8334fd9ea37511517cedf43083`.
Its inspected metadata reports `2.9.1+cpu`, `cuda=None`, `hip=None`, `xpu=None`,
and its archive has no CUDA DLL/PYD files. Native checks must still verify the
actual imported runtime. This is distinct from the similarly named CPU-index
wheel; inaccessible `download-r2.pytorch.org` artifacts are not substituted,
proxied or used by this recipe.

No managed CUDA pack is provided. CUDA/cuDNN terms, drivers, matching native
versions and payload sizes need separate review. Selecting CPU execution does
not make a GPU-containing wheel an acceptable CPU delivery input. User assent
cannot authorize a use excluded by a vendor's terms.

## Private offline assembly, activation and Repair

`assemble.py` is the shared install-time entrypoint for CI and the application.
It uses the exact private PBS interpreter and bundled **pip 26.2.1**, launched
with `-I -S -B -u -X utf8`. It adds only the known private site directory without
processing startup `.pth` files or site-customization hooks.

Every wheel is rechecked for exact bytes/hash, and the wheelhouse must contain
exactly the approved files. Requirements contain direct local file URLs with one
exact SHA-256 per wheel. Pip runs with no index, dependency resolution, source
builds, bytecode compilation or cache. Inherited environment and `PIP_*` options
are cleared; `PIP_CONFIG_FILE` is the platform null device, and the bootstrap
asserts that no global/user/site configuration loads. An audit guard installed
before pip imports denies network and child-process operations. The Rust caller
owns the only helper process, applies a timeout, and kills/reaps it on cancellation
or error. This is defense in depth, not an OS sandbox for arbitrary native code.

Installation uses a fresh private interpreter's explicit `--prefix`, preserving
standard wheel `.data/data` placement. The old bootstrap setuptools package is
removed before replacement. The bootstrap pip, generated launchers and reviewed
setuptools startup hook are removed from the finished component. Generated local
`direct_url.json` records are checked against approved wheel identities, then
removed. Deterministic `ASSEMBLY.json` preserves original upstream URLs/hashes
without CI/user paths. Only RECORD rows for deliberately removed files are dropped;
retained package file hashes and notices remain intact.

The app assembles into owned staging, verifies the bounded tree, self-tests it,
and activates atomically. Failed setup preserves the previous working component.
Managed **Repair** verifies/reinstalls the selected owned component through this
same verified recipe. Removal affects only Luma-owned copies, and replacement or
removal waits until the component is not in use. External Python/model paths are
advanced user-managed configurations: Luma does not install or repair them.

`generate_recipes.py` combines exact input bounds, Python/engine notices,
dependency inventory and applicable proprietary terms into review-only output.
CI generates it twice under `RUNNER_TEMP`, compares bytes and logs its hash.
The generator never changes the production catalog and retains an unavailable
reason for each candidate. `engine-terms.lock.json` pins displayed engine texts
and source-version evidence; an inventory is not a blanket license grant or a
runtime pass. The production catalog separately incorporates the reviewed recipes; the generator
continues to emit disabled candidates even after activation. For ownCPU generation,
provide `--cpu-publication-proof` with the exact downloaded proof pinned in
`cpu_component`; no live release discovery or implicit proof substitution occurs.
Its source identity remains `370c1e3` while new app/native reports use their own
activation-source identity.

## Licenses, notices and redistribution boundary

Private direct-upstream assembly does not grant Luma permission to redistribute
the downloaded binaries. Applicable upstream terms remain in force; required
terms must be displayed for explicit assent before the relevant download.
The engine's MIT/Apache notice does not cover every bundled dependency. Preserve
actual wheel notices, including those outside `.dist-info`, and source identities;
do not imply OpenAI, Alibaba, MLX or other upstream authors endorse this assembly.
ASR/aligner model licenses are a separate review and download decision.

Astral install-only archives omit the full distribution's third-party license
set. The exact full-vendor archives were hash-checked to retain `PYTHON.json` and
`python/licenses/*` under `scripts/asr-components/licenses/`.
`licenses.lock.json` pins each notice and its full-archive origin, including
CPython, OpenSSL, zlib, bzip2, libffi, SQLite and other native-library notices.
Supplemental locked material covers the missing macOS zlib-ng 2.2.4 notice.
Original package metadata/source references are retained; PyTorch, torchvision
and torchaudio revisions are recorded separately where no PyPI sdist exists.

The selected PyAV 19.0.1 Windows wheel includes FFmpeg, libx264, libx265,
libiconv and GCC runtime libraries, while its own license directory contains
PyAV's BSD notice and authors. Those notices alone do not establish the bundled
libraries' complete terms or corresponding-source fulfillment. See the
[upstream wheel-license report](https://github.com/PyAV-Org/PyAV/issues/2270)
and [FFmpeg build project](https://github.com/PyAV-Org/pyav-ffmpeg).
This blocks republishing a combined runtime; it is not itself an import failure.
The shipping route fetches the original PyAV wheel directly and does not publish
PyAV or FFmpeg binaries. The same separation applies to PyTorch, python-soxr,
soundfile, Microsoft CRT and other native inputs. Neither direct delivery nor
retaining notices waives applicable terms or source duties of a redistributor.

The old upstream CT2 wheel's Intel OpenMP/NVIDIA issues are not resolved by user
assent or deleting an unused DLL. That wheel is excluded. Luma's CPU wheel instead
builds a separately reviewed permissively licensed native closure, described below.
No new signing credentials or expanded publication permissions are requested.

## Separate CPU-only CTranslate2 wheel

The dedicated recipe in `scripts/asr-components/ct2-cpu/` builds official
CTranslate2 4.8.2 commit `d44d2d069eb88c7b7804da864c10c201501cb4a9` with static
oneDNN 3.1.1, exact cpu_features/spdlog sources and pybind11 headers. CUDA, cuDNN,
HIP, MKL and OpenMP runtimes are disabled. oneDNN uses its sequential CPU runtime
with MATMUL, CONVOLUTION and REORDER enabled. It may be slower than the upstream
multithreaded MKL/OpenMP variant; no speedup or performance parity is claimed.
The output is `ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl`.
Library sources are unmodified; the wheel adds reviewed notices/provenance and
normalized packaging. It contains no Microsoft DLLs or assembled Python runtime.

The managed Windows CPU recipe explicitly passes `cpu_threads=1` to
faster-whisper for reliable model cleanup, while keeping `num_workers=1`.
The launcher requires the exact owned-wheel recipe, Windows/CPU backend and an
active receipt matching the selected interpreter. A managed model or inherited
environment marker cannot select this policy. Native Whisper, external/manual
runtimes, macOS/MLX and Qwen are unchanged. This reliability tradeoff may reduce
CPU throughput; it makes no acceleration promise.

`sources.lock.json` and `notices.lock.json` bind every source/header, tool input,
flag and notice. The native closure uses MIT, Apache-2.0, BSD and Zlib licenses;
complete retained notices include embedded third-party code. The source ZIP
includes exact compiled-source dependencies, reviewed recipe/locks and provenance.
Luma's recipe scripts retain their own GPL license; this does not relicense
upstream libraries. Build-tool/Python binaries and inference weights are excluded.

The original CT2 source tar remains the exact hash-checked **build input**. For
public source export only, five reviewed unused `tests/data/models/**/model.bin`
fixtures are omitted because `BUILD_TESTS=OFF`. Every retained tar record,
compiled source, license and fixture metadata file is unchanged. `SOURCE-EXPORTS.json`
and the public proof record the original archive identity, every omitted member's
name/size/hash, retained inventory, and deterministic source-only export identity.
Any omission drift fails; the whole model subtree is not removed silently.

### Current Windows CPU evidence

Source proof [run 37996190880](https://github.com/csic21/luma-subtitle/actions/runs/37996190880),
at `370c1e3145b1a052a8317d8380f60b78b498f262`, passed both native jobs and produced
two genuinely fresh matching wheels at the same canonical path. The published
wheel is **25,058,630 bytes**, SHA-256
`5ce50225a796f676f812b732cc9e266d90e6ac3ef70e7de7a44e499a244bc449`.
[Publication run 38013836100](https://github.com/csic21/luma-subtitle/actions/runs/38013836100)
promoted the reviewed artifact without recompilation. Independent reads checked
all four public files, every archived Luma source file and all eleven notices;
the existing stable application release and `latest.json` stayed unchanged.

The proof passed isolated native imports/closure, pinned Tiny/JFK cold/warm timed
speech inference, Unicode relocation, model switch, explicit unload and normal
EOF under the one-thread source-selected policy. It explicitly records
`managed_receipt_selection_tested=false`; it does not establish app-global or
GUI selection. Earlier default-thread cleanup failures explain the one-thread
policy and are not converted into performance claims.

The final recipe proof must retain both independent layers: source-selected
reference assembly/smoke for this exact published wheel, and the actual Rust
managed receipt/lease lifecycle test. The latter must verify cold/warm SRT/export,
model replacement, release-idle, legacy release, active cancellation/recovery,
shutdown, script removal and process exit before lease release. A synthetic
constructor override or source-proof success cannot replace it. Final-source
native results remain required before release.

### Native proof and repeatability scope

`.github/workflows/asr-ct2-cpu.yml` runs cheap source guards on PRs. Native work
requires an explicit validated request-only push, manual proof or pinned reusable
metadata-only invocation. A proof request under `.github/requests/ct2-cpu-proof.json`
must be the only change in a single non-forced feature-branch push, name its sole
parent and preserve the exact tree elsewhere. The request commit itself is built;
run, jobs, checkout and report source identities must agree. A cheap PR guard is
not native proof.

The Windows proof requires all of the following:

- Exact preinstalled Visual Studio/SDK versions, compiler/tool hashes and an
  early actual compiler/path-mapping probe.
- Two fresh builds at the **same canonical native source/build path**, with
  sources re-extracted and the native root/objects removed between builds.
  Retained complete wheel bytes/hashes must match, with no native object-cache
  reuse. Actual paths/freshness remain diagnostic evidence; deterministic public
  provenance records only the strategy.
- Private PBS/offline assembly using the local CPU wheel; the original upstream
  CT2 wheel is skipped before fetch. All normal/delay PE dependencies and actually
  loaded native modules must close within the private runtime or allowed OS files.
  No host-global optional CRT, GPU, MKL or OpenMP fallback is accepted.
  The sole host-security exception is the exact registered Microsoft Defender
  AMSI `MpOav.dll`, checked by the byte-identified shared auditor for canonical
  non-reparse path, registration, locked-file hash, Authenticode, Microsoft product
  root and signed resource identity. DLLs loaded by trust APIs are re-audited to
  a bounded fixed point; no generic antivirus or signed-DLL exception exists.
- Relocation to a Unicode/spaced path, isolated imports, empty PATH, poisoned
  inherited Python configuration and fresh home/cache, then actual pinned Tiny/JFK
  cold/warm speech inference with valid text/timestamps and model reuse, plus
  normal EOF, explicit unload and a distinct Unicode-path model switch.
  Source-build selection requires the installed wheel's exact build provenance
  and uses the same production worker policy without a constructor override.
  Its report explicitly does not claim managed receipt or global-path selection.
  The native Rust installer proof separately holds a real managed use lease and
  rechecks the same production receipt/interpreter selector on every worker spawn
  using a test-owned catalog/root. That context is absent from release builds;
  the production embedded catalog is enabled only by the separately reviewed activation change.

This proves, if successful, fixed-path fresh-build repeatability on the recorded
CI toolchain/host. It does **not** prove path independence, cross-machine or
cross-toolchain binary reproducibility, performance, or old/non-AVX CPU support.
Matching hashes do not substitute for native inference; inference on one wheel
does not excuse a repeat-build mismatch.

### Guarded publication and catalog activation

Ordinary CPU proofs upload JSON/log/CMake-cache evidence only. Successful proof
constructs local wheel/source/notices candidates and `publication-proof.json`,
with `publication_authorized:false`. A separately reviewed direct proof request
may opt into `success_artifact: "cpu-wheel-source-notices-proof-14-days"` to retain
exactly those four files after both native jobs and all functional gates pass.
It is mutually exclusive with failed diagnostic retention; ordinary, manual,
reusable and PR routes remain metadata-only. No private runtime, Microsoft DLL,
model or download cache is uploaded.

The guarded `.github/workflows/asr-cpu-wheel-publish.yml` requires separate
maintainer approval and a request-only commit at `.github/asr-cpu-wheel-request.json`.
Its sole parent must be the exact tested source. The request binds repository,
feature branch, source, successful run/attempt, lock identities and expected
wheel/source/notices/proof sizes and hashes. Historical checks use immutable
`run.head_sha` and each `job.head_sha`, not a PR's mutable current head.
The request also pins the exact successful CPU job ID and candidate artifact
ID/name/ZIP size/SHA-256. A read-only `contents:read`/`actions:read` job promotes
that exact twice-built and tested artifact, without another compilation. It
validates the immutable run attempt and source, successful job/export steps,
creation interval, repository identity and expiration; rechecks metadata after
the restricted authenticated download; and extracts only four bounded regular
files after archive/member hash verification. Every proof/source/lock/asset gate
is reapplied before a same-run artifact-ID handoff. Only the separate validated
final publish job receives `contents:write`. Diagnostic, failed, expired, manual
or PR artifacts cannot qualify. Exact completed retries remain read-only even
after temporary artifact expiration; incomplete publication cannot substitute a
new build for missing reviewed bytes.

The separate tag is `asr-ct2-cpu-4.8.2-1`, a prerelease with explicit
`make_latest:false`, outside application `v*` releases. Existing tags/assets are
never overwritten, deleted or retargeted. An exact completed retry is read-only;
mismatches fail closed. Stable application `/releases/latest` and downloaded
`latest.json` identities are independently checked before and after mutation.
A GitHub permission failure is surfaced unchanged; the publisher must not change
its pinned target or expand credentials to bypass a Workflows-permission denial.
Publication still does not activate the app catalog: that requires a separately
reviewed immutable wheel pin and complete direct-source setup recipe.

## Native setup evidence and functional boundaries

`.github/workflows/asr-components.yml` is read-only and tests exact-source
private offline assembly on native Windows x64 and macOS arm64. Each eligible
recipe is assembled twice and compared, relocated and import-tested, then passed
to the actual Rust installer lifecycle test for Install, Repair, cancellation
rollback and removal. The generated Windows CPU candidate uses the published
ownCPU wheel and must pass the full reference and receipt-selected lifecycle
layers. A missing recipe or failed native dependency gate is not successful setup.

The workflow uploads only `asr-offline-pip-proof-<pack-id>` JSON evidence. Private
runtime binaries, model weights and caches stay out of artifacts. Test isolation
checks private Python search/import paths and actually loaded native libraries.
Python network/subprocess audit guards are defense in depth, not an OS sandbox.

The recorded app checks at source `370c1e3`,
[run 37996197510](https://github.com/csic21/luma-subtitle/actions/runs/37996197510),
passed on Windows and macOS. These checks predate the final catalog activation
and are separate from native setup and CPU-wheel proof gates.

Keep these evidence categories separate:

- Native setup [run 37996197575](https://github.com/csic21/luma-subtitle/actions/runs/37996197575)
  passed Windows Qwen, Apple Silicon Qwen and Apple Silicon MLX: private offline
  assembly repeatability, relocated imports and actual Rust install/repair/cancel/
  remove lifecycle tests. A real Metal tensor ran on the MLX host, not Whisper
  model inference. The old vendor-CT2 job in that run failed; it is replaced by
  the published ownCPU recipe and cannot be counted as a final recipe pass.
- CPU source proof `37996190880` establishes the exact published wheel's bounded
  inference and cleanup behavior. The new reference/receipt-selected activation
  proof is a separate release gate, not inferred from that source proof.
- Bounded Qwen model attempt [run 37994064106](https://github.com/csic21/luma-subtitle/actions/runs/37994064106)
  passed runtime/readiness/RAM gates, then failed at the downloader before
  inference. Its exact cause is unresolved; Qwen inference and memory fit are
  not established by that attempt.
- The Linux faster-whisper Tiny/JFK functional smoke exercised the real worker,
  cold/warm transcription, timestamp/SRT validation, export, inference cancellation
  and recovery. It does not validate the new Windows CPU wheel.
- Private runtime imports/API checks and small tensor operations do not load
  ASR/aligner weights. Qwen imports, alignment dataclasses and metadata-only probe
  fixtures are not Qwen speech inference, memory-fit or speed results.
- JSON-lines path rejection, clean EOF, idle termination/reaping and worker
  recovery tests are process/protocol evidence, not inference cancellation unless
  a real loaded model was transcribing at the time.
- MLX device reports distinguish `metal_available` from `metal_tested`; a CPU
  tensor on a host without Metal must never be called Metal validation. No MLX
  model inference is claimed. Qwen real-model proof is separately resource-gated;
  its existence alone is not a passing result.

The pinned SYSTRAN Tiny model and whisper.cpp JFK sample are CI inference inputs
only. No test model weights are included in public CPU assets or managed engine
assembly. Representative accuracy, speed/peak-memory comparisons and complete
native desktop UI/queue smoke remain separate checks. See [OPTIONAL_ASR.md](OPTIONAL_ASR.md)
for the dated user-facing validation boundary.

## Windows private dependencies

### Managed Nagisa initialization

Qwen pins Nagisa 0.2.11 and DyNet38 2.2. Nagisa's default absolute model path goes
through a native narrow-string API that fails under Unicode Windows paths.
The app-owned `src-tauri/src/asr/nagisa_compat.py` helper is embedded into the
managed worker/self-test, never loaded from a user-selected path. External
Python environments are unchanged.

Before worker activity threads, it checks exact private versions, bounded
metadata, retained RECORD files and module origins. For one official default
`Tagger` construction only, it supplies supported relative file arguments from
Python's Unicode-aware cwd. It restores constructor, loader and cwd in `finally`;
partial initialization failure is fatal. Original wheel bytes and the public API
remain unchanged. Native proof compares actual Japanese tokens/POS with ordinary
ASCII-path initialization and checks restoration after a deliberate error.
This uses bundled tokenizer data, not Qwen ASR weights, and does not establish
Qwen inference or CRT closure.

### Direct Microsoft CRT prerequisite

The implemented Windows Qwen candidate selects `windows_crt:msvc-14.44.35211-x64`;
the future CPU Whisper recipe must bind its reviewed prerequisite too. This is
one fixed package contract, not a generic installer. The package executable is
never run. Bounded exact CAB extraction yields only original `msvcp140.dll`,
`msvcp140_1.dll` and English/Chinese RTF notices. Existing PBS VCRUNTIME companions
must match the exact same-version originals. Full English/Chinese EULAs are
shown and hashed before assent and package download. Source identities and
original RTF hashes are preserved; this grants no binary redistribution right.

Install and Repair use bounded, cancellable Windows certificate-chain/revocation
validation. Trust checks can need internet even with cached bytes; private pip
stays offline, but fully offline Repair is not promised. Failures preserve the
current component and original trust error. No global install, admin access,
security-policy changes or Gatekeeper/SmartScreen bypass is used.

Native CI verifies signatures and exact extraction, protects the CRT files during
wheel preflight, and checks receipts after pip/activation. Whole-tree normal and
delay imports include Torch, PyAV and DyNet; loaded optional CRT/OpenMP libraries
must remain private. Unknown missing libraries fail rather than widening the
two-DLL policy. A CI-only CPU test copy from the licensed hosted Visual Studio
installation is technical evidence, never authority to publish a CRT sidecar.

### Managed Windows Qwen: configured Numba threading

Numba 0.68.0 contains an optional `tbbpool.cp312-win_amd64.pyd` plugin whose
`tbb12.dll` dependency is not installed. Numba documents its built-in
[`workqueue` backend](https://numba.readthedocs.io/en/stable/user/threading-layer.html)
and tolerates unavailable optional threading libraries. The managed Windows
Qwen child explicitly selects workqueue before importing Numba/Qwen. Its
private setup self-test uses the same embedded initializer. Inherited Numba
developer settings are replaced inside that child with a fixed JIT-enabled,
CPU-only, two-thread policy; manual runtimes are unchanged. An unexpected
working-directory `.numba_config.yaml` or conflicting initialized state fails
closed. The managed child uses its verified owned runtime as its working
directory, never modifying a user's launcher directory or configuration file.

The optional plugin is retained unchanged. Native inventory compares its
unique member bytes against the complete SHA-256-pinned official Windows Numba
wheel already in the verified input cache, then records the member size/hash.
Only that exact member's normal `tbb12.dll` edge can be labeled inactive. Every
other missing required import remains a failure. Evidence distinguishes
`required_closure_passed` from `full_tree_closure_passed:false`; it does not
claim unconditional closure for an installed but inactive optional plugin.

A real parallel JIT reduction checks numerical results, selected workqueue,
the private module origin, and absence of loaded TBB before/after compilation
and after Qwen imports. The configured-policy gate passes only with those
results. The actual managed worker runs the same proof at its first runtime
initialization, caches only successful proof, and rechecks initialized workqueue
after imports and each transcription. Native CI calls the real embedded
`worker.runtime()` twice in an isolated, offline child to verify initial JIT,
proof reuse and every loaded native origin without loading model weights;
the owned child has a 120-second/16-KiB output bound.
Numba workqueue is not reentrant: the application serializes worker
requests and makes no nested/concurrent Numba calls; heartbeat threads only
emit progress. These checks do not establish Qwen model inference. Native
success for the exact final source remains required.

The CI/private-assembly loaded-module audit separately identifies the host's
Microsoft Defender AMSI `MpOav.dll`. This is a single security-module category,
not a general ProgramData or signed-DLL exception. It requires the exact native
HKLM Defender AMSI/CLSID registration, the OS-known versioned Defender platform
location, identical canonical paths without reparse points, a read-held file,
successful Windows Authenticode/revocation verification, the Microsoft product-root
policy with test/flight roots disabled, and matching signed module resource and
publisher identity. Unknown injected modules still fail; numbered CRT/OpenMP
libraries must remain private before either OS/security classification applies.
Evidence records `verified_host_security_modules` separately from the legacy total
loaded-library check count, including file hash, version and certificate-chain
identity. Trust APIs' newly loaded libraries are also audited in at most three
snapshots. Normal Windows certificate/revocation metadata retrieval may occur
inside the existing timeout-owned proof child; model networking remains blocked,
and no security settings, Defender state or trust store are changed. This audit
is CI/private-assembly proof only, not a shipping Rust installer trust subsystem
or model-inference evidence (`inference_tested:false` remains explicit).

Adding current official TBB wheels is not a simpler unconditional-closure fix:
the inspected `tbb==2023.1.0` plus `tcmlib==1.5.0` add 795,220 download bytes,
1,974,592 unpacked bytes and Intel Simplified notices, while the latter also
contains an optional debug DLL requiring debug CRTs. Neither dependency is
added to the shipping recipe by this policy.

The direct-CRT proof compiles the real application's `main.rs` with
`cargo rustc --locked --bin luma-subtitle -- -C debug-assertions=no`, checks its
Windows GUI subsystem, and invokes only the fixed early signature-helper mode.
It tests online/cache-only trust separately, rejects invalid roles/paths, and
uses Unicode/spaced paths, poisoned PATH and bounded inherited handles. This is
not a full release/bundle or UI test; no downloaded installer is launched.
No macOS Developer ID/notarization or Windows SmartScreen reputation is claimed.

## Historical ZIP fixtures only

`build.py`, the normalized `luma-asr-<pack-id>-1.2.0-r1.zip` layout,
`asr-components-1.2.0-r1` tag naming and the old whole-runtime controller are
retained for deterministic comparison/security unit fixtures. They are **not**
the shipping route; `.github/workflows/asr-components-publish.yml` has no
publication path. There is no proposed whole-runtime prerelease.

Those fixtures use ordinary safe relative files, normalized ZIP order/timestamps,
fixed entry points, component manifests and notice inventories. Archive extraction,
traversal/link rejection, byte/file ceilings and deterministic-record tests remain
useful. The historical `pruning.json` removal of one exact unused cuDNN DLL and
its `INSTALLATION_CHANGES.json` receipt are not the current CPU build strategy
and do not clear the upstream wheel's Intel/NVIDIA redistribution issues.
The active CPU recipe builds from source and never fetches or prunes that wheel.

Run dependency-free source/packaging guards locally:

```sh
python -B -m unittest discover -s scripts/asr-components -p 'test_*.py' -v
python -B -m unittest discover -s scripts/asr-components/ct2-cpu -p 'test_*.py' -v
node --test scripts/asr-components/ct2-cpu/publish.node-test.cjs scripts/asr-components/ct2-cpu/proof_request.node-test.cjs
```

These unit guards do not install runtimes, run native compilation or establish
model inference. Successful exact activation-source native proof and final release review
remain required before merge and application release. The separate CPU component
publication does not satisfy those application-release gates.
