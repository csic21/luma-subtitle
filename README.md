# Luma Subtitle

Luma Subtitle 是一个面向桌面端的视频字幕生成与翻译工具。它把字幕制作流程收拢到一个本地应用里：导入视频后，先用 FFmpeg 抽取音频，再调用本地 whisper.cpp 生成原文字幕，最后通过 OpenAI 兼容的 `/v1/chat/completions` 接口翻译并输出标准 SRT 文件。

项目目标是让个人创作者、课程剪辑、访谈整理和跨语言内容制作可以少折腾命令行，把视频转写、翻译、模型准备、依赖检查和任务输出放在同一个工作台里完成。

## 核心能力

- 视频导入与任务管理：选择视频、输出目录、源语言、目标语言和翻译配置后启动任务。
- 本地转写：使用 whisper.cpp 在本机完成语音识别，支持选择本地 Whisper 模型文件。
- 字幕翻译：兼容 OpenAI 风格 Chat Completions API，可配置 Base URL、模型名和 API Key。
- SRT 输出：每次任务生成原文字幕和目标语言字幕，便于直接导入剪辑软件或播放器。
- 环境面板：检查 FFmpeg、whisper.cpp、模型目录和依赖目录，支持下载 Windows 侧依赖与模型预设。
- 跨平台方向：当前重点支持 Windows x64 与 macOS Apple Silicon。

## 隐私与凭据

- 视频、音频抽取和 whisper.cpp 转写在本机执行。
- 翻译阶段会把待翻译的字幕文本发送到你配置的 OpenAI 兼容接口。
- API Key 通过系统凭据存储读取和保存，不应写入代码、日志或 `.env` 文件。
- 仓库不应提交本地模型、FFmpeg/whisper 二进制、任务中间文件、开发日志或个人配置。

## 技术栈

- Tauri 2
- React 18
- TypeScript
- Vite
- Rust
- whisper.cpp
- FFmpeg

## 支持平台

- Windows x64：推荐使用 CUDA 版 whisper.cpp。
- macOS Apple Silicon：使用本机已安装或随应用内置的 arm64 `ffmpeg` 与 `whisper-cli`，whisper.cpp 需使用 Metal 后端进行 GPU 加速。

暂不适配 Intel Mac。

## 开发环境

- Node.js 20+
- pnpm 9+
- Rust 1.80+
- Windows：NVIDIA 驱动与 CUDA 可用 GPU
- macOS Apple Silicon：推荐安装 Xcode Command Line Tools，便于自行准备或验证本地依赖

## 安装依赖

```powershell
pnpm install
```

## 准备本地依赖

应用内“环境”面板会显示固定的“依赖目录”和“模型目录”。

点击“下载到依赖目录”会：

- Windows：下载并解压 `ffmpeg.exe` 与 CUDA/CPU 版 `whisper-cli.exe`。
- macOS Apple Silicon：优先使用本机已安装或内置的成品依赖；不会调用 Homebrew，也不会自动下载非官方 macOS 二进制。

macOS 上游限制：

- whisper.cpp 官方 release 当前提供 Windows CLI zip 和 Apple `xcframework`，但没有可直接执行的 Apple Silicon `whisper-cli` 成品包。
- FFmpeg 官方只发布源码，macOS 可执行文件来自外部构建者或发行渠道。

因此 macOS 依赖需要由发布流程预先放进应用资源，或由用户本机提前安装到 PATH。开发时可把可执行文件放到：

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

## 输出

每次任务会生成：

- `{video_name}.source.srt`
- `{video_name}.{target_language}.srt`

任务中间文件会写入输出目录下的 `.luma-subtitle-work`。

## 仓库卫生

提交前建议确认：

- 不提交 `.env`、`.env.*`、私钥、证书、token 或真实 API Key。
- 不提交 `node_modules/`、`dist/`、`src-tauri/target/`。
- 不提交本地开发日志、布局检查截图、Whisper 模型和 FFmpeg/whisper 二进制。
