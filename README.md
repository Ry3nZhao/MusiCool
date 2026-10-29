# Music_Cool

一个用于解码ncm格式文件的跨平台桌面应用程序。

## 项目结构

### Tauri 版本（推荐）

**位置**: `src/tauri/`

这是项目的**主要推荐版本**，使用以下技术栈构建：

- **后端**: Rust + Tauri
- **前端**: React + TypeScript + Vite
- **解码核心**: 纯 Rust（`src/tauri/src/decoder/`）
- **UI 设计**: Notion 风格

#### 特性

- ✅ 跨平台支持（Windows、macOS、Linux）
- ✅ 现代化的 Notion 风格界面
- ✅ 多线程导入支持
- ✅ rsync 风格的智能文件同步
- ✅ 实时进度显示
- ✅ 可配置的音乐库路径和导入设置

#### 构建与运行

**前置要求**:
- Rust (最新稳定版)
- Node.js 18+ 和 npm/yarn

**构建步骤**:

**构建 Tauri 应用**（解码器随 Rust 后端一起编译，无需额外步骤）:
   ```bash
   cd src/tauri
   # 安装前端依赖
   cd frontend
   npm install
   cd ..
   
   # 开发模式运行
   cargo tauri dev
   
   # 构建发布版本
   cargo tauri build
   ```

#### 快速脚本（Windows）

- 开发（可选 `--clean`/`-Clean` 清理后启动 Tauri dev）:
  - `.\scripts\develop\build_and_run.ps1`
- 开发（直接启动 Tauri dev）:
  - `.\scripts\develop\run_only.ps1`
- 仅构建发布版本（`tauri build`，不运行）:
  - `.\scripts\develop\build_only.ps1`
- Windows 打包（构建 Tauri 应用并收集安装包到 `release/<version>/`）:
  - `.\scripts\package\Windows\package_windows.ps1 -Version 1.0.0`

**配置文件位置**:
- 项目根目录: `config.json`

#### 快速脚本（Linux/macOS）

- 开发（可选 `--clean`/`-Clean` 清理后启动 Tauri dev）:
  - `./scripts/develop/build_and_run.sh`
- 开发（直接启动 Tauri dev）:
  - `./scripts/develop/run_only.sh`
- 仅构建发布版本（`tauri build`，不运行）:
  - `./scripts/develop/build_only.sh`
- Linux 打包（构建 Tauri 应用并收集安装包到 `release/<version>/`）:
  - `./scripts/package/Linux/package_linux.sh --version 2.0.0`
- macOS 打包（构建 Tauri 应用并收集安装包到 `release/<version>/`）:
  - `./scripts/package/MacOS/package_macos.sh --version 2.0.0`

## 核心解码器

**位置**: `src/tauri/src/decoder/`

纯 Rust 实现的 NCM 解码核心（`ncm.rs`），`wrapper.rs` 负责在 Tauri 后端中异步调用并上报进度。

解码器支持：
- NCM 文件解码为 FLAC/MP3（按音频文件头识别实际格式）
- 实时进度回调
- 写入标题/歌手/专辑/封面标签（MP3 与 FLAC）
- 先写临时文件再重命名，失败不留半截文件

用真实文件做回归测试：
```bash
cd src/tauri
NCM_DIR=~/Music/网易云音乐 OUT_DIR_TEST=/tmp/ncm_out cargo test --release decode_dir -- --ignored
```

## 开发指南

### 添加新功能

1. **后端逻辑**: 在 `src/tauri/src/` 中添加 Rust 模块
2. **Tauri 命令**: 在 `src/tauri/src/main.rs` 中注册新的 `#[tauri::command]`
3. **前端界面**: 在 `src/tauri/frontend/src/pages/` 中添加或修改 React 组件

### 调试

- **Rust 后端**: 使用 `cargo tauri dev` 运行，日志会输出到控制台
- **前端**: 使用浏览器开发者工具（在 Tauri 窗口中按 F12）

## 许可证

查看 `LICENSE` 文件了解详情。

## 贡献

欢迎提交 Issue 和 Pull Request！

---

**推荐使用 Tauri 版本以获得最佳体验和最新功能。**



