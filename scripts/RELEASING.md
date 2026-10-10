# Releasing Luma Subtitle

Release tags identify immutable source commits. The pipeline keeps the release in
draft until Windows x64 and macOS Apple Silicon tests and builds succeed, all seven
required assets are downloaded and checked, and the three updater signatures pass
the official `minisign` verifier against the public key in that source commit.
Only the final job writes `latest.json` and publishes the draft. A failed job leaves
the draft unpublished; fix the problem and rerun, or release a new version/source.
Never move an existing version tag to another commit.

## External final-source approval gate

Before merging the 1.2.0 optional-engine source or creating its application release
request, the release owner must approve all of the following for the exact final
candidate commit, after one-time Qwen controls are retired and documentation is
final:

- Both Windows x64 and macOS Apple Silicon app checks pass.
- All four native jobs in `asr-components.yml` pass: Windows Whisper CPU,
  Apple Silicon MLX Whisper, Windows Qwen CPU and Apple Silicon Qwen CPU.
- The Windows CPU report confirms execution and success of the receipt-selected
  Rust SRT/export, active cancellation/recovery and process/use-lease lifecycle
  proof, in addition to reference assembly/inference and installer checks. A
  skipped test or a reference-only smoke does not satisfy this gate.
- Independent review confirms the final source and exact run/attempt/job/report
  identities, including honest inference and UI limitations.

Record immutable source SHAs and exact evidence; an evolving PR head, older green
source, component publication or README-only diagnostic is not a substitute.
Global embedded-catalog managed path selection and full desktop UI/queue testing
remain unverified by the receipt-selected test-owned-root proof. Qwen/MLX speech
inference, performance and memory-fit claims require their own actual evidence.
Any subsequent source change requires checks and review for the new candidate.
After merge, verify that the release-source tree matches that approved candidate;
if it differs, obtain exact-source checks and review before requesting release.

This is an external approval gate. `release.yml` is unchanged and does not enforce
these four optional-runtime jobs or independent review. The application release
request must remain a separate request-only commit after approval; publication of
`asr-ct2-cpu-4.8.2-1` does not publish application 1.2.0 or advance its updater.

## Source-pinned request from main

1. Merge the release source (including matching package.json, tauri.conf.json,
   Cargo.toml and Cargo.lock versions and a CHANGELOG.md section) after CI passes.
2. Make a separate commit on main that changes **only**
   `.github/release-request.json`. Its only parent must be the release source.
3. The file must contain exactly these fields, with actual values:

   ```json
   {
     "schema_version": 1,
     "tag": "vX.Y.Z",
     "source_sha": "FULL_40_CHARACTER_PARENT_COMMIT_SHA"
   }
   ```

The workflow validates the complete changed trees, versions, current main head,
and any existing tag. It creates a matching tag with the existing ephemeral
`GITHUB_TOKEN`. The same run builds the source SHA: token-created tag events do
not start another workflow. No new token, persistent credential or repository
permission is needed. Do not put a release-request file in the implementation PR.

## Existing tag or manual release

A normal `v*` tag push still runs the pipeline. The manual workflow accepts an
existing version tag and optional release title/prerelease flag. The release notes
come from that source's CHANGELOG.md. Existing published releases are never edited;
a matching draft may be retried. An equal or older stable release cannot replace
GitHub's latest release. Manually publishing an incomplete draft is unsupported:
the previous `release: published` trigger was removed because it bypassed the gate.

## Artifacts and updater

Required files (where VERSION is the source version):

- Luma-Subtitle_VERSION_darwin_aarch64.dmg
- Luma-Subtitle_VERSION_darwin_aarch64.app.tar.gz and .app.tar.gz.sig
- Luma-Subtitle_VERSION_windows_x64-setup.exe and -setup.exe.sig
- Luma-Subtitle_VERSION_windows_x64.msi and .msi.sig

Tauri v2 uses the signed EXE directly for Windows updates. `latest.json` contains
`darwin-aarch64` and `windows-x86_64`, with exact release asset URLs and the original
base64 signature strings. Matrix jobs never write this manifest. Downloads use the
authenticated asset API while the release is still a draft; public asset URLs only
become available when publication succeeds. Signing secrets and platform targets
are unchanged. macOS distribution remains ad-hoc signed, without new notarization
or Developer ID credentials.

Run release guard tests with:

```sh
node --test scripts/prepare-release.test.cjs scripts/publish-release.test.cjs
```

The focused tests mock GitHub and signature results; actual installer builds and
production-key signature verification run on the release runners.

## Recover a built release without moving its tag

GitHub draft assets may use `untagged-<id>` browser URLs until publication. The
publisher validates their same-repository API identities and SHA-256 digests,
downloads by asset ID, and verifies all updater signatures. The updater manifest
always contains canonical version-tag URLs. After publication, asset metadata
must expose those canonical URLs without changing any verified byte identity.

When the application packages already built successfully but a controller check
failed, review and merge the controller fix first. Keep the app source, version
tag and all original binary assets unchanged. Then add a separate main commit
changing only `.github/release-recovery.json`, with exactly these fields:

- `schema_version`: 1
- `controller_sha`: the request commit's only parent, containing the reviewed fix
- `source_sha`: the immutable original application source commit
- `tag`: the existing application version tag
- `release_id`: the intended existing draft/release ID
- `build_run_id` and `build_run_attempt`: the original completed Release run/attempt
- `assets`: the seven original asset snapshots, each containing `name`, `id`,
  `size`, and `digest` (the full `sha256:...` value)

The new Release recovery workflow verifies the request-only tree, current main
head, original request/source tree, source versions, tag, release source marker,
repository/workflow/run identity and both successful platform jobs. It reads public
Actions provenance through fixed `api.github.com` URLs without sending credentials
or adding Actions/Workflows token permissions. Its only publication mutations are
creating/replacing a draft `latest.json` after verification and publishing that
same release. Binary assets and tags are never replaced. An interrupted empty
GitHub draft manifest upload may be cleaned up only after signatures pass.

An already-published exact match is fully reverified read-only; its existing date,
manifest and assets are never overwritten. Any mismatch fails closed. If GitHub
requires a permission unavailable to the existing token, stop and report the
specific permission rather than changing credentials or target metadata. Public
package URLs should also be independently downloaded and checked after publishing.

Run all controller tests with:

```sh
node --test scripts/prepare-release.test.cjs scripts/publish-release.test.cjs scripts/recover-release.node-test.cjs
```
