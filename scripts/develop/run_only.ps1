# Music_Cool - Run Tauri Dev Only
# 仅运行 Tauri 开发环境（不清理构建缓存）
# 示例: .\scripts\develop\run_only.ps1

$ErrorActionPreference = "Stop"

# 计算项目根目录
$ScriptPath  = $MyInvocation.MyCommand.Path
$ScriptDir   = Split-Path -Parent $ScriptPath
$ScriptsDir  = Split-Path -Parent $ScriptDir
$ProjectRoot = Split-Path -Parent $ScriptsDir

Write-Host "=== Music_Cool - Run Tauri Dev Only ===" -ForegroundColor Cyan
Write-Host "Project Directory: $ProjectRoot" -ForegroundColor Gray

# -------------------------------------------------------------
# 1. 检查并自动安装前端依赖
# -------------------------------------------------------------
$frontendDir = Join-Path $ProjectRoot "src\tauri\frontend"
$frontendPkg = Join-Path $frontendDir "package.json"
$frontendNodeModules = Join-Path $frontendDir "node_modules"

if (Test-Path $frontendPkg) {
    Write-Host "Ensuring frontend dependencies are installed..." -ForegroundColor Yellow
    Push-Location $frontendDir
    try {
        npm install
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: npm install failed" -ForegroundColor Red
            exit 1
        }
    } finally {
        Pop-Location
    }
}

# -------------------------------------------------------------
# 2. 启动 Tauri 开发环境
# -------------------------------------------------------------
Write-Host "Starting Tauri dev..." -ForegroundColor Green

$tauriDir = Join-Path $ProjectRoot "src\tauri"
if (-not (Test-Path $tauriDir)) {
    Write-Host "Error: Tauri directory not found: $tauriDir" -ForegroundColor Red
    exit 1
}

# 直接调用 frontend/node_modules 中的 Tauri CLI
# 必须在 src/tauri 目录下运行，以便正确读取 tauri.conf.json
Push-Location $tauriDir

$localTauriCli = Join-Path "frontend" "node_modules" | Join-Path -ChildPath ".bin" | Join-Path -ChildPath "tauri.cmd"

if (Test-Path $localTauriCli) {
    Write-Host "Using local Tauri CLI: $localTauriCli" -ForegroundColor Cyan
    & $localTauriCli dev
} else {
    Write-Host "Warning: Local Tauri CLI not found at $localTauriCli" -ForegroundColor Yellow
    Write-Host "Falling back to npm run tauri..." -ForegroundColor Yellow
    npm run --prefix frontend tauri -- dev
}

Pop-Location
