#!/usr/bin/env bash
# Music_Cool - Run Tauri Dev Only
# 仅运行 Tauri 开发环境（不清理构建缓存）
# 示例: ./scripts/develop/run_only.sh

set -e  # 遇到错误立即退出

# 获取脚本目录（兼容 zsh 和 bash）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-${(%):-%x}}")" && pwd)"
SCRIPTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$SCRIPTS_DIR/.." && pwd)"

echo "=== Music_Cool - Run Tauri Dev Only ==="
echo "Project Directory: $PROJECT_ROOT"
echo ""

# -------------------------------------------------------------
# 1. 检查并自动安装前端依赖
# -------------------------------------------------------------
FRONTEND_DIR="$PROJECT_ROOT/src/tauri/frontend"
FRONTEND_PKG="$FRONTEND_DIR/package.json"
FRONTEND_NODE_MODULES="$FRONTEND_DIR/node_modules"

# 检查 Node.js
if ! command -v node &> /dev/null; then
    echo "Error: Node.js not found" >&2
    
    # 检查 nvm
    if [[ -s "$HOME/.nvm/nvm.sh" ]]; then
        echo "nvm found. Attempting to use nvm..." >&2
        source "$HOME/.nvm/nvm.sh"
        
        if command -v node &> /dev/null; then
            echo "Node.js found via nvm" >&2
        else
            echo "Installing Node.js LTS via nvm..." >&2
            nvm install --lts
            nvm use --lts
        fi
    else
        echo "Please install Node.js:" >&2
        echo "  Install nvm: curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.0/install.sh | bash" >&2
        echo "  Or install Node.js directly: https://nodejs.org/" >&2
        exit 1
    fi
fi

NODE_VERSION=$(node --version)
echo "Using Node.js: $NODE_VERSION"

# 检查 npm
if ! command -v npm &> /dev/null; then
    echo "Error: npm not found" >&2
    exit 1
fi

NPM_VERSION=$(npm --version)
echo "Using npm: $NPM_VERSION"
echo ""

# 检查 Rust (cargo)
if ! command -v cargo &> /dev/null; then
    echo "Error: cargo (Rust) not found" >&2
    echo "Please install Rust from https://rustup.rs" >&2
    echo "  Or run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    exit 1
fi

CARGO_VERSION=$(cargo --version)
echo "Using Rust: $CARGO_VERSION"
echo ""

# 安装前端依赖
if [[ -f "$FRONTEND_PKG" ]]; then
    echo "Ensuring frontend dependencies are installed..."
    cd "$FRONTEND_DIR"
    
    if [[ ! -d "$FRONTEND_NODE_MODULES" ]]; then
        echo "Installing frontend dependencies..."
        npm install
        
        if [[ $? -ne 0 ]]; then
            echo "Error: npm install failed" >&2
            exit 1
        fi
    else
        echo "Frontend dependencies already installed"
    fi
    
    cd "$PROJECT_ROOT"
    echo ""
fi

# -------------------------------------------------------------
# 2. 启动 Tauri 开发环境
# -------------------------------------------------------------
echo "Starting Tauri dev..."

TAURI_DIR="$PROJECT_ROOT/src/tauri"
if [[ ! -d "$TAURI_DIR" ]]; then
    echo "Error: Tauri directory not found: $TAURI_DIR" >&2
    exit 1
fi

cd "$TAURI_DIR"

# 直接调用 frontend/node_modules 中的 Tauri CLI
LOCAL_TAURI_CLI="$FRONTEND_DIR/node_modules/.bin/tauri"

if [[ -f "$LOCAL_TAURI_CLI" ]]; then
    echo "Using local Tauri CLI: $LOCAL_TAURI_CLI"
    "$LOCAL_TAURI_CLI" dev
else
    echo "Warning: Local Tauri CLI not found at $LOCAL_TAURI_CLI" >&2
    echo "Falling back to npm run tauri..." >&2
    npm run --prefix frontend tauri -- dev
fi

cd "$PROJECT_ROOT"

