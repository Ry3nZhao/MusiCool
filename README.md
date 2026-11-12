<p align="center">
  <img src="assets/brand/logo-horizontal.svg" alt="MusiCool" width="360">
</p>

<p align="center">
  本地音乐库管理 + 高保真播放的桌面应用，支持 macOS、Windows、Linux。
</p>

<p align="center">
  <a href="https://github.com/Ry3nZhao/MusiCool/releases">下载</a> ·
  <a href="#功能">功能</a> ·
  <a href="#高保真播放">高保真播放</a> ·
  <a href="#从源码构建">从源码构建</a>
</p>

---

MusiCool 把散落在各处的音乐整理成一个干净的本地音乐库，并以尽可能原汁原味的方式播放出来：采样率跟随歌曲切换，100% 音量下送到声卡的数据与源文件逐位一致（bit-perfect），并在界面上清楚地告诉你信号经过了什么处理。

## 功能

### 音乐库

- **导入与同步**：把多个来源目录（下载目录、旧硬盘、移动设备……）整理进同一个音乐库，增量同步只处理新增和变化的文件，支持排除规则和多线程
- **自动索引**：标题、歌手、专辑、时长、格式、封面全部写入本地 SQLite 索引，几千首歌也能即时搜索和排序
- **封面与歌词**：读取内嵌封面并缓存缩略图；自动关联同目录同名的 `.lrc` 歌词
- **数据可重建**：索引只是缓存，删掉音乐库下的 `.musicool/` 目录，刷新即可从文件重新生成

### 播放

- **格式**：FLAC、ALAC、WAV、AAC/M4A、MP3、Ogg Vorbis
- **无缝播放（gapless）**：同一张专辑的连续曲目之间没有空隙
- **播放队列**：顺序 / 列表循环 / 单曲循环、随机播放，双击音乐库任意歌曲即从这里开始播放当前列表
- **正在播放页**：大封面、滚动同步歌词（点击歌词跳转）、播放队列、完整信号链路
- **快捷键**：空格播放/暂停，⌘/Ctrl + ← → 上一首/下一首

## 高保真播放

| | 做法 |
|---|---|
| 解码 | 纯 Rust 解码，24-bit 无损内容无精度损失（已与系统解码器逐样本比对一致） |
| 采样率 | 每首歌自动把输出设备切到歌曲的原始采样率；设备不支持时选最接近的整数倍率，再做高质量重采样 |
| 音量 | 100% 音量时完全不做数字处理；调低音量才会做带平滑的数字衰减 |
| 独占模式 | Windows 上可使用 WASAPI 独占，绕过系统混音器 |
| 透明度 | 播放条上的徽章实时显示「源格式 → 输出格式」，是否 bit-perfect、为什么不是，一目了然 |

在「设置 → 音频输出」里可以选择输出设备（如外置 USB DAC）、查看设备支持的采样率、开关采样率跟随和独占模式。

> 想要真正的 bit-perfect，请把系统音量和 MusiCool 音量都保持在 100%，用前级或功放调节音量。

### 平台支持

| 平台 | 状态 |
|---|---|
| macOS | 完整支持：采样率自动切换、bit-perfect 已在真机验证 |
| Windows | 共享模式可用；WASAPI 独占模式已实现，仍在测试中 |
| Linux | 可用（经系统音频服务输出），ALSA 直通输出在规划中 |

## 下载

前往 [Releases](https://github.com/Ry3nZhao/MusiCool/releases) 下载：

- **macOS**：`.dmg`（Intel 与 Apple Silicon 通用）。安装包暂未签名，首次打开请右键选择「打开」
- **Windows**：`.msi` 或 `-setup.exe`
- **Linux**：`.deb` 或 `.AppImage`

每次合入 `master` 都会自动构建并发布一个预览版（pre-release），想尝鲜可以下载最新的预览版。

## 从源码构建

需要 Rust（stable）和 Node.js 22+。Linux 另需 `libwebkit2gtk-4.1-dev`、`libgtk-3-dev`、`libasound2-dev` 等依赖（见 [CI 配置](.github/workflows/ci.yml)）。

```bash
cd src/tauri
npm install --prefix frontend

# 开发模式
./frontend/node_modules/.bin/tauri dev

# 构建安装包，产物在 src/tauri/target/release/bundle/
./frontend/node_modules/.bin/tauri build
```

也可以使用 `scripts/develop/` 下的脚本（`.sh` 用于 macOS/Linux，`.ps1` 用于 Windows）：

| 脚本 | 作用 |
|---|---|
| `build_and_run` | 安装依赖并以开发模式启动（`--clean` 先清理） |
| `run_only` | 直接以开发模式启动 |
| `build_only` | 构建发布安装包 |

### 测试

```bash
cd src/tauri
cargo test

# 需要真实音频设备和音乐文件的测试（解码逐位比对、完整播放流程）
PLAYER_TEST_DIR=~/Music/测试 cargo test --release device_ -- --ignored --nocapture --test-threads=1
```

## 技术栈

- **后端**：Rust + Tauri
  - `player/`：解码（symphonia）、重采样（rubato）、音频输出（cpal / WASAPI 独占）、播放引擎
  - `library/`：SQLite 音乐库索引
  - `importer/`：导入与增量同步
- **前端**：React + TypeScript + Vite

## 许可证

[AGPL-3.0](LICENSE)

欢迎提交 Issue 和 Pull Request。
