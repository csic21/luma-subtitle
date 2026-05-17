# Sidecar Binaries

Place platform binaries here for development.

The app can also install missing dependencies into its managed sidecar
directory. On macOS Apple Silicon it only downloads official source archives
and builds locally; it does not download third-party macOS binaries.

Windows x64:

- `ffmpeg.exe`
- `whisper-cli.exe`

macOS Apple Silicon:

- `macos-arm64/ffmpeg`
- `macos-arm64/whisper-cli`

The macOS binaries must be executable:

```zsh
chmod +x src-tauri/resources/bin/macos-arm64/ffmpeg
chmod +x src-tauri/resources/bin/macos-arm64/whisper-cli
```

Release builds bundle everything under `src-tauri/resources`.
