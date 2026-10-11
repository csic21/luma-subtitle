# Native release installation IPC regression

This runner builds a **separate diagnostic application**, in release mode, and
loads a small test page in the real Tauri Wry/WebView2 (or macOS WebKit) runtime.
The page invokes `install_llama_cpp`, `download_translation_model`, and
`download_status` through JavaScript IPC. The handler registers the production
`#[tauri::command]` functions with `tauri::generate_handler!`; it does not call
replacement commands, synthetic futures or a MockRuntime.

The ordinary application is built separately with no diagnostic feature. These
are not byte-identical executables: the fixture entry point, assets, logging and
trust roots differ. The production command definitions, wrappers, downloader,
hasher, extractor, receipt validation and activation code are shared. On Windows
the controller records and compares PE `SizeOfStackReserve`/`SizeOfStackCommit`
for diagnostic and production binaries. It never supplies `/STACK` or changes a
thread stack size. Real WebView IPC determines the dispatch thread; each stage
records whether it ran on the main thread.

## Run

On Windows x64 or macOS Apple Silicon, with the normal native Tauri prerequisites:

    python scripts/native-install-regression/run.py --ab

The `--ab` option additionally builds the same candidate source with the original
64 KiB inline hash buffer; the default variant uses the heap buffer. Both compile
with `--release --locked`. They use separate child processes and the same fixture
bytes and common cases. All fixed/heap cases must pass. The inline variant's
runtime outcome, including successful exits, is recorded without requiring an
invented baseline failure. A build/controller failure still fails the gate.

This is a **candidate-source buffer comparison, not an exact v1.2.2 baseline**.
Other fixes are present in both variants. If both pass it is negative evidence
for a reproduction, not proof that the reported host crash was reproduced or
that the buffer caused it. The original v1.2.2 crash remains unconfirmed unless
separately reproduced under faithful original conditions.

After an ordinary build, check the production executable separately:

    python scripts/native-install-regression/run.py --verify-production PATH_TO_NORMAL_EXECUTABLE

CI and the release workflow perform these steps on their exact checked-out SHA.
They preserve evidence as workflow artifacts even on failure. Use a new/empty
`--output` directory for each suite so stale results cannot mask a crash.

## Isolation and non-distribution guard

- The opt-in `native-install-regression` Cargo feature is absent by default.
- `build.rs` refuses diagnostic builds unless they are release mode, explicitly
  authorized by the build controller, and use the dedicated diagnostic
  `CARGO_TARGET_DIR` basename. Both variants reuse dependency compilation there,
  then preserve their exact executable in separate report directories before
  rebuilding. Normal Tauri packaging uses a different target.
- The feature selects a different main entry and a configuration with bundling
  disabled. It does not enter the normal application startup.
- Each process creates a fresh UUID temporary root for managed data, settings
  and its WebView profile. It does not open existing app data or settings.
- Only the exact embedded fixture descriptor is accepted by fixture trust. The
  loopback port is created in-process. No environment variable, URL parameter,
  catalog file or IPC argument can introduce a different fixture/source.
- The fixture HTTP client disables proxies and redirects. Ordinary production
  builds retain the original pinned HTTPS origin/hash/byte rules.
- The engine archives contain clearly labeled, non-executable dummy bytes. No
  model inference or engine execution occurs. File hashes and sizes are fixed
  in `fixtures/manifest.json`; ZIP is 360 B, tar.gz 213 B, and model 104 KiB.
- Normal production binaries are scanned for the diagnostic marker before the
  release publish job can run. The script records their SHA-256 and PE stack.

## Assertions and evidence

Each case is an independent native subprocess, with bounded completion time.
The controller retains stdout, stderr, signed/numeric and hexadecimal exit code,
timeout state, IPC report and flushed stage JSONL. A stack overflow/native abort
therefore cannot abort the test controller or be mistaken for an ordinary error.
It also records rustc/cargo versions, source SHA and working-tree status,
Cargo.lock hash, inherited compiler flags, fixture hashes, binary hash and PE
stack values. Existing Windows Application Error/WER events are read after the
run. No registry/debugger settings are changed and no minidump/native call-stack
collector is installed; absence of those artifacts is explicitly reported.

Fixed suite:

- Engine cold install, valid cached receipt, stale unreceipted engine, and missing
  DLL repair. Warmup and repeated calls use real IPC. Cache asserts one request;
  missing-DLL repair asserts two. Successful repair backups preserve prior bytes.
- Invalid archive after successful hash verification, wrong hash, HTTP rejection,
  invalid activation target, and a selected model nested under the engine tree.
  Failure asserts prior data unchanged, terminal failed event, rejected IPC and
  the same authoritative `download_status` in the WebView and Rust state.
- Model cold install, valid cache, same-sized corrupt cache/recovery, genuinely
  truncated HTTP body with resumed retries, wrong hash, HTTP rejection, and
  activation failure after successful download/hash verification.
- Two concurrent engine requests and two concurrent model requests go through the
  generated wrappers and existing serialization gates. A second install reuses
  the result instead of downloading again.
- Windows additionally holds real OS handles that deny delete sharing to exercise
  directory/file rename failures. Those cases are not claimed on macOS.
- No `.part` or `.installing` leftovers; retained `.previous` directories are
  required and inspected for repair cases, forbidden for other cases.
- Metrics run in a separate child, so constructing measurement futures does not
  perturb ordinary regression dispatch. It records actual verifier, command,
  llama inner/outer, downloader with no-op callbacks and Tauri ResultFutureTag
  future sizes. ResultFutureTag and no-op callback measurements are labeled;
  they are not a measurement of every compiler-generated wrapper closure or of
  production UI callback monomorphizations.

The tiny page validates real frontend IPC/event delivery, not the complete React
settings UI. React state/event ordering has separate frontend tests. This does
not test external network/CDN availability, real multi-GB artifacts, CUDA/Vulkan
hardware, full engine startup or the installed MSI/NSIS launch environment.

## Platform-independent controller checks

    python scripts/native-install-regression/run.py --check-fixtures
    python -m unittest discover -s scripts/native-install-regression -p 'test_*.py' -v

No native execution result should be claimed from those checks. The native suite
must actually run on its target platform before its outcome is known.
