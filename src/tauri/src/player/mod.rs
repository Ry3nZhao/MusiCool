//! 高保真播放器。
//!
//! 结构：`decoder`（symphonia 解码）→ `convert`（声道映射 / 必要时重采样）→ `output`（环形缓冲 + 设备流），
//! 由 `engine` 线程串起来。前端通过下面的 Tauri 命令控制，状态通过 `player-status` 事件推送。

mod convert;
mod decoder;
mod engine;
pub mod output;

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

pub use engine::{Command, PlayerStatus, Repeat};
use engine::{Engine, EngineConfig};

use crate::AppState;

/// 持久化在配置文件里的播放设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSettings {
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// 输出设备（cpal DeviceId 字符串）；None 表示跟随系统默认设备
    #[serde(default)]
    pub device_id: Option<String>,
    /// 采样率跟随歌曲（切换设备采样率，避免系统重采样）
    #[serde(default = "default_true")]
    pub match_sample_rate: bool,
    /// 独占模式（目前仅 Windows WASAPI）
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub repeat: Repeat,
    #[serde(default)]
    pub shuffle: bool,
}

fn default_volume() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self { volume: 1.0, device_id: None, match_sample_rate: true, exclusive: false, repeat: Repeat::Off, shuffle: false }
    }
}

pub struct Player {
    tx: Mutex<Sender<Command>>,
    status: Arc<Mutex<PlayerStatus>>,
}

impl Player {
    pub fn spawn(app: tauri::AppHandle, s: &PlayerSettings) -> Self {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(PlayerStatus {
            volume: s.volume,
            repeat: s.repeat,
            shuffle: s.shuffle,
            ..Default::default()
        }));
        let cfg = EngineConfig {
            volume: s.volume,
            device_id: s.device_id.clone(),
            match_rate: s.match_sample_rate,
            exclusive: s.exclusive,
            repeat: s.repeat,
            shuffle: s.shuffle,
        };
        let notify = Box::new(move |st: &PlayerStatus| {
            let _ = app.emit_all("player-status", st);
        });
        let engine = Engine::new(rx, status.clone(), notify, cfg);
        std::thread::Builder::new()
            .name("musicool-player".into())
            .spawn(move || engine.run())
            .expect("启动播放线程失败");
        Self { tx: Mutex::new(tx), status }
    }

    fn send(&self, cmd: Command) -> Result<(), String> {
        self.tx.lock().unwrap().send(cmd).map_err(|_| "播放线程已退出".to_string())
    }
}

#[tauri::command]
pub fn player_status(player: State<'_, Player>) -> PlayerStatus {
    player.status.lock().unwrap().clone()
}

/// 替换播放队列并从第 `index` 首开始播放。
#[tauri::command]
pub fn player_play_queue(paths: Vec<String>, index: usize, player: State<'_, Player>) -> Result<(), String> {
    player.send(Command::Load { paths, index })
}

/// 播放控制：play / pause / toggle / next / prev / stop
#[tauri::command]
pub fn player_control(action: String, player: State<'_, Player>) -> Result<(), String> {
    let cmd = match action.as_str() {
        "play" => Command::Play,
        "pause" => Command::Pause,
        "toggle" => Command::Toggle,
        "next" => Command::Next,
        "prev" => Command::Prev,
        "stop" => Command::Stop,
        other => return Err(format!("未知操作: {other}")),
    };
    player.send(cmd)
}

#[tauri::command]
pub fn player_seek(position: f64, player: State<'_, Player>) -> Result<(), String> {
    player.send(Command::Seek(position))
}

#[tauri::command]
pub async fn player_set_volume(volume: f32, player: State<'_, Player>, state: State<'_, AppState>) -> Result<(), String> {
    let volume = volume.clamp(0.0, 1.0);
    player.send(Command::SetVolume(volume))?;
    state.settings.write().await.player.volume = volume;
    state.save_settings().await
}

#[tauri::command]
pub async fn player_set_mode(
    repeat: Repeat,
    shuffle: bool,
    player: State<'_, Player>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    player.send(Command::SetMode { repeat, shuffle })?;
    {
        let mut s = state.settings.write().await;
        s.player.repeat = repeat;
        s.player.shuffle = shuffle;
    }
    state.save_settings().await
}

#[tauri::command]
pub async fn player_set_output(
    device_id: Option<String>,
    match_sample_rate: bool,
    exclusive: bool,
    player: State<'_, Player>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let exclusive = exclusive && output::EXCLUSIVE_SUPPORTED;
    player.send(Command::SetOutput { device_id: device_id.clone(), match_rate: match_sample_rate, exclusive })?;
    {
        let mut s = state.settings.write().await;
        s.player.device_id = device_id;
        s.player.match_sample_rate = match_sample_rate;
        s.player.exclusive = exclusive;
    }
    state.save_settings().await
}

#[derive(Serialize)]
pub struct OutputOptions {
    devices: Vec<output::DeviceInfo>,
    exclusive_supported: bool,
}

#[tauri::command]
pub async fn player_output_options() -> Result<OutputOptions, String> {
    let devices = tokio::task::spawn_blocking(output::list_devices).await.map_err(|e| e.to_string())?;
    Ok(OutputOptions { devices, exclusive_supported: output::EXCLUSIVE_SUPPORTED })
}

/// 读取同名 `.lrc` 歌词。
#[tauri::command]
pub async fn read_lyrics(path: String) -> Result<String, String> {
    let p = std::path::Path::new(&path);
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrc")) {
        return Err("只能读取 .lrc 歌词文件".into());
    }
    let bytes = tokio::fs::read(p).await.map_err(|e| format!("读取歌词失败: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim_start_matches('\u{feff}').to_string())
}
