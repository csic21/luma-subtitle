$ErrorActionPreference = "Stop"

Write-Host "Node:" -ForegroundColor Cyan
node --version

Write-Host "pnpm:" -ForegroundColor Cyan
pnpm --version

Write-Host "Rust:" -ForegroundColor Cyan
cargo --version

Write-Host "NVIDIA:" -ForegroundColor Cyan
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader

Write-Host "Sidecars:" -ForegroundColor Cyan
$bin = Join-Path $PSScriptRoot "..\src-tauri\resources\bin"
Get-ChildItem $bin -Force | Select-Object Name,Length
