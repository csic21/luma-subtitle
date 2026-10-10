# One-time, bounded Qwen inference evidence

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
`one-time-feature-branch-qwen-inference-proof`; repository/feature branch above;
40-hex source SHA; one of the two Qwen CPU pack IDs; exact prior native run ID,
run attempt, job ID and independently reviewed proof JSON SHA-256; the two fixed
model revisions in that script; model bytes 3,720,689,099; JFK audio SHA-256;
minimum available RAM 13,958,643,712 bytes; minimum free disk 12,884,901,888 bytes;
inference timeout 900 seconds; downloader deadline 600 seconds; metadata_only true.
The request must be a regular Git blob with mode 100644 and a non-symlink file.
Unknown fields, a merge, a bundled push, a previous request, or different pins fail.

Native evidence may come from an explicitly pinned successful retry. It must be
for this repository's PR 3, the correct source/branch, exact runtime job and proof
step, and `.github/workflows/asr-components.yml`. The immutable `run.head_sha` and
`job.head_sha` establish identity. Historical nested PR head SHAs are mutable and
are deliberately not used. This control checks public, read-only run/job metadata;
it does not claim to download or independently reconstruct the old report digest.
The owner verifies that digest before creating the request. The selected complete
native proof is repeated before any model download.

The one-time model request itself rejects Actions re-runs. A failed or skipped
attempt is reported for a fresh decision; it cannot silently repeat a multi-GB
fetch. No release writes or new credentials are created. The checkout token is contents-read-only.

## Native command and limits

The workflow checks out the tested source and invokes:

```
python -B -X utf8 scripts/asr-components/qwen-inference/run_proof.py \
  --source-sha EXACT_TESTED_SOURCE \
  --pack qwen3-asr-cpu-macos-arm64 \
  --request-copy ABSOLUTE_VALIDATED_REQUEST_COPY \
  --output ABSOLUTE_FRESH_OUTPUT
```

Windows uses `qwen3-asr-cpu-windows-x64`. The script first repeats deterministic
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

Only bounded proof JSON and the fresh native-readiness JSON are uploaded. Failure
at any stage writes a failure report and removes private model/runtime outputs; binaries and model weights are never uploaded.
No local-cloud model download is authorized by these scripts' existence.

## Download-free tests

```
node --test scripts/asr-components/qwen-inference/validate_request.node-test.cjs
python -B -m unittest discover -s scripts/asr-components/qwen-inference -p 'test_*.py' -v
```
