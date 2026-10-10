# Legacy engine and model download trust

The in-app FFmpeg, whisper.cpp, llama.cpp, Whisper/Silero VAD and Hy-MT2
installers use the embedded `src-tauri/src/dependencies/artifacts.json` catalog.
There is no runtime release discovery, remote manifest, `latest` executable,
mutable model revision, checksum learned from the download response, or
unverified fallback. Changing a pin requires a reviewed app update.

## Reviewed versions and provenance

Pins were reviewed on 2026-10-10. The catalog records every artifact's exact
HTTPS URL, byte size, SHA-256, archive expansion limit and entry limit.

- Windows FFmpeg: Gyan's **8.1.1 essentials** ZIP, from the publisher's
  [versioned GitHub release](https://github.com/GyanD/codexffmpeg/releases/tag/8.1.1).
  SHA-256 and download size are the release asset's GitHub `digest` and `size`.
- macOS FFmpeg: official [8.1.1 source tar.gz](https://ffmpeg.org/releases/ffmpeg-8.1.1.tar.gz).
  The complete 17,674,354-byte archive was downloaded from that HTTPS origin
  and hashed to `1b856f26a07082b6879f3e5300d81e8c7ce3b410ade5898b14382d90c2904634`.
- whisper.cpp: official [v1.8.3 Windows releases](https://github.com/ggml-org/whisper.cpp/releases/tag/v1.8.3),
  preserving CUDA 12.4, BLAS and CPU packages. Asset hashes and sizes come from
  the publisher's versioned GitHub release metadata. macOS builds source commit
  [`2eeeba56e9edd762b4b38467bab96c2517163158`](https://github.com/ggml-org/whisper.cpp/commit/2eeeba56e9edd762b4b38467bab96c2517163158),
  resolved from that release tag. The complete commit-addressed codeload tar.gz
  was downloaded and hashed to `089b898aa83b24a8321e0fd554eeb0967fb03dd687e27f6374c72d3363b5b429`
  (7,901,615 bytes).
- llama.cpp: official [b11243 release](https://github.com/ggml-org/llama.cpp/releases/tag/b11243),
  preserving Windows CUDA 12.4 plus its matching CUDA runtime, Vulkan and CPU,
  and Apple Silicon Metal. Hashes and sizes are the versioned release assets'
  GitHub `digest` and `size`. The downloaded Metal tarball was also rehashed.
- Whisper models: repository revision
  [`5359861c739e955e79d9a303bcbc70fb988958b1`](https://huggingface.co/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1).
- Silero VAD 6.2.0: repository revision
  [`9ffd54a1e1ee413ddf265af9913beaf518d1639b`](https://huggingface.co/ggml-org/whisper-vad/tree/9ffd54a1e1ee413ddf265af9913beaf518d1639b).
- Hy-MT2 1.8B Q4_K_M: repository revision
  [`a0c709d9fac510f2c807aa3af52872340dc37a4a`](https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF/tree/a0c709d9fac510f2c807aa3af52872340dc37a4a).
- Hy-MT2 7B Q4_K_M: repository revision
  [`ab8472660ac61fac25f1af43fac2599d52a8a775`](https://huggingface.co/tencent/Hy-MT2-7B-GGUF/tree/ab8472660ac61fac25f1af43fac2599d52a8a775).

Model hashes and sizes were cross-checked against Hugging Face's Git LFS object
metadata at each exact revision, not only the moving default branch. These are
publisher/HTTPS trust pins, not a claim of independent reproducible builds or
publisher signatures for all artifacts.

## Download and activation rules

- Only exact embedded URLs can start downloads. HTTPS, no URL credentials and
  default HTTPS port are required. Redirects have a hop limit and a separate
  source-specific host allowlist, including GitHub release-asset hosting and
  Hugging Face's current model CDNs. No browser cookies or app API keys are sent.
- Every download has an exact byte cap. Content-Length must agree when present;
  encoded bodies, oversized output and unexpected success statuses are rejected.
- A resumed HTTP 206 response must specify the exact expected offset, end and
  pinned total. HTTP 200 restarts from zero. HTTP 416 never means completion.
  All bytes, including a resumed prefix or pre-existing model, are rehashed
  against the pin before the installer reports completion.
- Unique staging paths isolate concurrent requests. Partial files are cleaned
  after failure. Network retries can resume within the current request, but a
  later user retry starts a fresh staging file.
- ZIP extraction reuses the optional-runtime strict extractor. It rejects unsafe
  paths, duplicate/case-aliased names, symlinks, special files, inconsistent local
  and central metadata, unsupported extensions and extraction-limit overruns.
  Well-formed Unix UID/GID fields used by official llama ZIPs are ignored;
  ownership and unsafe permissions are never restored.
- The tar.gz extractor bounds the whole decompressed stream and all file output.
  It rejects path overrides, sparse/special files, unknown extensions and unsafe
  or cyclic links. Internal dylib links in the official macOS llama archive are
  materialized as regular-file copies. The only accepted global PAX record is
  the exact pinned Git commit comment in the whisper source archive.
- CUDA executable and runtime payloads are both verified and extracted before
  activation. Failed downloads, builds or extraction leave an existing install
  intact; a failed final rename attempts to restore the previous directory.
  Native installers and conflicting preset downloads are serialized within the app process.
  macOS build tools and rpath utilities use owned process groups, bounded output
  and deadlines; abandoned requests terminate/reap them before staging cleanup.

Previously installed, bundled or explicitly configured engines continue to work.
They are not retroactively asserted to match these download pins. User-selected
local models remain supported. The pins govern new in-app downloads and preset
reuse, not arbitrary files supplied by the user.

## Regression coverage

Run the network-free native tests on Windows and macOS:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked dependencies::
```

The tests cover catalog completeness, origins and redirects, exact 200/206/416
handling, hash/size mismatch, rehashing cached files, ZIP and tar attacks, safe
macOS dylib link copies, inert UID/GID metadata and activation rollback.
These tests do not establish native engine inference performance or reproduce
upstream binaries. The existing native application build and engine proof gates
remain required for release.
