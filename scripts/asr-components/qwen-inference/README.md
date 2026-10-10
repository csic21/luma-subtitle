# One-time, bounded Qwen diagnostic and inference evidence

This temporary control is for the reviewed feature branch only. It does not enable a
production runtime or model. Remove `.github/workflows/asr-qwen-inference-proof.yml`
after the reviewed one-time attempt and before the application release. It may
accompany a reviewed source prerequisite merge to main for exact component-release
workflow parity; its strictly feature-branch-scoped push trigger still cannot run
on main. No request file accompanies these scripts, and no ordinary PR action
invokes a model download.

## Request boundary

After independent review of a successful native Qwen runtime job, the owner may add
only `.github/requests/qwen-inference-proof.json` in a new commit whose sole parent
is that exact tested source. The push must advance only
`csic21/luma-subtitle:feat/optional-asr-engines` from the tested source to that one
request commit. The parent must not already contain a request. The branch-only
push path trigger avoids GitHub's whole-PR path-diff behavior.

The request schema is enforced by `validate_request.cjs`: schema 1; purpose
`one-time-feature-branch-qwen-inference-proof`; required selector `full-inference`
or `readme-diagnostic`; repository/feature branch above;
40-hex source SHA; one of the two Qwen CPU pack IDs; exact prior native run ID,
run attempt, job ID and independently reviewed proof JSON SHA-256; the two fixed
model revisions in that script; model bytes 3,720,689,099; JFK audio SHA-256;
minimum available RAM 13,958,643,712 bytes; minimum free disk 12,884,901,888 bytes;
inference timeout 900 seconds; downloader deadline 600 seconds; metadata_only true.
The request must be a regular Git blob with mode 100644 and a non-symlink file.
Unknown or missing selectors, unknown fields, a merge, a bundled push, a previous
request, or different pins fail. A `readme-diagnostic` request additionally requires
exactly the `diagnostic` fields exported as `README_REQUEST` by the validator:
component `qwen3-asr-0-6b`, its fixed revision, path `README.md`, expected bytes and
maximum bytes both 57,456, the pinned README SHA-256, and deadline 30 seconds.
`full-inference` rejects that diagnostic object. The full-pair identity and
resource/budget fields remain required and unchanged for both selectors; a
README selector does not authorize downloading those model or audio files.

Native evidence may come from an explicitly pinned successful retry. It must be
for this repository's PR 4, the correct source/branch, exact runtime job and proof
step, and `.github/workflows/asr-components.yml`. The immutable `run.head_sha` and
`job.head_sha` establish identity. Historical nested PR head SHAs are mutable and
are deliberately not used. This control checks public, read-only run/job metadata;
it does not claim to download or independently reconstruct the old report digest.
The owner verifies that digest before creating the request. The selected complete
native proof is repeated before model download only for `full-inference`. The
README-only diagnostic checks the same prior native provenance but cannot enter
native assembly or model inference.

Both one-time selectors reject Actions re-runs. A failed or skipped
attempt is reported for a fresh decision; it cannot silently repeat a multi-GB
fetch. Success of the README diagnostic also cannot trigger a full attempt: that
requires a separately reviewed fresh request, with the previous request retired
before the new tested-source parent. Retiring a request skips the model job.
No release writes or new credentials are created. The checkout token is contents-read-only.

## README-only diagnostic

The `readme-diagnostic` selector reads only the first downloader target, the ASR
README at revision `5eb144179a02acc5e5ba31e748d22b0cf3e303b0`, with SHA-256
`5058416891bc47a2051557765997e8c42f8eb78a0e33c3e775bd17d4b0ba4d50`.
It does not prepare a runtime, measure the full-attempt resource gate, read the
JFK audio, download weights, or call inference. There is no fallthrough to those
operations. The README content is hashed and discarded rather than written or
uploaded.

The child has a hard 30-second aggregate deadline, the existing 5-second bounded
kill/reap, and an 8,192-byte combined stdout/stderr limit. Payload reads are capped
at 57,456 bytes. The final response must declare exactly that Content-Length,
without transfer encoding or compression, and match the fixed hash. Missing or
ambiguous lengths fail closed. Only the existing official HTTPS origins/CDNs are
allowed; proxy settings and credentials are not inherited. Redirect responses
are closed without reading their bodies, use the same 2-repeat/8-destination loop
limits, and carry fresh GET requests without forwarded credentials or authority.
Every redirect tightens the socket timeout to the remaining aggregate deadline.

A `passed-readme-only` result establishes only the current exact README read and
catalog pin. It does not recover the original lost downloader failure, establish
weight-CDN access, download model files, or validate ASR/alignment. Failure evidence
contains fixed categories, exact pinned file identity and bounded status/counts,
without response bodies, URLs, queries, tokens or environment contents.

## Native command and full-inference limits

The workflow checks out the tested source and invokes:

```
python -B -X utf8 scripts/asr-components/qwen-inference/run_proof.py \
  --source-sha EXACT_TESTED_SOURCE \
  --pack qwen3-asr-cpu-macos-arm64 \
  --request-copy ABSOLUTE_VALIDATED_REQUEST_COPY \
  --output ABSOLUTE_FRESH_OUTPUT
```

Windows uses `qwen3-asr-cpu-windows-x64`. The validated request selects the mode;
there is no default. For `full-inference`, the script first repeats deterministic
private assembly, relocation/import checks and actual Rust install/repair/cancel/
remove. Windows also requires native DLL closure, Japanese Unicode compatibility
and the real GUI-subsystem trust-helper proof. It then reconstructs just one private
runtime from the same exact artifact hashes and removes its duplicate ZIP.

Only then does private psutil 7.2.2 measure available physical RAM and free disk.
A measurement failure or values below 13 GiB / 12 GiB yields an explicit skip with
no model files downloaded. The existing native runtime and its verified input cache
are included in the occupied disk space; resource requirements are not hidden.

After a passing gate, the downloader's owned child fetches only the two catalog-pinned
0.6B models plus the public-domain JFK audio. It verifies exact bytes/SHA-256,
permits HTTPS official HF/CDN and raw GitHub hosts only, inherits no proxy settings,
and is killed/reaped on its hard 600-second deadline. The isolated offline worker
then gets one CPU cold ASR+forced-alignment request, a 900-second timeout, and a
2 MiB combined stdout/stderr cap enforced through file-backed owned handles.
Success requires the expected transcript content and ordered in-duration timestamps.
No success is inferred from imports, tensors, tokenization, or a resource-gate skip.

Only bounded proof JSON and, when produced, the fresh native-readiness JSON are
uploaded. Failure writes a report. Normal completion and confirmed child
termination remove private fixture/model/runtime outputs. If downloader reap or
the owned native Windows Job drain is unconfirmed, their request/input/output
directories are preserved and the report explicitly marks cleanup as failed; it
does not claim those outputs were removed. Binaries, README content and model
weights are never uploaded.
No local-cloud model download is authorized by these scripts' existence.

## Download-free tests

```
node --test scripts/asr-components/qwen-inference/validate_request.node-test.cjs
python -B -m unittest discover -s scripts/asr-components/qwen-inference -p 'test_*.py' -v
```
