//! 音频输出：选择设备格式、建立输出流、实时回调。
//!
//! 数据流：引擎线程 → 无锁环形缓冲（f32 交错样本）→ 设备回调 [`render`] → 声卡。
//!
//! 高保真相关：
//! - 采样率跟随歌曲：设备支持源采样率时直接用源采样率建流。macOS 上 cpal 会同时把设备的
//!   物理格式 / 标称采样率切到该值，系统混音器不再做重采样。
//! - 设备不支持源采样率时才重采样（引擎里用 rubato FFT 重采样），优先选源采样率的整数倍。
//! - 音量 100% 时回调里不做任何运算，样本原样送出（bit-perfect）。

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

#[cfg(windows)]
pub mod wasapi_exclusive;

/// 引擎与实时回调之间共享的状态。
pub struct OutputShared {
    /// 已经送到设备的帧数（不含欠载时补的静音），用来算播放进度
    pub played_frames: AtomicU64,
    volume_bits: AtomicU32,
    /// 设备出错（拔出等）后置位，引擎据此重建输出
    pub failed: AtomicBool,
    /// 出错后可以按原设置重建流继续播放（流失效，而不是设备消失）
    pub recoverable: AtomicBool,
    pub error: Mutex<Option<String>>,
    /// 欠载次数（仅用于诊断）
    pub xruns: AtomicU64,
}

impl OutputShared {
    pub fn new(volume: f32) -> Self {
        Self {
            played_frames: AtomicU64::new(0),
            volume_bits: AtomicU32::new(volume.to_bits()),
            failed: AtomicBool::new(false),
            recoverable: AtomicBool::new(false),
            error: Mutex::new(None),
            xruns: AtomicU64::new(0),
        }
    }
    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }
    pub fn fail(&self, msg: String) {
        *self.error.lock().unwrap() = Some(msg);
        self.failed.store(true, Ordering::Release);
    }

    /// 处理 cpal 的流错误：欠载、默认设备切换等不影响继续播放的只记录；其余让引擎处理。
    fn on_cpal_error(&self, e: cpal::Error) {
        use cpal::ErrorKind as K;
        match e.kind() {
            K::Xrun => {
                self.xruns.fetch_add(1, Ordering::Relaxed);
            }
            K::DeviceChanged | K::RealtimeDenied => {}
            K::StreamInvalidated => {
                self.recoverable.store(true, Ordering::Relaxed);
                self.fail(format!("音频流失效: {e}"));
            }
            _ => self.fail(format!("音频设备错误: {e}")),
        }
    }
}

/// 环形缓冲的读端。实时回调用 `try_lock` 取用；引擎需要清空缓冲（跳转、切歌）时短暂持锁。
pub type SampleConsumer = Arc<Mutex<rtrb::Consumer<f32>>>;

/// 输出流实际使用的格式，用于展示信号链路。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OutputFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// 送给设备的样本格式，如 "F32" / "I24" / "I16"
    pub sample_format: String,
    /// 该格式能无损承载的最大源位深（F32 = 24，I32 = 32，I16 = 16）
    pub lossless_bits: u32,
    /// 是否独占设备（绕过系统混音器）
    pub exclusive: bool,
}

/// 实时回调的公共实现：从环形缓冲取样本、乘音量、转成设备样本格式。所有后端共用。
///
/// `vol_state` 是回调私有的“当前音量”，用于在一个缓冲内平滑过渡到目标音量，避免拉动音量时的爆音。
/// 缓冲不足时补静音（欠载），只把真实送出的帧计入 `played_frames`。
pub fn render<T>(data: &mut [T], channels: usize, consumer: &SampleConsumer, shared: &OutputShared, vol_state: &mut f32)
where
    T: SizedSample + FromSample<f32>,
{
    let target = shared.volume();
    let mut written = 0;
    if let Ok(mut cons) = consumer.try_lock() {
        let n = data.len().min(cons.slots()) / channels * channels;
        if n > 0 {
            if let Ok(chunk) = cons.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                let src = a.iter().chain(b.iter());
                if *vol_state == target && target == 1.0 {
                    // bit-perfect 路径：不做任何运算
                    for (o, s) in data.iter_mut().zip(src) {
                        *o = T::from_sample(*s);
                    }
                } else {
                    let frames = n / channels;
                    let step = (target - *vol_state) / frames as f32;
                    let mut v = *vol_state;
                    for (i, (o, s)) in data.iter_mut().zip(src).enumerate() {
                        if i % channels == 0 {
                            v += step;
                        }
                        *o = T::from_sample(*s * v);
                    }
                    *vol_state = target;
                }
                chunk.commit_all();
                written = n;
            }
        }
    }
    for o in &mut data[written..] {
        *o = T::EQUILIBRIUM;
    }
    if written > 0 {
        shared.played_frames.fetch_add((written / channels) as u64, Ordering::Relaxed);
    }
}

/// 一个设备的描述，给设置界面用。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfo {
    /// cpal 的 `DeviceId` 字符串（`<host>:<id>`），持久化到设置里
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// 设备支持的常见采样率
    pub sample_rates: Vec<u32>,
}

const COMMON_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000,
];

pub fn list_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default_id = host.default_output_device().and_then(|d| d.id().ok()).map(|id| id.to_string());
    let Ok(devices) = host.output_devices() else { return Vec::new() };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let name = d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| d.to_string());
            let ranges: Vec<_> = d.supported_output_configs().ok()?.collect();
            let sample_rates = COMMON_RATES
                .iter()
                .copied()
                .filter(|r| ranges.iter().any(|c| c.min_sample_rate() <= *r && *r <= c.max_sample_rate()))
                .collect();
            Some(DeviceInfo { is_default: default_id.as_deref() == Some(id.as_str()), id, name, sample_rates })
        })
        .collect()
}

fn find_device(host: &cpal::Host, device_id: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(id) = device_id {
        if let Some(d) = cpal::DeviceId::from_str(id).ok().and_then(|id| host.device_by_id(&id)) {
            return Ok(d);
        }
        // 设置里的设备已拔出：退回系统默认设备
    }
    host.default_output_device().ok_or_else(|| "没有可用的音频输出设备".to_string())
}

/// 为一首歌决定输出格式。
pub struct OutputRequest {
    pub device_id: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub exclusive: bool,
}

fn format_rank(f: SampleFormat) -> Option<u8> {
    // f32 能无损承载 24 bit，且是 CoreAudio / WASAPI 共享模式的原生格式，最优先
    Some(match f {
        SampleFormat::F32 => 0,
        SampleFormat::I32 => 1,
        SampleFormat::I24 => 2,
        SampleFormat::F64 => 3,
        SampleFormat::I16 => 4,
        SampleFormat::U16 => 5,
        _ => return None,
    })
}

fn lossless_bits(f: SampleFormat) -> u32 {
    match f {
        SampleFormat::F64 | SampleFormat::I32 => 32,
        SampleFormat::F32 | SampleFormat::I24 => 24,
        _ => 16,
    }
}

/// 选择输出采样率与声道数。`match_rate` 为 false 时始终用设备当前的默认采样率（由引擎重采样）。
pub fn negotiate(device_id: Option<&str>, src_rate: u32, src_channels: u16, match_rate: bool, exclusive: bool) -> Result<OutputRequest, String> {
    let host = cpal::default_host();
    let device = find_device(&host, device_id)?;
    let ranges: Vec<_> = device.supported_output_configs().map_err(|e| format!("读取设备能力失败: {e}"))?.collect();
    let default = device.default_output_config().map_err(|e| format!("读取设备默认格式失败: {e}"))?;
    let has_ch = |c: u16| ranges.iter().any(|r| r.channels() == c);
    let want = if src_channels == 1 { 2 } else { src_channels };
    let channels = if has_ch(want) {
        want
    } else if has_ch(2) {
        2
    } else {
        default.channels()
    };
    // 独占模式下 cpal（按共享模式报告、带自动转换）给出的能力不可信，改用独占探测结果；探测失败时退回 cpal 的结果
    #[cfg(windows)]
    let exclusive_rates = if exclusive { wasapi_exclusive::exclusive_sample_rates(device_id, channels, COMMON_RATES) } else { Vec::new() };
    #[cfg(not(windows))]
    let exclusive_rates: Vec<u32> = Vec::new();
    let supports = |rate: u32| {
        if !exclusive_rates.is_empty() {
            return exclusive_rates.contains(&rate);
        }
        ranges.iter().any(|r| r.channels() == channels && r.min_sample_rate() <= rate && rate <= r.max_sample_rate())
    };

    let sample_rate = if !match_rate {
        default.sample_rate()
    } else if supports(src_rate) {
        src_rate
    } else {
        let ok: Vec<u32> = COMMON_RATES.iter().copied().filter(|r| supports(*r)).collect();
        ok.iter()
            .copied()
            .find(|r| *r > src_rate && r % src_rate == 0)
            .or_else(|| ok.iter().copied().find(|r| *r >= src_rate))
            .or_else(|| ok.last().copied())
            .unwrap_or(default.sample_rate())
    };
    Ok(OutputRequest { device_id: device_id.map(String::from), sample_rate, channels, exclusive })
}

enum Backend {
    Cpal(cpal::Stream),
    #[cfg(windows)]
    Exclusive(wasapi_exclusive::ExclusiveStream),
}

pub struct Output {
    backend: Backend,
    pub format: OutputFormat,
    pub device_name: String,
    pub producer: rtrb::Producer<f32>,
    pub consumer: SampleConsumer,
    pub shared: Arc<OutputShared>,
    /// 请求时是否要求独占（独占失败退回共享模式时 `format.exclusive` 为 false，但这里仍为 true）
    requested_exclusive: bool,
}

impl Output {
    /// 按请求建立输出流（建立后处于暂停状态，调用 [`Output::play`] 开始）。环形缓冲约 1.5 秒。
    pub fn open(req: &OutputRequest, volume: f32) -> Result<Self, String> {
        let capacity = (req.sample_rate as usize * req.channels as usize * 3 / 2).max(8192);
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let consumer: SampleConsumer = Arc::new(Mutex::new(consumer));
        let shared = Arc::new(OutputShared::new(volume));

        #[cfg(windows)]
        if req.exclusive {
            match wasapi_exclusive::ExclusiveStream::open(
                req.device_id.as_deref(),
                req.sample_rate,
                req.channels,
                consumer.clone(),
                shared.clone(),
            ) {
                Ok((stream, format, device_name)) => {
                    return Ok(Self { backend: Backend::Exclusive(stream), format, device_name, producer, consumer, shared, requested_exclusive: true });
                }
                // 独占失败（设备被占用、格式不支持）时退回共享模式
                Err(e) => eprintln!("独占模式打开失败，改用共享模式: {e}"),
            }
        }

        let host = cpal::default_host();
        let device = find_device(&host, req.device_id.as_deref())?;
        let device_name = device.description().map(|x| x.name().to_string()).unwrap_or_else(|_| device.to_string());
        let ranges: Vec<_> = device.supported_output_configs().map_err(|e| format!("读取设备能力失败: {e}"))?.collect();
        let sample_format = ranges
            .iter()
            .filter(|r| r.channels() == req.channels && r.min_sample_rate() <= req.sample_rate && req.sample_rate <= r.max_sample_rate())
            .filter_map(|r| format_rank(r.sample_format()).map(|k| (k, r.sample_format())))
            .min_by_key(|(k, _)| *k)
            .map(|(_, f)| f)
            .unwrap_or(SampleFormat::F32);
        let config = cpal::StreamConfig {
            channels: req.channels,
            sample_rate: req.sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        macro_rules! build {
            ($t:ty) => {{
                let consumer = consumer.clone();
                let shared_cb = shared.clone();
                let shared_err = shared.clone();
                let channels = req.channels as usize;
                let mut vol = volume;
                device.build_output_stream::<$t, _, _>(
                    config,
                    move |data: &mut [$t], _| render(data, channels, &consumer, &shared_cb, &mut vol),
                    move |e| shared_err.on_cpal_error(e),
                    None,
                )
            }};
        }
        let stream = match sample_format {
            SampleFormat::F64 => build!(f64),
            SampleFormat::I32 => build!(i32),
            SampleFormat::I24 => build!(cpal::I24),
            SampleFormat::I16 => build!(i16),
            SampleFormat::U16 => build!(u16),
            _ => build!(f32),
        }
        .map_err(|e| format!("打开音频输出失败（{} Hz / {} 声道）: {e}", req.sample_rate, req.channels))?;
        stream.pause().ok();

        Ok(Self {
            backend: Backend::Cpal(stream),
            format: OutputFormat {
                sample_rate: req.sample_rate,
                channels: req.channels,
                sample_format: format!("{sample_format:?}").to_uppercase(),
                lossless_bits: lossless_bits(sample_format),
                exclusive: false,
            },
            device_name,
            producer,
            consumer,
            shared,
            requested_exclusive: req.exclusive,
        })
    }

    pub fn play(&self) -> Result<(), String> {
        match &self.backend {
            Backend::Cpal(s) => s.play().map_err(|e| format!("开始播放失败: {e}")),
            #[cfg(windows)]
            Backend::Exclusive(s) => s.play(),
        }
    }

    pub fn pause(&self) -> Result<(), String> {
        match &self.backend {
            Backend::Cpal(s) => s.pause().map_err(|e| format!("暂停失败: {e}")),
            #[cfg(windows)]
            Backend::Exclusive(s) => s.pause(),
        }
    }

    /// 清空环形缓冲中尚未播放的样本（跳转/切歌时用）。返回清空时刻已播放的帧数。
    pub fn flush(&mut self) -> u64 {
        let mut cons = self.consumer.lock().unwrap();
        let n = cons.slots();
        if let Ok(chunk) = cons.read_chunk(n) {
            chunk.commit_all();
        }
        self.shared.played_frames.load(Ordering::Relaxed)
    }

    pub fn played_frames(&self) -> u64 {
        self.shared.played_frames.load(Ordering::Relaxed)
    }

    /// 请求的格式能否沿用当前流：采样率、声道一致即可无缝衔接（gapless）。
    pub fn matches(&self, req: &OutputRequest) -> bool {
        self.format.sample_rate == req.sample_rate
            && self.format.channels == req.channels
            && req.exclusive == self.requested_exclusive
    }
}

/// 当前平台是否支持独占模式（目前只有 Windows WASAPI）。
pub const EXCLUSIVE_SUPPORTED: bool = cfg!(windows);
