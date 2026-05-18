$ErrorActionPreference = "Stop"

Write-Host "Node:" -ForegroundColor Cyan
node --version

Write-Host "pnpm:" -ForegroundColor Cyan
pnpm --version

Write-Host "Rust:" -ForegroundColor Cyan
cargo --version

Write-Host "NVIDIA:" -ForegroundColor Cyan
try {
  nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
} catch {
  Write-Host "Not detected; Windows will use BLAS/CPU whisper.cpp." -ForegroundColor Yellow
}

Write-Host "Sidecars:" -ForegroundColor Cyan
$bin = Join-Path $PSScriptRoot "..\src-tauri\resources\bin"
Get-ChildItem $bin -Force | Select-Object Name,Length
