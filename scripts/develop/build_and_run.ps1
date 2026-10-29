# Music_Cool - Run Tauri Dev
# 启动 Tauri 开发环境（tauri dev 会自动编译 Rust 后端，含解码器）
# 示例: .\scripts\develop\build_and_run.ps1 [-Clean]

param(
    [switch]$Clean = $false
)

$ErrorActionPreference = "Stop"

$ScriptPath  = $MyInvocation.MyCommand.Path
$ScriptDir   = Split-Path -Parent $ScriptPath
$ScriptsDir  = Split-Path -Parent $ScriptDir
$ProjectRoot = Split-Path -Parent $ScriptsDir

Write-Host "=== Music_Cool - Build & Run Tauri Dev ===" -ForegroundColor Cyan
Write-Host "Project Directory: $ProjectRoot" -ForegroundColor Gray
Write-Host ""

if ($Clean) {
    Write-Host "Cleaning Rust build directory..." -ForegroundColor Yellow
    Push-Location (Join-Path $ProjectRoot "src\tauri")
    try { cargo clean } finally { Pop-Location }
    Write-Host ""
}

& "$ScriptDir\run_only.ps1"

if ($LASTEXITCODE -ne 0) {
    Write-Host "Error: Tauri dev failed" -ForegroundColor Red
    exit 1
}
