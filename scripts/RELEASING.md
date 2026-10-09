# Releasing Luma Subtitle

Release tags identify immutable source commits. The pipeline keeps the release in
draft until Windows x64 and macOS Apple Silicon tests and builds succeed, all seven
required assets are downloaded and checked, and the three updater signatures pass
the official `minisign` verifier against the public key in that source commit.
Only the final job writes `latest.json` and publishes the draft. A failed job leaves
the draft unpublished; fix the problem and rerun, or release a new version/source.
Never move an existing version tag to another commit.

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
