# Luma Subtitle

[简体中文](README.zh-CN.md)

Luma Subtitle is a desktop app for generating, translating, and exporting video subtitles. After you import a video, the app extracts audio with FFmpeg, transcribes it locally with whisper.cpp, translates the subtitles through an OpenAI-compatible `/v1/chat/completions` API, and exports standard SRT files.

It is built for individual creators, course editors, interview workflows, and multilingual content production. Transcription, subtitle translation, model setup, dependency checks, task queues, and exports all live in one local desktop workspace.

The macOS build targets Apple Silicon. Automatic FFmpeg and whisper.cpp builds require macOS 11.0 or later, Xcode Command Line Tools, and `cmake`; whisper.cpp is built with the Metal backend.

## Screenshot

![Luma Subtitle homepage](screenshots/homepage.jpg)

## Features

- Video import and task management: choose video files, output folders, source language, target language, and translation settings.
- Local transcription: run whisper.cpp on your machine and select local Whisper model files.
- Subtitle proofreading: search, jump to cue IDs, compare source and translation, and edit individual source or translated cues without changing IDs or timing. Source edits require retranslation; translation edits require re-export. Existing exported files are preserved.
- Versioned results: retranscription or translation configuration changes invalidate old translations, and the preview refreshes for each result revision. Running a task saves any pending task settings first.
- Repeated filler cleanup: within each subtitle cue, recognized filler runs of six or more repetitions are shortened to three plus an ellipsis before translation, preserving dialogue and timestamps.
- Subtitle translation: use an OpenAI-compatible Chat Completions API, a local Hy-MT2 GGUF model through llama.cpp, or a local CLI such as opencode.
- SRT export: generate source and translated subtitle files for editors, players, and subtitle tooling.
- Task queue: batch transcribe, translate, and export; optionally enable automatic chaining from transcription to translation to export.
- Environment panel: check FFmpeg, whisper.cpp, model folders, dependency folders, and download supported presets.
- Platform focus: Windows x64 and macOS Apple Silicon.

## Privacy And Credentials

- Video processing, audio extraction, and whisper.cpp transcription run locally.
- API translation sends subtitle text to the OpenAI-compatible endpoint configured by the user. Local-model translation runs llama.cpp and Hy-MT2 GGUF files on the same machine.
- API keys are stored in the local SQLite database under the app user data directory.
- Do not commit local models, FFmpeg/whisper binaries, task artifacts, development logs, personal settings, or API keys.

## Tech Stack

- Tauri 2
- React 18
- TypeScript
- Vite
- Rust
- whisper.cpp
- llama.cpp
- FFmpeg

## Supported Platforms

- Windows x64: the app selects a CUDA whisper.cpp package when an NVIDIA GPU is available, otherwise it uses a BLAS/CPU package. Local translation installs official llama.cpp builds: CUDA 12 when NVIDIA is present, otherwise Vulkan, then CPU.
- macOS Apple Silicon: the app first uses installed or bundled arm64 `ffmpeg` and `whisper-cli`; if missing, it can build FFmpeg and Metal-enabled whisper.cpp from official source archives. Local translation downloads the official Metal llama.cpp macOS arm64 build.

Intel Mac is not currently supported.

## Development Requirements

- Node.js 20+
- pnpm 9+
- Rust 1.80+
- Windows: NVIDIA driver and CUDA-capable GPU optional for faster transcription
- macOS Apple Silicon: macOS 11.0+, Xcode Command Line Tools, and `cmake`

## Install Dependencies

```powershell
pnpm install
```

## Prepare Local Runtime Dependencies

The in-app Environment panel shows the fixed dependency folder and model folder.

Clicking "Download to dependency folder" will:

- Windows: download and extract `ffmpeg.exe` and the best matching CUDA, BLAS, or CPU `whisper-cli.exe`.
- macOS Apple Silicon: use installed or bundled dependencies first; if missing, download official source archives and build locally. It does not call Homebrew or download unofficial macOS binaries.

macOS automatic build requirements:

- Apple Silicon device running macOS 11.0 or later.
- Xcode Command Line Tools installed, with `clang`, `make`, `tar`, and `sh` available.
- `cmake` installed for configuring and building whisper.cpp.
- Network access to FFmpeg official source archives and `ggml-org/whisper.cpp` GitHub release source archives.

macOS build sources and configuration:

- FFmpeg is downloaded from `https://ffmpeg.org/releases/` and built locally with `VideoToolbox`, `AudioToolbox`, and `AVFoundation` enabled.
- whisper.cpp is downloaded from official `ggml-org/whisper.cpp` GitHub release source archives and built with CMake using `GGML_METAL=ON`.
- whisper.cpp explicitly uses the `macOS 11.0` deployment target for Apple Silicon and C++17 `std::filesystem` compatibility.

To skip in-app compilation, pre-bundle dependencies in the release process or install them to the user's PATH. During development, you can also place executables at:

- `src-tauri/resources/bin/macos-arm64/ffmpeg`
- `src-tauri/resources/bin/macos-arm64/whisper-cli`

Release builds bundle files under `src-tauri/resources`. macOS executables must keep executable permissions:

```zsh
chmod +x src-tauri/resources/bin/macos-arm64/ffmpeg
chmod +x src-tauri/resources/bin/macos-arm64/whisper-cli
```

Lookup order: app data directory, bundled resources, common macOS executable paths, then system PATH.

Whisper models can live anywhere. Select the model file in the app. On Apple Silicon, `large-v3-turbo-q5_0` or `small` are good starting points depending on memory and speed requirements.

Local translation models are downloaded the same way into the app `models` folder. They are not bundled in the installer.

| Preset | File | Size | Download |
| --- | --- | --- | --- |
| Hy-MT2 1.8B Q4 | `Hy-MT2-1.8B-Q4_K_M.gguf` | 1.1 GB | https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF/resolve/main/Hy-MT2-1.8B-Q4_K_M.gguf |
| Hy-MT2 7B Q4 | `Hy-MT2-7B-Q4_K_M.gguf` | 4.3 GB | https://huggingface.co/tencent/Hy-MT2-7B-GGUF/resolve/main/Hy-MT2-7B-Q4_K_M.gguf |

In Settings, choose Translation provider → Local model, then Install local translation. That installs `llama-server` into the dependency folder and downloads the 1.8B preset if no GGUF is selected.

## Whisper Model Presets

The in-app model presets download into the app data directory's `models` folder and automatically update the selected Whisper model path. You can also download these files manually and select them in the app:

| Preset | File | Size | Download |
| --- | --- | --- | --- |
| tiny | `ggml-tiny.bin` | 75 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin |
| base | `ggml-base.bin` | 142 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin |
| small | `ggml-small.bin` | 466 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin |
| large-v3-turbo-q5_0 | `ggml-large-v3-turbo-q5_0.bin` | 547 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin |

## Optional ASR Engines (Experimental)

Whisper.cpp remains the default, including the existing Turbo preset above. Existing models, task results, and settings do not require Python or migration steps.

Settings and per-task configuration can explicitly select a persistent local MLX/faster-whisper worker, or Qwen3-ASR 0.6B/1.7B with its forced aligner. Install the optional private engine component and choose separate model downloads inside Luma; no system Python, terminal commands or PATH changes are required. Downloads start only when requested, show their sizes, and are verified before activation. Managed components provide Windows x64 Whisper CPU, Apple Silicon MLX Whisper/Metal, and Qwen CPU on both platforms. CUDA is not included in these components; advanced external runtimes remain optional.

See the [offline setup guide](docs/OPTIONAL_ASR.md) for managed setup, model formats, storage planning, recovery and optional advanced external-runtime examples. The same guide is bundled in the settings screen. Optional engines are unbenchmarked in Luma and should be validated with your own sample audio before use. They do not replace the existing default.

## Run

```powershell
pnpm tauri:dev
```

The same command works on macOS.

## Build

```powershell
pnpm tauri:build
```

For production macOS distribution, handle `.icns` icons, codesigning, and notarization on a macOS machine.

## App Updates

Luma Subtitle uses the official Tauri updater plugin. Release builds publish signed update artifacts and `latest.json` to GitHub Releases.

Generate the updater signing key once:

```zsh
pnpm tauri signer generate -w ~/.tauri/luma-subtitle.key
```

Store the private key content in the GitHub secret `TAURI_SIGNING_PRIVATE_KEY`. If you protect the key with a password, store it in `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The public key is committed in `src-tauri/tauri.conf.json`.

The app checks:

```text
https://github.com/csic21/luma-subtitle/releases/latest/download/latest.json
```

## Output

Each task can generate:

- `{video_name}.source.srt`
- `{video_name}.{target_language}.srt`

Intermediate task files are written under `.luma-subtitle-work` inside the output folder.

## Repository Hygiene

Before committing, make sure you do not include:

- `.env`, `.env.*`, private keys, certificates, tokens, or real API keys.
- `node_modules/`, `dist/`, or `src-tauri/target/`.
- Local development logs, layout-check screenshots, Whisper models, or FFmpeg/whisper binaries.

## License

Luma Subtitle is licensed under the GNU General Public License v3.0 or later. See [LICENSE](LICENSE) for details.
