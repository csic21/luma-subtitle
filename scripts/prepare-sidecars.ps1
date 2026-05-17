$ErrorActionPreference = "Stop"

$bin = Resolve-Path (Join-Path $PSScriptRoot "..\src-tauri\resources\bin")

Write-Host "Place ffmpeg.exe and CUDA-enabled whisper-cli.exe in:" -ForegroundColor Cyan
Write-Host $bin
Write-Host ""
Write-Host "Expected files:" -ForegroundColor Cyan
Write-Host "- ffmpeg.exe"
Write-Host "- whisper-cli.exe"
Write-Host ""
Write-Host "After copying them, run:" -ForegroundColor Cyan
Write-Host "pnpm tauri:dev"
