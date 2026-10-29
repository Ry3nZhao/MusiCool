# Music_Cool - Build Only
# 构建 Tauri 发布版本（解码器为纯 Rust，无需单独编译），不运行程序
# 示例: .\scripts\develop\build_only.ps1 -Clean

param(
    [switch]$Clean = $false
)

$ErrorActionPreference = "Stop"

$ScriptPath  = $MyInvocation.MyCommand.Path
$ScriptDir   = Split-Path -Parent $ScriptPath
$ScriptsDir  = Split-Path -Parent $ScriptDir
$ProjectRoot = Split-Path -Parent $ScriptsDir

Write-Host "=== Music_Cool - Build Tauri App ===" -ForegroundColor Cyan
Write-Host "Project Directory: $ProjectRoot" -ForegroundColor Gray
Write-Host ""

foreach ($tool in @("cargo", "npm")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Write-Host "Error: $tool not found" -ForegroundColor Red
        exit 1
    }
}

$tauriDir    = Join-Path $ProjectRoot "src\tauri"
$frontendDir = Join-Path $tauriDir "frontend"

Push-Location $tauriDir
try {
    if ($Clean) {
        Write-Host "Cleaning Rust build directory..." -ForegroundColor Yellow
        cargo clean
    }

    if (-not (Test-Path (Join-Path $frontendDir "node_modules"))) {
        Write-Host "Installing frontend dependencies..." -ForegroundColor Yellow
        npm install --prefix $frontendDir
        if ($LASTEXITCODE -ne 0) { exit 1 }
    }

    $localTauriCli = Join-Path $frontendDir "node_modules\.bin\tauri.cmd"
    if (Test-Path $localTauriCli) {
        & $localTauriCli build
    } else {
        npm run --prefix frontend tauri -- build
    }
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Error: Tauri build failed" -ForegroundColor Red
        exit 1
    }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Build completed successfully." -ForegroundColor Green
Write-Host "Bundles: $tauriDir\target\release\bundle\" -ForegroundColor Gray
