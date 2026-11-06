# MusiCool v2.0.0 Release Notes

## 🎉 重大更新

MusiCool v2.0.0 是一个**重大架构升级版本**，从 Python/PyQt6 架构完全迁移到现代化的 Tauri + Rust + React 技术栈，带来更好的性能、更现代的界面和更强的跨平台支持。

## ✨ 主要特性

### 🚀 全新架构
- **后端**: Rust + Tauri，提供原生级别的性能和安全性
- **前端**: React + TypeScript + Vite，现代化的开发体验
- **解码核心**: C++ (通过 FFI 集成)，保持高性能解码能力
- **UI 设计**: Notion 风格的现代化界面

### 🎯 核心功能
- ✅ **跨平台支持** - Windows、macOS、Linux 全平台支持
- ✅ **多线程导入** - 充分利用多核 CPU，大幅提升导入速度
- ✅ **智能文件同步** - rsync 风格的增量同步，避免重复处理
- ✅ **实时进度显示** - 详细的进度信息和状态反馈
- ✅ **项目管理系统** - 支持多个导入项目，每个项目独立的配置和排除规则
- ✅ **可配置设置** - 灵活的音乐库路径和导入参数配置

### 🔧 技术改进
- **性能提升**: Rust 后端带来显著的性能提升
- **内存占用**: 更低的资源占用，更流畅的运行体验
- **启动速度**: 更快的应用启动时间
- **稳定性**: 更强的错误处理和崩溃恢复能力

## 📦 安装说明

### Windows
下载并运行 `MusiCool Setup.exe` (MSI 或 NSIS 安装包)

### macOS / Linux
请参考 README.md 中的构建说明

## 🔄 从 v1.0.0 迁移

- **配置文件**: 配置文件位置保持不变 (`config.json`)，格式兼容
- **数据迁移**: 无需手动迁移，应用会自动识别现有配置
- **Python 版本**: Python/PyQt6 版本已标记为 Legacy，不再积极维护

## 🐛 已知问题

- 某些系统可能需要安装 WiX Toolset 才能生成 MSI 安装包（Windows）
- 首次运行可能需要较长时间进行依赖检查和初始化

## 📝 开发相关

### 构建要求
- Rust (最新稳定版)
- Node.js 18+ 和 npm/yarn
- CMake 3.20+
- C++ 编译器（MSVC/MinGW on Windows, GCC/Clang on Linux/macOS）
- OpenSSL 开发库

### 快速开始
```bash
# Windows 开发
.\scripts\develop\build_and_run.ps1

# Windows 打包
.\scripts\package\Windows\package_windows.ps1 -Version 2.0.0
```

## 🙏 致谢

感谢所有贡献者和用户的支持！

## 📄 许可证

查看 `LICENSE` 文件了解详情。

---

**推荐所有用户升级到 v2.0.0 以获得最佳体验！**

