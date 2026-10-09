# CPU-only CTranslate2 Windows proof

This is a **non-publishing proof build**, not an approved engine download. The
application's native Whisper/Turbo defaults and the existing component locks are
unchanged. No upstream Windows CTranslate2 wheel is downloaded, unpacked, pruned
or repackaged by this build.

## Scope and identity

- CTranslate2 4.8.2, official commit
  `d44d2d069eb88c7b7804da864c10c201501cb4a9`.
- CPython 3.12.15 x64 from the existing byte-locked Astral
  python-build-standalone release 20261003.
- Output identity:
  `ctranslate2-4.8.2-1lumacpu-cp312-cp312-win_amd64.whl`.
- The library and Python source are not changed. The supported wheel build tag
  identifies this as a Luma build, not the upstream PyPI artifact.
- Packaging adds licenses and `ctranslate2-4.8.2.dist-info/LUMA_CPU_BUILD.json`,
  removes the upstream GPU classifier, and normalizes ZIP order/timestamps and
  RECORD. It does not rewrite executable payloads.

## Supported build options

The complete exact option sets, official archive URLs, sizes, SHA-256 values,
submodule gitlinks and build-tool wheels are in `sources.lock.json`.

CTranslate2 uses `WITH_DNNL=ON`, `OPENMP_RUNTIME=NONE`,
`BUILD_SHARED_LIBS=ON`, `ENABLE_CPU_DISPATCH=ON`, and Release mode. MKL, CUDA,
cuDNN, HIP, GPU dynamic loading, tensor parallel, flash attention, Ruy, OpenBLAS,
Accelerate, tests and CLI are disabled.

oneDNN 3.1.1, official commit `64f6bcbcbab628e96f33a62c3e975f8535a7bde4`,
is statically linked with `DNNL_CPU_RUNTIME=SEQ`, `DNNL_GPU_RUNTIME=NONE`, and
`DNNL_BLAS_VENDOR=NONE`. Its supported inference primitive selection includes
MATMUL, CONVOLUTION and REORDER: the latter two are required by CT2's oneDNN
Conv1D implementation used for speech models. The float32 and int8 GEMM APIs
remain available. Graph, tests, examples, ITT tasks and JIT
profiling integrations are disabled. No oneAPI, OpenMP or TBB runtime is used.
The CTranslate2 cpu_features and spdlog submodules are pinned to the exact
upstream gitlinks. Unused GPU/CLI/test/Ruy submodules are recorded but not fetched.

This is a supported CMake variant, not a rewrite of CTranslate2 or oneDNN. Native
code uses the normal MSVC dynamic CRT (`/MD`) consistently. The official Python
setup does not offer a static-CRT switch; this proof does not override it or mix
its `/MD` Python extension with the `/MT` CMake static-library configuration.

## Performance limitations

CTranslate2 uses its built-in custom threading when OpenMP is disabled, but
oneDNN's SEQ GEMMs are sequential. This may be substantially slower than an
upstream MKL/OpenMP build on a multicore CPU. The proof makes no claim of speedup,
parity with an upstream wheel, GPU support, or suitability for all model sizes.
A passing Tiny test is a correctness check on the CI host, not a benchmark or
validation of old/non-AVX machines.

## Build and verification

The dedicated `.github/workflows/asr-ct2-cpu.yml` workflow has a read-only token and
no release steps. Checkout/setup-python/upload actions are pinned by commit.
The orchestrator is exact x64 CPython 3.12.10, separately installed by
setup-python and invoked by its absolute path after developer-shell setup. The
private target/build interpreter remains the byte-locked PBS runtime. It:

1. Runs the pure-Python tests for source locks, safe archive extraction, notice
   hashes, canonical wheel records, and PE normal/delay import inventory.
2. Requires the version-pinned preinstalled Visual Studio 2022 toolchain and
   Windows SDK. Records compiler/linker/resource-tool versions and SHA-256 values.
   A serviced image with a different version fails closed for review.
3. Inventories the official toolchain's original x64 CRT redistributable files,
   names, versions, hashes and Authenticode signers before any private proof copy.
   A bounded read-only scan of the exact installation/Redist trees records
   existing public license-document metadata and recognizable Microsoft terms.
   It skips activation/key files and links; recovered text is evidence for
   review, never an automatically established redistribution grant.
4. Fetches every input by exact URL, byte length and SHA-256. The build interpreter,
   headers and import library come from the pinned PBS archive; setuptools,
   wheel, pybind11, CMake and Ninja are exact hash-locked inputs.
5. Builds oneDNN, CTranslate2 and the wheel twice at one canonical native
   source/build path. Each build starts with an absent native root and freshly
   extracts all pinned sources. Its wheel is retained separately, then every
   native source, build directory and object is removed before the second build.
   Freshness reports record the actual canonical path and retained hashes.
   `/Brepro`, the pinned compiler-required `/experimental:deterministic`
   and path mapping control native build differences; an early actual compiler
   probe confirms mapping before the full build. Matching complete wheel hashes
   are required. Per-member hashes, PE timestamps, section hashes and CodeView
   build IDs are reported on mismatch, without modifying binary bytes.
6. Creates a new private PBS runtime and uses the shared guarded offline pip
   installer with the existing faster-whisper 1.2.1 closure, replacing only CT2
   with this locally built wheel. The original CT2 wheel is skipped before fetch.
   In the reviewed licensed hosted-CI environment only, adds the exact original
   `msvcp140.dll` and `msvcp140_1.dll` after checking their source paths, signatures,
   versions and bytes. Existing PBS VCRUNTIME files must be identical to the same
   official Redist versions. Existing notices and the observed Redist pointer
   remain intact; no runtime or CRT binaries are uploaded.
7. Relocates that runtime to a Unicode/spaced path and inventories every EXE, DLL
   and PYD, including PE delay imports. Unknown dependencies, any GPU/MKL/OpenMP
   runtime and any missing private VC CRT block the proof before imports.
8. Only after the dependency gate passes, uses the private isolated interpreter, empty PATH,
   poisoned inherited Python configuration, fresh home/cache and offline worker
   audit to import actual backends and run the pinned Tiny/JFK fixture cold and
   warm. Requires valid transcription/timestamps and warm-model reuse; inventories
   the modules actually loaded and rejects host-global VC CRT fallback.
   These functional tests run on the exact first wheel even if the second build
   differs; that mismatch still fails the final proof and publication gate.

The OS/runner image is recorded, not a hermetically reproduced Windows image.
The two-build result establishes only fresh-build repeatability at the same
canonical native path with that pinned toolchain. It does not establish path
independence or cross-machine/cross-toolchain binary reproducibility.

CT2 4.8.2 also contains an upstream GEMM/im2col Conv1D fallback when oneDNN is
disabled. A different GEMM backend is therefore possible, but would require its
own Windows dependency/source and functional proof. Keeping the already pinned
oneDNN backend and enabling its required CONVOLUTION/REORDER primitives is the
smaller correction for this variant; no library source rewrite is needed.

For an already initialized official developer environment, the build entrypoint is:

```powershell
. ./scripts/asr-components/ct2-cpu/prepare_toolchain.ps1 `
  -Reports 'dist/ct2-cpu-reports' `
  -Lock 'scripts/asr-components/ct2-cpu/sources.lock.json'
python -B scripts/asr-components/ct2-cpu/build_cpu.py `
  --source-sha '<exact checked-out commit>' `
  --work '<new empty private work directory>' `
  --cache '<download cache>' --reports 'dist/ct2-cpu-reports' `
  --worker 'src-tauri/src/asr/worker.py'
```

No administrative action, global install or change to global PATH is used.

## License and CRT gates

The native source/header closure is MIT, Apache-2.0, BSD-2-Clause,
BSD-3-Clause and Zlib. `notices.lock.json` records each actual notice and its
source/hash, including CT2's embedded thread-pool/math headers, spdlog's bundled
fmt, oneDNN's bundled-program notices and pybind11. The thread-pool MIT notice is retained from its pinned upstream source.

Unused libraries are not described as part of the executable closure just
because a broad upstream notices file mentions them.

Microsoft's VC runtime is **not** a permissively licensed open-source dependency.
The existing PBS supplies plain `vcruntime140.dll` and `vcruntime140_1.dll`, but the
current whole faster-whisper runtime also needs plain `msvcp140.dll` and
`msvcp140_1.dll` (including ONNX Runtime's dependencies). NumPy's renamed private
MSVCP DLL is not a supported substitute and is never renamed or copied for CT2.
A host import succeeding because System32 contains those files is not a clean
private-runtime pass.

The official [Microsoft redistribution guidance](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170),
[Visual Studio 2022 REDIST list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution),
[applicable Enterprise/Professional license terms](https://visualstudio.microsoft.com/license-terms/vs2022-ga-proenterprise/),
and [VC runtime terms](https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/)
are review inputs. Microsoft conditions redistribution on applicable licensed
Visual Studio use and terms. A signed file and a REDIST entry are provenance,
not by themselves a confirmed redistribution grant for this project.

`crt_proof.py` permits only a CI-local technical test using the two reviewed
original files from the licensed hosted Visual Studio installation. The source
version is 14.44.35211.0, and PBS's existing VCRUNTIME files exactly match those
same-version original hashes. This does not authorize a public CRT sidecar.
`private-crt-proof.json` records every identity and retains the unmodified
187-byte Redist pointer as pointer-only evidence, never as a license grant.
The independent public-redistribution gate remains false. The proof still fails
for any missing private dependency or host-global loaded CRT; no global install,
DLL substitution or source/static-CRT workaround is used.

## Direct upstream CRT package audit

A separate short Windows job validates one exact Microsoft-hosted VC Redist
14.44.35211.0 package without running its installer, DLLs or MSI, installing
anything globally, or accepting an agreement. The helper in `direct-crt/`
checks the whole package and fixed CAB slices/members before using Windows'
existing system `expand.exe`. Native PowerShell then validates Authenticode for
the original installer and the two extracted DLLs. Only extraction/signature
JSON is uploaded; package, CAB, DLL and notice bytes remain private to CI.
This audit does not integrate an app installer or clear public redistribution.

PR revisions run cheap source/configuration guards and supersede only that PR's
earlier guard run. Native compilation requires a validated request-only push,
an explicit manual proof, or the source-pinned reusable publication invocation.
Native proofs have unique concurrency groups and are not automatically canceled
by later branch updates. A successful cheap guard is never native-build evidence.

## Release hold

Default, PR and manual proof runs upload only JSON/log/CMake-cache evidence.
Successful proofs also generate deterministic source/notices candidates and their
asset pins locally; their publication-proof.json alone is included in reports.
An optional workflow_call publication input is accepted only from the dedicated
validated request-only publisher. After all native gates succeed, that caller
may export exactly the CPU wheel, source ZIP, notices ZIP and proof JSON as a
candidate artifact. No assembled Python/runtime, model or CRT bytes are exported.
The build itself never creates a release. All result/provenance reports state
`publication_authorized: false`. A successful native test does not clear licenses
or authorize release.

Before any later immutable, non-latest component release, independently review:

- Every native source/header license and the full-runtime notices/source duties
- The exact official app-local CRT files and applicable redistribution grant
- Matching two-build wheel hashes and exact native/import/inference evidence
- The immutable wheel URL/hash and original-vendor-wheel rejection in the recipe

That future release must remain separate from the application's `v*` release
workflow. The guarded publisher in `publish.cjs` and
`.github/workflows/asr-cpu-wheel-publish.yml` remains dormant without a separately
reviewed request-only commit pinning the successful source, proof and assets.
These build controls do not activate a component download URL or embedded catalogue.
