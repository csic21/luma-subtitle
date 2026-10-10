# Retired Qwen diagnostic and inference tooling

The one-time feature-branch workflow and request have been retired. Neither
`.github/workflows/asr-qwen-inference-proof.yml` nor
`.github/requests/qwen-inference-proof.json` is present. These retained scripts
and fixtures do not provide an active Actions execution route or authorize a
model download. Ordinary app/native CI runs only their download-free tests.

## Recorded outcomes and limits

- The bounded Windows CPU Qwen3-ASR-0.6B + ForcedAligner-0.6B proof in
  [run 38028126893](https://github.com/csic21/luma-subtitle/actions/runs/38028126893),
  attempt 1, passed at source `401c344b6e9786ba52af280639b57b7c3ed27dbf` for
  `qwen3-asr-cpu-windows-x64`. Fresh native readiness and resource gating passed;
  the exact pinned 3,720,689,099-byte pair downloaded, and the 11-second JFK
  fixture produced three ordered, positive, in-duration timestamped segments,
  ending at 10,480 ms. Cold-worker elapsed was 36.875 seconds, including startup,
  model loading, ASR/alignment, process completion and result checks; it excludes
  setup/downloads. Whole proof elapsed was 1,686.25 seconds, a separate measure.
  Before download, available RAM was 14,428,192,768 bytes, total RAM was
  17,174,360,064 bytes and free disk was 150,426,079,232 bytes. These capacity
  readings are not peak-memory measurements or device recommendations.
  Fixture-cache and private-output cleanup passed; model cache was not uploaded.
  The 2,046-byte inference report SHA-256 is
  `0ac94ff0c8b1394fa6b7cb43362c51ddf17507cf20b745e5c61caa7d75f3d71f`;
  the repeated fresh-native-proof SHA-256 is
  `c17d1714bd8edf4bb670703344f5c8b33636d971449e4855cd45f57ba6ba5db7`.
- The earlier bounded Windows full-model attempt in
  [run 38024179671](https://github.com/csic21/luma-subtitle/actions/runs/38024179671)
  at source `0875deb4430ac84de1fabc80a9f4454589e13fba` passed fresh native
  readiness and resource gates. Five metadata/tokenizer files (1,736,805 bytes)
  were downloaded before the first model-weight redirect was rejected. No
  inference ran. Owned child/cache/private-output cleanup was confirmed; the
  exact rejected host was not recorded. An independently documented two-host
  CDN compatibility gap is corrected in the app and fixture (see
  [model sources](../../../docs/ASR_MODEL_SOURCES.md)). The later passing download
  does not identify the earlier rejected host; this was not an inference failure.
- The README-only diagnostic in
  [run 38021420377](https://github.com/csic21/luma-subtitle/actions/runs/38021420377)
  failed in locale-dependent tests before its network operation. It did not
  establish a successful README fetch, weight download or model inference.
- All four native runtime jobs in
  [run 38026666599](https://github.com/csic21/luma-subtitle/actions/runs/38026666599)
  passed at source `401c344b6e9786ba52af280639b57b7c3ed27dbf`. The Qwen jobs
  exercised private setup/import/repair/cancel/remove and native dependency
  checks, without loading ASR/aligner weights.

The successful result establishes only that one cold Windows CPU 0.6B speech
fixture and forced alignment. It does not establish real-time performance,
quality/speed improvements, general memory fit or peak memory, Qwen 1.7B/macOS
inference, warm reuse, Qwen managed receipt/use-lease worker lifecycle,
cancellation/export/GUI, MLX speech or GPU inference/performance.
All optional runtimes remain experimental. A README-only success, if obtained
separately, would prove only that exact file
read and hash; it could not establish weight-CDN access or inference. Native
imports, Japanese tokenizer results, workqueue/tensor checks and resource-gate
skips are also distinct from speech-model inference. Final application-source
checks and review are specified in [RELEASING.md](../../RELEASING.md).

## Retained request and resource guards

`validate_request.cjs` preserves the historical strict request contract and
negative-test fixtures: exact repository/feature branch and source parent;
request-only, regular-file changes; exact native run/attempt/job provenance;
fixed model revisions, byte counts and audio hash; and disjoint `full-inference`
and `readme-diagnostic` selectors. Unknown fields or relaxed limits fail closed.
These fixtures are not instructions to recreate a request or workflow.

The retained full-inference harness repeats native private-runtime readiness
before measuring available physical RAM and free disk. Its minimums are 13 GiB
RAM and 12 GiB free disk; a failed measurement or insufficient resources stops
before model downloads. The locked 0.6B ASR/aligner files total 3,720,689,099 bytes.
A permitted run has a 600-second aggregate downloader deadline and a separate
900-second CPU inference deadline. Successful inference requires actual expected
speech text and ordered, in-duration forced-alignment timestamps.

The README diagnostic remains isolated from runtime assembly, model/audio fetches
and inference. Its fixed 57,456-byte README and SHA-256, 30-second deadline,
8,192-byte combined output cap and exact response-length checks are unchanged.
Official HTTPS origins, credential/proxy isolation, bounded redirect handling,
child kill/reap and model-byte/hash checks remain covered by durable tests.
UTF-8 subprocess handling and CRLF-independent fixtures are also retained.

Reports use bounded metadata rather than response bodies, tokens, environment
contents or weights. The harness removes owned runtime/model/fixture output only
after confirmed child termination; uncertain reap/Windows Job drain must be
reported as cleanup failure and preserve those paths. No binaries or weights
are intended as proof artifacts. Reusing this harness for a new native/model
attempt requires a separately authorized and reviewed execution route.

## Download-free tests

```sh
node --test scripts/asr-components/qwen-inference/validate_request.node-test.cjs
python -B -m unittest discover -s scripts/asr-components/qwen-inference -p 'test_*.py' -v
```

The Node suite also checks that both retired control paths remain absent.
These tests do not install packages, fetch models or establish inference.
