//! 播放引擎：一个独立线程，持有解码器、输出流和播放队列，通过命令通道接受控制。
//!
//! - 无缝播放（gapless）：一首歌解码完立即在同一个输出流里接着解码下一首；
//!   下一首的输出格式（采样率/声道）不同时，等当前流播完再按新格式重建流（设备随之切换采样率）。
//! - 进度：输出回调累计真实送出的帧数；引擎记录每首歌在流中的起始帧（`Segment`），
//!   界面上显示的“当前歌曲”是正在发声的那首，而不是已经提前开始解码的下一首。

use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::convert::Converter;
use super::decoder::{SourceInfo, TrackDecoder};
use super::output::{negotiate, Output, OutputFormat, OutputRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

pub enum Command {
    /// 替换播放队列并从 `index` 开始播放
    Load { paths: Vec<String>, index: usize },
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Stop,
    Seek(f64),
    SetVolume(f32),
    SetMode { repeat: Repeat, shuffle: bool },
    SetOutput { device_id: Option<String>, match_rate: bool, exclusive: bool },
}

/// 信号链路：源 → 输出。
#[derive(Debug, Clone, Serialize)]
pub struct SignalPath {
    pub source: SourceInfo,
    pub output: OutputFormat,
    pub device: String,
    pub resampled: bool,
    /// 送到设备的样本与源文件逐位一致
    pub bit_perfect: bool,
    /// 不是 bit-perfect 时的原因
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlayerStatus {
    pub state: PlayState,
    /// 当前歌曲在队列（前端传入的列表）中的下标
    pub queue_index: Option<usize>,
    pub queue_len: usize,
    /// 每次替换队列加一，前端据此判断 `queue_index` 对应哪个列表
    pub queue_id: u64,
    pub path: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub signal: Option<SignalPath>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub volume: f32,
    pub device_id: Option<String>,
    pub match_rate: bool,
    pub exclusive: bool,
    pub repeat: Repeat,
    pub shuffle: bool,
}

/// 正在解码、往输出流里写的歌。
struct Feeding {
    index: usize,
    decoder: TrackDecoder,
    conv: Converter,
}

/// 流中某一段属于哪首歌。
struct Segment {
    start_frame: u64,
    index: usize,
    start_secs: f64,
    source: SourceInfo,
    resampled: bool,
}

pub struct Engine {
    rx: Receiver<Command>,
    status: Arc<Mutex<PlayerStatus>>,
    notify: Box<dyn Fn(&PlayerStatus) + Send>,
    cfg: EngineConfig,

    queue: Vec<String>,
    queue_id: u64,
    /// 播放顺序（随机播放时是打乱的下标）
    order: Vec<usize>,
    rng: u64,

    output: Option<Output>,
    feeding: Option<Feeding>,
    /// 下一首需要不同的输出格式：等当前流播完再切换
    waiting: Option<(Feeding, OutputRequest)>,
    pending: Vec<f32>,
    pending_off: usize,
    written_samples: u64,
    segments: VecDeque<Segment>,
    /// 队列已全部解码完，播完即停止
    ended: bool,
    playing: bool,
    /// 没有输出流时（停止、出错）记住的位置
    idle: Option<(usize, f64)>,
    idle_duration: f64,
    error: Option<String>,
    dirty: bool,
    last_emit: Instant,
}

impl Engine {
    pub fn new(
        rx: Receiver<Command>,
        status: Arc<Mutex<PlayerStatus>>,
        notify: Box<dyn Fn(&PlayerStatus) + Send>,
        cfg: EngineConfig,
    ) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);
        Self {
            rx,
            status,
            notify,
            cfg,
            queue: Vec::new(),
            queue_id: 0,
            order: Vec::new(),
            rng: seed | 1,
            output: None,
            feeding: None,
            waiting: None,
            pending: Vec::new(),
            pending_off: 0,
            written_samples: 0,
            segments: VecDeque::new(),
            ended: false,
            playing: false,
            idle: None,
            idle_duration: 0.0,
            error: None,
            dirty: true,
            last_emit: Instant::now(),
        }
    }

    pub fn run(mut self) {
        loop {
            let timeout = Duration::from_millis(if self.playing { 15 } else { 250 });
            match self.rx.recv_timeout(timeout) {
                Ok(cmd) => {
                    self.handle(cmd);
                    while let Ok(cmd) = self.rx.try_recv() {
                        self.handle(cmd);
                    }
                    self.dirty = true;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.check_device();
            self.pump();
            self.advance();
            self.publish();
        }
    }

    // ---------- 命令 ----------

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Load { paths, index } => {
                if paths.is_empty() {
                    return;
                }
                let index = index.min(paths.len() - 1);
                self.queue = paths;
                self.queue_id += 1;
                self.rebuild_order(Some(index));
                self.playing = true;
                self.start_at(index, 0.0);
            }
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Toggle => {
                if self.playing {
                    self.pause()
                } else {
                    self.play()
                }
            }
            Command::Next => {
                if let Some(cur) = self.current_index() {
                    match self.next_index(cur, false) {
                        Some(i) => self.start_at(i, 0.0),
                        None => self.stop(),
                    }
                }
            }
            Command::Prev => {
                if let Some(cur) = self.current_index() {
                    if self.position() > 3.0 {
                        self.start_at(cur, 0.0);
                    } else {
                        let i = self.prev_index(cur);
                        self.start_at(i, 0.0);
                    }
                }
            }
            Command::Stop => self.stop(),
            Command::Seek(secs) => {
                if let Some(cur) = self.current_index() {
                    self.start_at(cur, secs);
                }
            }
            Command::SetVolume(v) => {
                self.cfg.volume = v.clamp(0.0, 1.0);
                if let Some(o) = &self.output {
                    o.shared.set_volume(self.cfg.volume);
                }
            }
            Command::SetMode { repeat, shuffle } => {
                self.cfg.repeat = repeat;
                if shuffle != self.cfg.shuffle {
                    self.cfg.shuffle = shuffle;
                    let cur = self.current_index();
                    self.rebuild_order(cur);
                }
            }
            Command::SetOutput { device_id, match_rate, exclusive } => {
                let changed = device_id != self.cfg.device_id || match_rate != self.cfg.match_rate || exclusive != self.cfg.exclusive;
                self.cfg.device_id = device_id;
                self.cfg.match_rate = match_rate;
                self.cfg.exclusive = exclusive;
                if changed {
                    // 按新设置重建输出，从当前位置继续
                    if let Some(cur) = self.current_index() {
                        let pos = self.position();
                        self.output = None;
                        if self.playing || self.feeding.is_some() {
                            self.start_at(cur, pos);
                        } else {
                            self.idle = Some((cur, pos));
                        }
                    }
                }
            }
        }
    }

    fn play(&mut self) {
        if self.playing {
            return;
        }
        if let Some(out) = &self.output {
            self.playing = true;
            if let Err(e) = out.play() {
                self.error = Some(e);
            }
            return;
        }
        // 没有输出流（已停止或出错后）：从记住的位置重新开始
        let (index, pos) = match self.idle.or_else(|| self.current_index().map(|i| (i, 0.0))) {
            Some(x) => x,
            None if !self.queue.is_empty() => (self.order[0], 0.0),
            None => return,
        };
        self.playing = true;
        self.start_at(index, pos);
    }

    fn pause(&mut self) {
        self.playing = false;
        if let Some(out) = &self.output {
            let _ = out.pause();
        }
    }

    fn stop(&mut self) {
        let cur = self.current_index();
        if let Some(d) = self.segments.front().and_then(|s| s.source.duration) {
            self.idle_duration = d;
        }
        self.playing = false;
        self.reset_stream_state();
        self.output = None;
        self.idle = cur.map(|i| (i, 0.0));
    }

    fn reset_stream_state(&mut self) {
        self.feeding = None;
        self.waiting = None;
        self.pending.clear();
        self.pending_off = 0;
        self.segments.clear();
        self.ended = false;
    }

    // ---------- 队列顺序 ----------

    fn rand(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// 重建播放顺序。随机模式下 `first` 排在最前，其余打乱。
    fn rebuild_order(&mut self, first: Option<usize>) {
        let n = self.queue.len();
        self.order = (0..n).collect();
        if self.cfg.shuffle && n > 1 {
            for i in (1..n).rev() {
                let j = (self.rand() % (i as u64 + 1)) as usize;
                self.order.swap(i, j);
            }
            if let Some(f) = first {
                if let Some(p) = self.order.iter().position(|&x| x == f) {
                    self.order.swap(0, p);
                }
            }
        }
    }

    fn order_pos(&self, index: usize) -> usize {
        self.order.iter().position(|&x| x == index).unwrap_or(0)
    }

    /// 下一首。`auto` 表示自然播完（单曲循环只对自然播完生效）。
    fn next_index(&mut self, cur: usize, auto: bool) -> Option<usize> {
        if self.queue.is_empty() {
            return None;
        }
        if auto && self.cfg.repeat == Repeat::One {
            return Some(cur);
        }
        let p = self.order_pos(cur);
        if p + 1 < self.order.len() {
            return Some(self.order[p + 1]);
        }
        if self.cfg.repeat == Repeat::Off {
            return None;
        }
        if self.cfg.shuffle {
            self.rebuild_order(None);
            if self.order.len() > 1 && self.order[0] == cur {
                let last = self.order.len() - 1;
                self.order.swap(0, last);
            }
        }
        Some(self.order[0])
    }

    fn prev_index(&self, cur: usize) -> usize {
        let p = self.order_pos(cur);
        if p > 0 {
            self.order[p - 1]
        } else if self.cfg.repeat == Repeat::All {
            *self.order.last().unwrap_or(&cur)
        } else {
            cur
        }
    }

    // ---------- 播放 ----------

    fn open_feeding(&self, index: usize) -> Result<(Feeding, OutputRequest), String> {
        let path = self.queue.get(index).ok_or("队列下标越界")?;
        let decoder = TrackDecoder::open(Path::new(path))?;
        let req = negotiate(
            self.cfg.device_id.as_deref(),
            decoder.info.sample_rate,
            decoder.info.channels,
            self.cfg.match_rate,
            self.cfg.exclusive,
        )?;
        let conv = Converter::new(decoder.info.sample_rate, req.sample_rate, req.channels as usize)?;
        Ok((Feeding { index, decoder, conv }, req))
    }

    fn file_label(&self, index: usize) -> String {
        self.queue
            .get(index)
            .and_then(|p| Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// 从 `index` 的 `secs` 秒处开始播放（切歌、跳转都走这里）。
    fn start_at(&mut self, index: usize, secs: f64) {
        self.reset_stream_state();
        self.error = None;
        self.idle = Some((index, secs));
        self.dirty = true;

        let (mut feeding, req) = match self.open_feeding(index) {
            Ok(x) => x,
            Err(e) => {
                self.error = Some(format!("{}：{e}", self.file_label(index)));
                self.output = None;
                self.playing = false;
                return;
            }
        };
        self.idle_duration = feeding.decoder.info.duration.unwrap_or(0.0);
        let start_secs = if secs > 0.0 { feeding.decoder.seek(secs).unwrap_or(0.0) } else { 0.0 };

        let start_frame = match &mut self.output {
            Some(out) if out.matches(&req) => out.flush(),
            _ => {
                // 先释放旧流，再按新格式打开（设备随之切换采样率）
                self.output = None;
                match Output::open(&req, self.cfg.volume) {
                    Ok(o) => {
                        self.output = Some(o);
                        0
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.playing = false;
                        return;
                    }
                }
            }
        };
        let ch = req.channels as u64;
        self.written_samples = start_frame * ch;
        self.segments.push_back(Segment {
            start_frame,
            index,
            start_secs,
            source: feeding.decoder.info.clone(),
            resampled: feeding.conv.resampling(),
        });
        self.feeding = Some(feeding);
        self.idle = None;
        // 先灌满缓冲再开始出声，避免开头欠载
        self.pump();
        // 设备刚切换采样率时 play() 可能阻塞几百毫秒，先把新歌的状态推给界面
        self.dirty = true;
        self.publish();
        if self.playing {
            if let Some(out) = &self.output {
                if let Err(e) = out.play() {
                    self.error = Some(e);
                }
            }
        }
    }

    fn written_frames(&self) -> u64 {
        let ch = self.output.as_ref().map(|o| o.format.channels as u64).unwrap_or(1).max(1);
        self.written_samples / ch
    }

    /// 尽可能多地解码并写入环形缓冲。
    fn pump(&mut self) {
        loop {
            let Some(out) = self.output.as_mut() else { return };
            if self.pending_off < self.pending.len() {
                let (pushed, _) = out.producer.push_partial_slice(&self.pending[self.pending_off..]);
                self.pending_off += pushed.len();
                self.written_samples += pushed.len() as u64;
                if self.pending_off < self.pending.len() {
                    return; // 缓冲已满
                }
            }
            self.pending.clear();
            self.pending_off = 0;

            let Some(f) = self.feeding.as_mut() else { return };
            match f.decoder.next() {
                Ok(Some((samples, ch))) => {
                    f.conv.process(samples, ch, &mut self.pending);
                }
                Ok(None) => {
                    f.conv.finish(&mut self.pending);
                    self.on_track_decoded();
                }
                Err(e) => {
                    eprintln!("{e}");
                    f.conv.finish(&mut self.pending);
                    self.on_track_decoded();
                }
            }
        }
    }

    /// 当前解码的歌结束：准备下一首（能无缝衔接就直接接上）。
    fn on_track_decoded(&mut self) {
        let Some(done) = self.feeding.take() else { return };
        let mut cur = done.index;
        // 连续打不开的文件跳过，最多尝试整个队列一遍
        for _ in 0..self.queue.len().max(1) {
            let Some(next) = self.next_index(cur, true) else { break };
            match self.open_feeding(next) {
                Ok((feeding, req)) => {
                    let gapless = self.output.as_ref().is_some_and(|o| o.matches(&req));
                    if gapless {
                        let start_frame = self.written_frames() + (self.pending.len() - self.pending_off) as u64
                            / self.output.as_ref().map(|o| o.format.channels as u64).unwrap_or(1);
                        self.segments.push_back(Segment {
                            start_frame,
                            index: next,
                            start_secs: 0.0,
                            source: feeding.decoder.info.clone(),
                            resampled: feeding.conv.resampling(),
                        });
                        self.feeding = Some(feeding);
                    } else {
                        self.waiting = Some((feeding, req));
                    }
                    return;
                }
                Err(e) => {
                    self.error = Some(format!("{}：{e}", self.file_label(next)));
                    cur = next;
                }
            }
        }
        self.ended = true;
    }

    /// 根据播放进度推进：切换当前歌曲、换格式重建流、播完停止。
    fn advance(&mut self) {
        let Some(out) = &self.output else { return };
        let played = out.played_frames();
        while self.segments.len() >= 2 && self.segments[1].start_frame <= played {
            self.segments.pop_front();
            self.dirty = true;
        }
        let drained = played >= self.written_frames() && self.pending_off >= self.pending.len();
        if !drained {
            return;
        }
        if let Some((feeding, req)) = self.waiting.take() {
            self.output = None;
            match Output::open(&req, self.cfg.volume) {
                Ok(o) => {
                    self.written_samples = 0;
                    self.segments.clear();
                    self.segments.push_back(Segment {
                        start_frame: 0,
                        index: feeding.index,
                        start_secs: 0.0,
                        source: feeding.decoder.info.clone(),
                        resampled: feeding.conv.resampling(),
                    });
                    self.output = Some(o);
                    self.feeding = Some(feeding);
                    self.pump();
                    if self.playing {
                        if let Some(out) = &self.output {
                            let _ = out.play();
                        }
                    }
                }
                Err(e) => {
                    self.idle = Some((feeding.index, 0.0));
                    self.error = Some(e);
                    self.playing = false;
                }
            }
            self.dirty = true;
        } else if self.ended {
            let last = self.current_index();
            self.stop();
            self.idle = last.map(|i| (i, 0.0));
            self.dirty = true;
        }
    }

    fn check_device(&mut self) {
        let Some(out) = &self.output else { return };
        if !out.shared.failed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let msg = out.shared.error.lock().unwrap().take();
        let recoverable = out.shared.recoverable.load(std::sync::atomic::Ordering::Relaxed);
        let cur = self.current_index();
        let pos = self.position();
        self.reset_stream_state();
        self.output = None;
        self.dirty = true;
        if recoverable {
            // 流失效（例如其他程序改了设备采样率）：按当前设置重建，从原位置继续
            if let Some(i) = cur {
                self.start_at(i, pos);
                return;
            }
        }
        // 设备消失：停下并提示，不自动换到别的设备上外放
        self.playing = false;
        self.idle = cur.map(|i| (i, pos));
        self.error = msg.or(Some("音频设备不可用".into()));
    }

    // ---------- 状态 ----------

    fn current_index(&self) -> Option<usize> {
        self.segments.front().map(|s| s.index).or(self.idle.map(|(i, _)| i))
    }

    fn position(&self) -> f64 {
        match (self.segments.front(), &self.output) {
            (Some(seg), Some(out)) => {
                let played = out.played_frames().saturating_sub(seg.start_frame);
                seg.start_secs + played as f64 / out.format.sample_rate as f64
            }
            _ => self.idle.map(|(_, p)| p).unwrap_or(0.0),
        }
    }

    fn signal(&self) -> Option<SignalPath> {
        let seg = self.segments.front()?;
        let out = self.output.as_ref()?;
        let src = &seg.source;
        let mut notes = Vec::new();
        if !src.lossless {
            notes.push(format!("有损音源（{}）", src.codec));
        }
        if seg.resampled {
            notes.push(format!(
                "设备不支持 {}，已重采样到 {}",
                khz(src.sample_rate),
                khz(out.format.sample_rate)
            ));
        }
        if src.channels != out.format.channels {
            notes.push(format!("声道 {} → {}", src.channels, out.format.channels));
        }
        if src.bits_per_sample.is_some_and(|b| b > out.format.lossless_bits) {
            notes.push(format!("{} 无法无损承载 {} bit", out.format.sample_format, src.bits_per_sample.unwrap_or(0)));
        }
        if self.cfg.volume < 1.0 {
            notes.push("软件音量低于 100%".into());
        }
        Some(SignalPath {
            source: src.clone(),
            output: out.format.clone(),
            device: out.device_name.clone(),
            resampled: seg.resampled,
            bit_perfect: notes.is_empty(),
            notes,
        })
    }

    fn publish(&mut self) {
        let periodic = self.playing && self.last_emit.elapsed() >= Duration::from_millis(250);
        if !self.dirty && !periodic {
            return;
        }
        self.dirty = false;
        self.last_emit = Instant::now();
        let index = self.current_index();
        let duration = self
            .segments
            .front()
            .and_then(|s| s.source.duration)
            .unwrap_or(self.idle_duration);
        let state = if self.output.is_none() && self.feeding.is_none() {
            PlayState::Stopped
        } else if self.playing {
            PlayState::Playing
        } else {
            PlayState::Paused
        };
        let status = PlayerStatus {
            state,
            queue_index: index,
            queue_len: self.queue.len(),
            queue_id: self.queue_id,
            path: index.and_then(|i| self.queue.get(i).cloned()),
            position: self.position().min(if duration > 0.0 { duration } else { f64::MAX }),
            duration,
            volume: self.cfg.volume,
            repeat: self.cfg.repeat,
            shuffle: self.cfg.shuffle,
            signal: self.signal(),
            error: self.error.clone(),
        };
        (self.notify)(&status);
        *self.status.lock().unwrap() = status;
    }
}

fn khz(rate: u32) -> String {
    let k = rate as f64 / 1000.0;
    if k.fract() == 0.0 {
        format!("{k:.0} kHz")
    } else {
        format!("{k} kHz")
    }
}

#[cfg(test)]
mod device_tests {
    //! 需要真实音频文件与声卡，默认忽略：
    //! `PLAYER_TEST_DIR=~/Music/xxx cargo test --release device_ -- --ignored --nocapture --test-threads=1`
    use super::*;
    use std::sync::mpsc::channel;

    fn files() -> Vec<String> {
        let dir = std::env::var("PLAYER_TEST_DIR").expect("PLAYER_TEST_DIR");
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path().to_string_lossy().to_string())
            .filter(|p| crate::library::is_audio(Path::new(p)))
            .collect();
        v.sort();
        v
    }

    fn info(p: &str) -> SourceInfo {
        TrackDecoder::open(Path::new(p)).unwrap().info
    }

    /// 解码结果与 macOS 自带解码器（afconvert）逐样本一致。
    #[test]
    #[ignore]
    fn device_decode_bit_exact() {
        let tmp = std::env::temp_dir().join("musicool-afconvert.wav");
        let mut checked = 0;
        for p in files().iter().filter(|p| p.ends_with(".flac")).take(4) {
            let src = info(p);
            let bits = src.bits_per_sample.unwrap();
            let fmt = if bits > 16 { "LEI24" } else { "LEI16" };
            let ok = std::process::Command::new("afconvert")
                .args(["-f", "WAVE", "-d", fmt, p, tmp.to_str().unwrap()])
                .status()
                .unwrap()
                .success();
            assert!(ok, "afconvert 失败");
            let mut a = TrackDecoder::open(Path::new(p)).unwrap();
            let mut b = TrackDecoder::open(&tmp).unwrap();
            let (mut va, mut vb) = (Vec::new(), Vec::new());
            while let Some((s, _)) = a.next().unwrap() {
                va.extend_from_slice(s);
            }
            while let Some((s, _)) = b.next().unwrap() {
                vb.extend_from_slice(s);
            }
            assert_eq!(va.len(), vb.len(), "{p} 帧数不一致");
            let diff = va.iter().zip(&vb).filter(|(x, y)| x != y).count();
            println!("{p}: {} {}Hz/{}bit, {} 样本, 不一致 {}", src.codec, src.sample_rate, bits, va.len(), diff);
            assert_eq!(diff, 0);
            checked += 1;
        }
        assert!(checked > 0);
    }

    struct Rig {
        tx: std::sync::mpsc::Sender<Command>,
        status: Arc<Mutex<PlayerStatus>>,
    }
    impl Rig {
        fn new() -> Self {
            let (tx, rx) = channel();
            let status = Arc::new(Mutex::new(PlayerStatus::default()));
            let cfg = EngineConfig {
                volume: 0.0, // 测试时不出声
                device_id: None,
                match_rate: true,
                exclusive: false,
                repeat: Repeat::Off,
                shuffle: false,
            };
            let engine = Engine::new(rx, status.clone(), Box::new(|_| {}), cfg);
            std::thread::spawn(move || engine.run());
            Self { tx, status }
        }
        fn send(&self, c: Command) {
            self.tx.send(c).unwrap();
        }
        fn wait(&self, secs: f64) -> PlayerStatus {
            std::thread::sleep(Duration::from_secs_f64(secs));
            let s = self.status.lock().unwrap().clone();
            let sig = s.signal.as_ref().map(|g| {
                format!(
                    "{} {}Hz/{:?}bit → {} {}Hz {}ch [{}] resampled={} bit_perfect={} {:?}",
                    g.source.codec, g.source.sample_rate, g.source.bits_per_sample, g.output.sample_format,
                    g.output.sample_rate, g.output.channels, g.device, g.resampled, g.bit_perfect, g.notes
                )
            });
            println!("  {:?} idx={:?} pos={:.2}/{:.2} err={:?}\n    {}", s.state, s.queue_index, s.position, s.duration, s.error, sig.unwrap_or_default());
            s
        }
    }

    #[test]
    #[ignore]
    fn device_playback_flow() {
        let all = files();
        let hi: Vec<_> = all.iter().filter(|p| info(p).sample_rate > 48000).take(2).cloned().collect();
        let lo: Vec<_> = all.iter().filter(|p| info(p).sample_rate == 44100).take(1).cloned().collect();
        assert!(hi.len() == 2 && lo.len() == 1, "需要两首高采样率和一首 44.1k 的歌");
        let queue = vec![hi[0].clone(), hi[1].clone(), lo[0].clone()];
        let rig = Rig::new();

        println!("1. 开始播放第一首");
        rig.send(Command::Load { paths: queue.clone(), index: 0 });
        let s = rig.wait(1.5);
        assert_eq!(s.state, PlayState::Playing);
        assert_eq!(s.queue_index, Some(0));
        assert!(s.position > 0.3 && s.position < 2.0, "pos={}", s.position); // 切换采样率时硬件约需 0.6 秒稳定

        println!("2. 跳到第一首结尾，应无缝进入第二首（同格式，不重建流）");
        let d0 = s.duration;
        rig.send(Command::Seek(d0 - 1.5));
        let s = rig.wait(0.5);
        assert!((s.position - (d0 - 1.0)).abs() < 0.4, "pos={}", s.position);
        let s = rig.wait(2.0);
        assert_eq!(s.queue_index, Some(1));
        assert!(s.position > 0.2 && s.position < 1.5, "pos={}", s.position);

        println!("3. 暂停后进度不动");
        rig.send(Command::Pause);
        let a = rig.wait(0.3);
        let b = rig.wait(1.0);
        assert_eq!(b.state, PlayState::Paused);
        assert!((a.position - b.position).abs() < 0.05);

        println!("4. 继续播放，跳到第二首结尾，进入 44.1k 的第三首（需要按新采样率重建流）");
        rig.send(Command::Play);
        let d1 = b.duration;
        rig.send(Command::Seek(d1 - 1.0));
        let s = rig.wait(2.5);
        assert_eq!(s.queue_index, Some(2));
        assert_eq!(s.state, PlayState::Playing);
        assert!(s.position > 0.3 && s.position < 2.0, "pos={}", s.position);

        println!("5. 上一首（播放超过 3 秒前）回到第二首");
        rig.send(Command::Prev);
        let s = rig.wait(1.0);
        assert_eq!(s.queue_index, Some(1));

        println!("6. 下一首 ×2 后到队尾停止");
        rig.send(Command::Next);
        rig.wait(0.5);
        rig.send(Command::Next);
        let s = rig.wait(0.5);
        assert_eq!(s.state, PlayState::Stopped);

        println!("7. 播完自动停止");
        rig.send(Command::Load { paths: vec![lo[0].clone()], index: 0 });
        let s = rig.wait(0.5);
        rig.send(Command::Seek(s.duration - 0.8));
        let s = rig.wait(2.0);
        assert_eq!(s.state, PlayState::Stopped);
        assert_eq!(s.queue_index, Some(0));
    }
}
