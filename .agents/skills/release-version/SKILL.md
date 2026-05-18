---
name: release-version
description: Use when releasing Luma Subtitle, bumping the app version, creating release commits, creating v* tags, pushing to origin, or triggering the repository Release GitHub Action.
---

# Luma Subtitle Release

Use this skill when the user asks to "发版", "发个新版本", "release", "publish a version", "trigger action", or similar.

## Release Workflow

1. Confirm the repo state:

```bash
rtk proxy git status --short
rtk proxy git branch --show-current
rtk sed -n '1,120p' .github/workflows/release.yml
```

2. Determine the next version. Default to a patch bump unless the user specifies otherwise.

3. Update the project version in all four places:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- the `luma-subtitle` package entry only in `src-tauri/Cargo.lock`

Do not replace every matching dependency version in `Cargo.lock`.

4. Validate locally:

```bash
rtk pnpm build
rtk pnpm tauri:build
```

5. Commit the version bump:

```bash
rtk proxy git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
rtk proxy git commit -m "chore: release X.Y.Z"
```

6. Trigger the Release workflow. This repository's Release Action runs on `v*` tags, not ordinary branch pushes.

```bash
rtk proxy git push origin <branch>
rtk proxy git tag vX.Y.Z
rtk proxy git push origin vX.Y.Z
```

If the release commit was already pushed, still create and push the missing `vX.Y.Z` tag at that commit.

7. Report the local artifacts and remote trigger:

- local DMG: `src-tauri/target/release/bundle/dmg/Luma Subtitle_X.Y.Z_aarch64.dmg`
- local app bundle: `src-tauri/target/release/bundle/macos/Luma Subtitle.app`
- pushed branch and tag

