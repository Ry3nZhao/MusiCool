#!/usr/bin/env bash
# Music_Cool - Run Tauri Dev
# 启动 Tauri 开发环境（tauri dev 会自动编译 Rust 后端，含解码器）
# 示例: ./scripts/develop/build_and_run.sh [--clean]

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

echo "=== Music_Cool - Build & Run Tauri Dev ==="
echo "Project Directory: $PROJECT_ROOT"
echo ""

if [[ "$CLEAN" == true ]]; then
    echo "Cleaning Rust build directory..."
    (cd "$PROJECT_ROOT/src/tauri" && cargo clean)
    echo ""
fi

"$SCRIPT_DIR/run_only.sh"
