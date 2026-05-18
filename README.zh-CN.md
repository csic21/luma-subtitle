# Luma Subtitle

[English](README.md)

Luma Subtitle 是一款桌面端视频字幕生成与翻译工具。导入视频后，应用会通过 FFmpeg 抽取音频，调用本地 whisper.cpp 生成原文字幕，再使用 OpenAI 兼容的 `/v1/chat/completions` 接口完成翻译并输出标准 SRT 文件。

项目面向个人创作者、课程剪辑、访谈整理和跨语言内容制作场景，将视频转写、字幕翻译、模型准备、依赖检查、任务队列和任务输出集中在同一个本地工作台中完成。

macOS 版本面向 Apple Silicon 设备。自动编译 FFmpeg 和 whisper.cpp 需要 macOS 11.0 或更高版本、Xcode Command Line Tools 和 `cmake`；whisper.cpp 会以 Metal 后端构建。

## 核心能力

- 视频导入与任务管理：选择视频、输出目录、源语言、目标语言和翻译配置后启动任务。
- 本地转写：使用 whisper.cpp 在本机完成语音识别，支持选择本地 Whisper 模型文件。
- 字幕翻译：兼容 OpenAI 风格 Chat Completions API，可配置 Base URL、模型名和 API Key。
- SRT 输出：每次任务生成原文字幕和目标语言字幕，便于直接导入剪辑软件或播放器。
- 任务队列：支持批量转写、翻译和导出，也可以开启从转写到翻译再到导出的自动链路。
- 环境面板：检查 FFmpeg、whisper.cpp、模型目录和依赖目录，支持下载运行依赖与模型预设。
- 平台支持：当前重点支持 Windows x64 与 macOS Apple Silicon。

## 隐私与凭据

- 视频、音频抽取和 whisper.cpp 转写在本机执行。
- 翻译阶段会将待翻译的字幕文本发送到用户配置的 OpenAI 兼容接口。
- API Key 保存在应用用户数据目录的本地 SQLite 数据库中。
- 仓库不应提交本地模型、FFmpeg/whisper 二进制、任务中间文件、开发日志、个人配置或 API Key。

## 技术栈

- Tauri 2
- React 18
- TypeScript
- Vite
- Rust
- whisper.cpp
- FFmpeg

## 支持平台

- Windows x64：检测到 NVIDIA GPU 时使用 CUDA 版 whisper.cpp，否则使用 BLAS/CPU 版。
- macOS Apple Silicon：优先使用本机已安装或随应用内置的 arm64 `ffmpeg` 与 `whisper-cli`；缺失时可从官方源码自动编译 FFmpeg 与 Metal 版 whisper.cpp。

暂不适配 Intel Mac。

## 开发环境

- Node.js 20+
- pnpm 9+
- Rust 1.80+
- Windows：NVIDIA 驱动与 CUDA 可用 GPU 可用于加速转写，非必需
- macOS Apple Silicon：macOS 11.0+、Xcode Command Line Tools、`cmake`

## 安装依赖

```powershell
pnpm install
```

## 准备本地依赖

应用内“环境”面板会显示固定的“依赖目录”和“模型目录”。

点击“下载到依赖目录”会：

- Windows：下载并解压 `ffmpeg.exe` 与最匹配当前环境的 CUDA、BLAS 或 CPU 版 `whisper-cli.exe`。
- macOS Apple Silicon：优先使用本机已安装或内置的成品依赖；缺失时下载官方源码并在本机编译，不调用 Homebrew，也不下载非官方 macOS 二进制。

macOS 自动编译条件：

- Apple Silicon 设备，macOS 11.0 或更高版本。
- 已安装 Xcode Command Line Tools，确保 `clang`、`make`、`tar` 和 `sh` 可用。
- 已安装 `cmake`，用于配置和构建 whisper.cpp。
- 构建过程需要访问 FFmpeg 官方源码包和 `ggml-org/whisper.cpp` GitHub release 源码包。

macOS 自动编译来源与配置：

- FFmpeg 从 `https://ffmpeg.org/releases/` 下载官方源码包，本机编译时启用 `VideoToolbox`、`AudioToolbox` 和 `AVFoundation`。
- whisper.cpp 从 `ggml-org/whisper.cpp` 官方 GitHub release 源码包下载，本机通过 CMake 编译启用 `GGML_METAL=ON` 的 `whisper-cli`。
- whisper.cpp 构建会显式使用 `macOS 11.0` deployment target，以满足 Apple Silicon 和 C++17 `std::filesystem` 的系统要求。

如需跳过应用内编译，可以在发布流程中预置依赖，或在用户本机提前安装到 PATH。开发时也可以把可执行文件放到：

- `src-tauri/resources/bin/macos-arm64/ffmpeg`
- `src-tauri/resources/bin/macos-arm64/whisper-cli`

发布构建会打包 `src-tauri/resources` 下的内容。macOS 可执行文件需要保留执行权限：

```zsh
chmod +x src-tauri/resources/bin/macos-arm64/ffmpeg
chmod +x src-tauri/resources/bin/macos-arm64/whisper-cli
```

应用查找顺序：应用数据目录、内置资源、常见 macOS 可执行路径、系统 PATH。

Whisper 模型可以放在任意位置，在应用内选择模型文件即可。Apple Silicon 推荐优先使用 `large-v3-turbo-q5_0` 或 `small`，根据内存和转写速度取舍。

## Whisper 模型预设

应用内“模型预设”会自动下载到应用数据目录的 `models` 文件夹，并自动写入“Whisper 模型”路径。也可以手动下载以下文件后在应用内选择：

| 预设 | 文件 | 大小 | 下载 |
| --- | --- | --- | --- |
| tiny | `ggml-tiny.bin` | 75 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin |
| base | `ggml-base.bin` | 142 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin |
| small | `ggml-small.bin` | 466 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin |
| large-v3-turbo-q5_0 | `ggml-large-v3-turbo-q5_0.bin` | 547 MiB | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin |

## 运行

```powershell
pnpm tauri:dev
```

macOS 上也可以直接运行同一条命令。

## 打包

```powershell
pnpm tauri:build
```

正式分发 macOS 包时，还需要在 macOS 机器上处理 `.icns` 图标、codesign 和 notarization。

## 应用更新

Luma Subtitle 使用官方 Tauri updater 插件。发布构建会把签名更新包和 `latest.json` 上传到 GitHub Releases。

首次配置时生成 updater 签名密钥：

```zsh
pnpm tauri signer generate -w ~/.tauri/luma-subtitle.key
```

将私钥内容保存为 GitHub Secret `TAURI_SIGNING_PRIVATE_KEY`。如果生成密钥时设置了密码，将密码保存为 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。公钥已提交到 `src-tauri/tauri.conf.json`。

应用检查的更新清单地址：

```text
https://github.com/csic21/luma-subtitle/releases/latest/download/latest.json
```

## 输出

每次任务可以生成：

- `{video_name}.source.srt`
- `{video_name}.{target_language}.srt`

任务中间文件会写入输出目录下的 `.luma-subtitle-work`。

## 仓库卫生

提交前建议确认：

- 不提交 `.env`、`.env.*`、私钥、证书、token 或真实 API Key。
- 不提交 `node_modules/`、`dist/`、`src-tauri/target/`。
- 不提交本地开发日志、布局检查截图、Whisper 模型和 FFmpeg/whisper 二进制。
