#!/usr/bin/env bash
# Music_Cool - Build Only
# 构建 Tauri 发布版本（解码器为纯 Rust，无需单独编译），不运行程序
# 示例: ./scripts/develop/build_only.sh --clean

set -e  # 遇到错误立即退出

# 获取脚本目录（兼容 zsh 和 bash）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-${(%):-%x}}")" && pwd)"
SCRIPTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_ROOT="$(cd "$SCRIPTS_DIR/.." && pwd)"

# 解析参数
CLEAN=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --clean)
            CLEAN=true
            shift
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Usage: $0 [--clean]" >&2
            exit 1
            ;;
    esac
done

echo "=== Music_Cool - Build Tauri App ==="
echo "Project Directory: $PROJECT_ROOT"
echo ""

for tool in cargo npm; do
    if ! command -v "$tool" &> /dev/null; then
        echo "Error: $tool not found" >&2
        exit 1
    fi
done

TAURI_DIR="$PROJECT_ROOT/src/tauri"
FRONTEND_DIR="$TAURI_DIR/frontend"

cd "$TAURI_DIR"

if [[ "$CLEAN" == true ]]; then
    echo "Cleaning Rust build directory..."
    cargo clean
fi

if [[ ! -d "$FRONTEND_DIR/node_modules" ]]; then
    echo "Installing frontend dependencies..."
    npm install --prefix "$FRONTEND_DIR"
fi

LOCAL_TAURI_CLI="$FRONTEND_DIR/node_modules/.bin/tauri"
if [[ -f "$LOCAL_TAURI_CLI" ]]; then
    "$LOCAL_TAURI_CLI" build
else
    npm run --prefix frontend tauri -- build
fi

cd "$PROJECT_ROOT"

echo ""
echo "Build completed successfully."
echo "Bundles: $TAURI_DIR/target/release/bundle/"
