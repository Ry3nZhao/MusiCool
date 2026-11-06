//! Windows WASAPI 独占模式输出（exclusive mode）。
//!
//! 共享模式下 Windows 音频引擎会把所有流重采样 / 转换到“声音设置”里的系统混音格式；
//! 独占模式绕过混音器，样本原样交给驱动，采样率与位深由我们决定，配合音量 100% 即 bit-perfect。
//!
//! 结构：
//! - 所有 COM 对象都在专用渲染线程里创建、使用、释放（windows-rs 的接口类型不是 `Send`）。
//!   [`ExclusiveStream::open`] 等渲染线程完成设备打开与格式协商后才返回。
//! - 事件驱动（`AUDCLNT_STREAMFLAGS_EVENTCALLBACK`）：设备每消耗完一个周期就触发事件，
//!   渲染线程每次把整块设备缓冲（独占事件模式下 = 一个设备周期）填满。
//! - 渲染线程以 MMCSS "Pro Audio" 优先级运行。
//! - 播放 / 暂停经命令通道 + 唤醒事件交给渲染线程执行，调用方同步等待结果。
//! - 设备拔出 / 失效时调用 [`OutputShared::fail`] 并退出线程，由引擎重建输出。

use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{Error as WinError, GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_BUFFER_TOO_LARGE, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_DEVICE_IN_USE,
    AUDCLNT_E_ENDPOINT_CREATE_FAILED, AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED, AUDCLNT_E_RESOURCES_INVALIDATED,
    AUDCLNT_E_SERVICE_NOT_RUNNING, AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
    WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForMultipleObjects,
    INFINITE,
};

use super::{render, OutputFormat, OutputShared, SampleConsumer};

/// 播放中超过这么久没收到设备事件，就认为设备已失效（正常每个周期约 3~10 ms 一次）。
const DEVICE_TIMEOUT_MS: u32 = 2000;
/// play / pause 等待渲染线程回应的最长时间。
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// 设备样本格式与协商
// ---------------------------------------------------------------------------

/// 送给驱动的样本格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevSample {
    /// 32-bit 整型容器、高 24 位有效（左对齐，低 8 位为 0）——多数 24-bit DAC 的原生格式
    I32Valid24,
    /// 32-bit 整型，32 位全部有效
    I32,
    /// 24-bit 紧凑排列（每样本 3 字节，小端）
    I24Packed,
    /// 32-bit 浮点
    F32,
    /// 16-bit 整型
    I16,
}

/// 格式协商顺序：整型优先（驱动通常直接交给 DAC），浮点在独占模式下仍需驱动自行转换，排在整型之后；16-bit 兜底。
const NEGOTIATION_ORDER: [DevSample; 5] =
    [DevSample::I32Valid24, DevSample::I32, DevSample::I24Packed, DevSample::F32, DevSample::I16];

impl DevSample {
    fn container_bits(self) -> u16 {
        match self {
            Self::I32Valid24 | Self::I32 | Self::F32 => 32,
            Self::I24Packed => 24,
            Self::I16 => 16,
        }
    }
    fn valid_bits(self) -> u16 {
        match self {
            Self::I32Valid24 | Self::I24Packed => 24,
            Self::I32 | Self::F32 => 32,
            Self::I16 => 16,
        }
    }
    fn is_float(self) -> bool {
        self == Self::F32
    }
    /// 展示用的格式名（与 cpal 一致：32-bit 容器 24 位有效也记作 I24）
    fn label(self) -> &'static str {
        match self {
            Self::I32Valid24 | Self::I24Packed => "I24",
            Self::I32 => "I32",
            Self::F32 => "F32",
            Self::I16 => "I16",
        }
    }
    /// 该格式能无损承载的最大源位深
    fn lossless_bits(self) -> u32 {
        match self {
            Self::I32Valid24 | Self::I24Packed | Self::F32 => 24,
            Self::I32 => 32,
            Self::I16 => 16,
        }
    }
}

/// 各声道数可尝试的扬声器布局掩码：先试标准布局，最后试 0（KSAUDIO_SPEAKER_DIRECTOUT，不指定布局、按声道顺序直通）。
/// 位定义：FL 0x1 FR 0x2 FC 0x4 LFE 0x8 BL 0x10 BR 0x20 FLC 0x40 FRC 0x80 BC 0x100 SL 0x200 SR 0x400。
/// 交错顺序始终按位从低到高，与 symphonia 的声道顺序一致。
fn channel_masks(channels: u16) -> &'static [u32] {
    match channels {
        1 => &[0x4, 0],
        2 => &[0x3, 0],
        3 => &[0x7, 0],
        4 => &[0x33, 0x107, 0],
        5 => &[0x37, 0x607, 0],
        6 => &[0x3F, 0x60F, 0],
        7 => &[0x13F, 0x70F, 0],
        8 => &[0x63F, 0xFF, 0],
        _ => &[0],
    }
}

/// 构造格式描述。`extensible = false` 时生成传统 WAVEFORMATEX（cbSize = 0，个别老驱动只认这种），
/// 此时后面的扩展字段不会被读取。
fn wave_format(sample: DevSample, rate: u32, channels: u16, mask: u32, extensible: bool) -> WAVEFORMATEXTENSIBLE {
    let block_align = channels * sample.container_bits() / 8;
    let (tag, cb_size) = if extensible {
        (WAVE_FORMAT_EXTENSIBLE as u16, (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16)
    } else if sample.is_float() {
        (WAVE_FORMAT_IEEE_FLOAT as u16, 0)
    } else {
        (WAVE_FORMAT_PCM as u16, 0)
    };
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: tag,
            nChannels: channels,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: sample.container_bits(),
            cbSize: cb_size,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: sample.valid_bits() },
        dwChannelMask: mask,
        SubFormat: if sample.is_float() { KSDATAFORMAT_SUBTYPE_IEEE_FLOAT } else { KSDATAFORMAT_SUBTYPE_PCM },
    }
}

/// 按优先级列出所有待试格式。采样率、声道数严格等于请求值。
fn candidates(rate: u32, channels: u16) -> Vec<(DevSample, WAVEFORMATEXTENSIBLE)> {
    let mut v = Vec::new();
    for sample in NEGOTIATION_ORDER {
        for &mask in channel_masks(channels) {
            v.push((sample, wave_format(sample, rate, channels, mask, true)));
        }
        // 传统 WAVEFORMATEX 只能表达“有效位 = 容器位”且不超过 2 声道的格式
        if channels <= 2 && sample.valid_bits() == sample.container_bits() {
            v.push((sample, wave_format(sample, rate, channels, 0, false)));
        }
    }
    v
}

fn wf_ptr(wf: &WAVEFORMATEXTENSIBLE) -> *const WAVEFORMATEX {
    wf as *const WAVEFORMATEXTENSIBLE as *const WAVEFORMATEX
}

/// 帧数 → 100 ns 单位的时长（与微软示例相同的取整方式）。
fn frames_to_hns(frames: u32, rate: u32) -> i64 {
    (10_000_000.0 * frames as f64 / rate as f64 + 0.5) as i64
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// 设备已不可用（拔出、被禁用、音频服务停止、系统里改了设备格式等）。
fn is_device_lost(code: HRESULT) -> bool {
    matches!(
        code,
        AUDCLNT_E_DEVICE_INVALIDATED
            | AUDCLNT_E_RESOURCES_INVALIDATED
            | AUDCLNT_E_SERVICE_NOT_RUNNING
            | AUDCLNT_E_ENDPOINT_CREATE_FAILED
    )
}

/// 换其他格式也不会成功的初始化错误，遇到就不再尝试后续格式。
fn fatal_for_all_formats(code: HRESULT) -> bool {
    matches!(code, AUDCLNT_E_DEVICE_IN_USE | AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED) || is_device_lost(code)
}

fn describe(e: &WinError) -> String {
    match e.code() {
        AUDCLNT_E_DEVICE_IN_USE => "设备正被其他程序使用，无法独占（关闭正在使用该设备的程序后重试）".to_string(),
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => {
            "系统禁止了独占模式（声音设置 → 设备属性 → 高级 → 勾选“允许应用程序独占控制该设备”）".to_string()
        }
        AUDCLNT_E_UNSUPPORTED_FORMAT => "设备不支持该格式".to_string(),
        c if is_device_lost(c) => "音频设备已断开或失效".to_string(),
        _ => e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Win32 小工具
// ---------------------------------------------------------------------------

/// 自动复位事件句柄。只做 SetEvent / Wait，跨线程使用是安全的。
struct Event(HANDLE);

// SAFETY: 事件句柄是内核对象，可以在任意线程上 SetEvent / Wait；Drop 时只关闭一次。
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Self, String> {
        // 自动复位、初始无信号、匿名
        unsafe { CreateEventW(None, false, false, None) }.map(Self).map_err(|e| format!("创建事件失败: {e}"))
    }
    fn set(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 解析 cpal `DeviceId` 的字符串形式（`wasapi:<端点 ID>`），取出 WASAPI 端点 ID。
/// 端点 ID 形如 `{0.0.0.00000000}.{guid}`，本身不含冒号；cpal 也是按第一个冒号切分。
fn parse_endpoint_id(device_id: &str) -> Option<&str> {
    let (host, id) = device_id.split_once(':')?;
    (host.eq_ignore_ascii_case("wasapi") && !id.is_empty()).then_some(id)
}

/// 找到设置里选的设备；找不到（已拔出、不是 WASAPI 设备）时退回系统默认输出设备，与 `output::find_device` 一致。
unsafe fn find_device(enumerator: &IMMDeviceEnumerator, device_id: Option<&str>) -> Result<IMMDevice, String> {
    if let Some(endpoint) = device_id.and_then(parse_endpoint_id) {
        if let Ok(device) = enumerator.GetDevice(&HSTRING::from(endpoint)) {
            if device.GetState().map(|s| s == DEVICE_STATE_ACTIVE).unwrap_or(false) {
                return Ok(device);
            }
        }
    }
    enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)
        .map_err(|e| format!("没有可用的音频输出设备: {e}"))
}

/// 设备友好名（PKEY_Device_FriendlyName），与 cpal 的 `description().name()` 取的是同一属性。
unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let key = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };
    let mut value = store.GetValue(&key).ok()?;
    let name = PropVariantToStringAlloc(&value).ok().and_then(|p| {
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    });
    let _ = PropVariantClear(&mut value);
    name.filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// 渲染会话（只在渲染线程上存在）
// ---------------------------------------------------------------------------

/// 渲染前的中间缓冲：`render` 写成目标样本类型，再按字节拷进设备缓冲（避免对设备指针的对齐假设）。
enum Scratch {
    Int(Vec<i32>),
    Float(Vec<f32>),
    Short(Vec<i16>),
}

struct Session {
    // 字段按释放顺序排列：先释放 render_client / client，最后才关闭它们引用的事件句柄
    render_client: IAudioRenderClient,
    client: IAudioClient,
    event: Event,
    sample: DevSample,
    /// 设备缓冲帧数（独占事件模式下 = 一个周期），每次事件都整块填满
    buffer_frames: u32,
    channels: usize,
    scratch: Scratch,
}

/// 以独占事件模式初始化一个新的 IAudioClient。缓冲时长用设备默认周期（拿不到时用最小周期）；
/// 驱动要求对齐时（`AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED`）按它给出的帧数重算时长、换一个新客户端再初始化。
unsafe fn init_client(device: &IMMDevice, wf: &WAVEFORMATEXTENSIBLE, rate: u32) -> windows::core::Result<IAudioClient> {
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let mut default_period = 0i64;
    let mut min_period = 0i64;
    client.GetDevicePeriod(Some(&mut default_period), Some(&mut min_period))?;
    let period = if default_period > 0 { default_period } else { min_period };

    // 独占事件模式要求 缓冲时长 == 周期
    match client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, period, wf_ptr(wf), None) {
        Ok(()) => Ok(client),
        Err(e) if e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
            // 失败的客户端上 GetBufferSize 返回驱动对齐后的帧数；按微软文档的做法换新客户端重试
            let aligned_frames = client.GetBufferSize()?;
            drop(client);
            let period = frames_to_hns(aligned_frames, rate);
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                period,
                period,
                wf_ptr(wf),
                None,
            )?;
            Ok(client)
        }
        Err(e) => Err(e),
    }
}

impl Session {
    /// 打开设备、协商格式、初始化独占流。返回 (会话, 实际格式, 设备友好名)。
    unsafe fn open(device_id: Option<&str>, rate: u32, channels: u16) -> Result<(Self, OutputFormat, String), String> {
        let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| format!("无法创建音频设备枚举器: {e}"))?;
        let device = find_device(&enumerator, device_id)?;
        let name = friendly_name(&device).unwrap_or_else(|| "音频设备".to_string());

        // 先用一个未初始化的客户端筛出驱动声明支持的格式（独占模式不提供“最接近格式”，ppClosestMatch 传空）
        let probe =
            device.Activate::<IAudioClient>(CLSCTX_ALL, None).map_err(|e| format!("无法打开音频设备「{name}」: {}", describe(&e)))?;
        let supported: Vec<_> = candidates(rate, channels)
            .into_iter()
            .filter(|(_, wf)| probe.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr(wf), None) == S_OK)
            .collect();
        drop(probe);
        if supported.is_empty() {
            return Err(format!("设备「{name}」在独占模式下不支持 {rate} Hz / {channels} 声道的整型或浮点格式"));
        }

        // IsFormatSupported 通过后 Initialize 仍可能失败（个别驱动），依次尝试
        let mut last_err = None;
        for (sample, wf) in &supported {
            match init_client(&device, wf, rate) {
                Ok(client) => return Self::finish(client, *sample, rate, channels, name),
                Err(e) => {
                    let stop = fatal_for_all_formats(e.code());
                    last_err = Some(e);
                    if stop {
                        break;
                    }
                }
            }
        }
        let reason = last_err.map(|e| describe(&e)).unwrap_or_default();
        Err(format!("独占模式打开设备「{name}」失败: {reason}"))
    }

    unsafe fn finish(
        client: IAudioClient,
        sample: DevSample,
        rate: u32,
        channels: u16,
        name: String,
    ) -> Result<(Self, OutputFormat, String), String> {
        let event = Event::new()?;
        client.SetEventHandle(event.0).map_err(|e| format!("设置音频事件失败: {e}"))?;
        let buffer_frames = client.GetBufferSize().map_err(|e| format!("读取设备缓冲大小失败: {e}"))?;
        let render_client =
            client.GetService::<IAudioRenderClient>().map_err(|e| format!("获取音频渲染接口失败: {e}"))?;
        let len = buffer_frames as usize * channels as usize;
        let scratch = match sample {
            DevSample::F32 => Scratch::Float(vec![0.0; len]),
            DevSample::I16 => Scratch::Short(vec![0; len]),
            DevSample::I32Valid24 | DevSample::I32 | DevSample::I24Packed => Scratch::Int(vec![0; len]),
        };
        let format = OutputFormat {
            sample_rate: rate,
            channels,
            sample_format: sample.label().to_string(),
            lossless_bits: sample.lossless_bits(),
            exclusive: true,
        };
        let session = Self { render_client, client, event, sample, buffer_frames, channels: channels as usize, scratch };
        Ok((session, format, name))
    }

    /// 取一整块设备缓冲并填满。
    unsafe fn fill(&mut self, consumer: &SampleConsumer, shared: &OutputShared, vol: &mut f32) -> windows::core::Result<()> {
        let frames = self.buffer_frames;
        let data = match self.render_client.GetBuffer(frames) {
            Ok(p) => p,
            // 设备还没消耗完上一块（个别驱动会多发一次事件），这次跳过
            Err(e) if e.code() == AUDCLNT_E_BUFFER_TOO_LARGE => return Ok(()),
            Err(e) => return Err(e),
        };
        let samples = frames as usize * self.channels;
        let ch = self.channels;
        // Windows 只跑在小端平台上，按内存字节拷贝即为小端 PCM
        match (&mut self.scratch, self.sample) {
            (Scratch::Float(buf), _) => {
                render::<f32>(buf.as_mut_slice(), ch, consumer, shared, vol);
                std::ptr::copy_nonoverlapping(buf.as_ptr() as *const u8, data, samples * 4);
            }
            (Scratch::Short(buf), _) => {
                render::<i16>(buf.as_mut_slice(), ch, consumer, shared, vol);
                std::ptr::copy_nonoverlapping(buf.as_ptr() as *const u8, data, samples * 2);
            }
            (Scratch::Int(buf), DevSample::I24Packed) => {
                // 先渲染成满幅 i32（24-bit 值左对齐），再取高 3 字节紧凑排列
                render::<i32>(buf.as_mut_slice(), ch, consumer, shared, vol);
                let dst = std::slice::from_raw_parts_mut(data, samples * 3);
                for (o, s) in dst.chunks_exact_mut(3).zip(buf.iter()) {
                    o.copy_from_slice(&s.to_le_bytes()[1..4]);
                }
            }
            (Scratch::Int(buf), _) => {
                // 32 位全有效与“高 24 位有效”都直接写满幅 i32：后者驱动只取高 24 位，正好是左对齐的 24-bit 样本
                render::<i32>(buf.as_mut_slice(), ch, consumer, shared, vol);
                std::ptr::copy_nonoverlapping(buf.as_ptr() as *const u8, data, samples * 4);
            }
        }
        self.render_client.ReleaseBuffer(frames, 0)
    }
}

// ---------------------------------------------------------------------------
// 渲染线程
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    Play,
    Pause,
}

type Reply = mpsc::Sender<Result<(), String>>;

struct ThreadArgs {
    device_id: Option<String>,
    sample_rate: u32,
    channels: u16,
    consumer: SampleConsumer,
    shared: Arc<OutputShared>,
    cmd_rx: mpsc::Receiver<(Cmd, Reply)>,
    cmd_event: Arc<Event>,
    quit: Arc<AtomicBool>,
    init_tx: mpsc::Sender<Result<(OutputFormat, String), String>>,
}

fn render_thread(args: ThreadArgs) {
    // 新线程，进入 MTA 不会与已有的 COM 模式冲突
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        let _ = args.init_tx.send(Err(format!("初始化 COM 失败: {e}")));
        return;
    }
    // 所有 COM 对象都在这个函数里创建并释放，保证先于 CoUninitialize
    run(args);
    unsafe { CoUninitialize() };
}

fn run(args: ThreadArgs) {
    let ThreadArgs { device_id, sample_rate, channels, consumer, shared, cmd_rx, cmd_event, quit, init_tx } = args;
    let mut session = match unsafe { Session::open(device_id.as_deref(), sample_rate, channels) } {
        Ok((session, format, name)) => {
            let _ = init_tx.send(Ok((format, name)));
            session
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    drop(init_tx);

    // MMCSS 提升线程优先级；失败（比如服务被禁用）不影响播放
    let mut task_index = 0u32;
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(&HSTRING::from("Pro Audio"), &mut task_index) }.ok();

    let mut vol = shared.volume();
    let mut playing = false;
    // 首次 Start 前要先填好一块缓冲（微软推荐，避免开头毛刺）；暂停用 Stop，缓冲里的数据保留，恢复时无需再预填
    let mut primed = false;
    // 设备事件放在前面：两者同时有信号时优先喂设备
    let handles = [session.event.0, cmd_event.0];

    loop {
        if quit.load(Ordering::Acquire) {
            break;
        }
        let timeout = if playing { DEVICE_TIMEOUT_MS } else { INFINITE };
        let r = unsafe { WaitForMultipleObjects(&handles, false, timeout) };
        if r == WAIT_OBJECT_0 {
            if !playing {
                // Stop 前最后一次事件的残留信号
                continue;
            }
            if let Err(e) = unsafe { session.fill(&consumer, &shared, &mut vol) } {
                shared.fail(format!("音频输出中断（独占模式）: {}", describe(&e)));
                break;
            }
        } else if r.0 == WAIT_OBJECT_0.0 + 1 {
            // 命令或退出请求（退出在下一轮循环开头处理）
            let mut lost = false;
            while let Ok((cmd, reply)) = cmd_rx.try_recv() {
                let res = match cmd {
                    Cmd::Play if !playing => {
                        let prime = if primed { Ok(()) } else { unsafe { session.fill(&consumer, &shared, &mut vol) } };
                        match prime {
                            Ok(()) => unsafe { session.client.Start() },
                            Err(e) => Err(e),
                        }
                    }
                    Cmd::Pause if playing => unsafe { session.client.Stop() },
                    _ => Ok(()),
                };
                match res {
                    Ok(()) => {
                        if cmd == Cmd::Play {
                            primed = true;
                        }
                        playing = cmd == Cmd::Play;
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        let msg = match cmd {
                            Cmd::Play => format!("开始播放失败: {}", describe(&e)),
                            Cmd::Pause => format!("暂停失败: {}", describe(&e)),
                        };
                        if is_device_lost(e.code()) {
                            shared.fail(msg.clone());
                            lost = true;
                        }
                        let _ = reply.send(Err(msg));
                        if lost {
                            break;
                        }
                    }
                }
            }
            if lost {
                break;
            }
        } else if r == WAIT_TIMEOUT {
            shared.fail("音频设备长时间没有响应（独占模式），可能已断开".to_string());
            break;
        } else {
            shared.fail(format!("等待音频设备事件失败: {}", WinError::from_thread()));
            break;
        }
    }

    unsafe {
        let _ = session.client.Stop();
    }
    if let Some(h) = mmcss {
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
    }
    // session 在此释放（IAudioRenderClient → IAudioClient → 事件句柄），设备随之解除独占
    drop(session);
}

// ---------------------------------------------------------------------------
// 独占模式能力探测
// ---------------------------------------------------------------------------

/// (端点 ID, 声道数, 采样率) → 独占模式是否支持。设备能力基本不变，而设备被本程序独占期间无法再探测，所以缓存。
static RATE_CACHE: OnceLock<Mutex<HashMap<(String, u16, u32), bool>>> = OnceLock::new();

/// 设备在独占模式下支持的采样率（从 `rates` 中筛选，声道数严格匹配）。
///
/// cpal 的 WASAPI 后端按共享模式报告能力：输出流带 AUTOCONVERTPCM，由 Windows 混音器自动转换，
/// 几乎所有常见采样率都显示为“支持”，据此选出的采样率在独占模式下未必能打开。
/// 开启独占模式时应改用本函数的结果来选输出采样率（不支持时由引擎重采样到设备支持的采样率，仍走独占）。
///
/// 一次传入完整的候选列表（如 `COMMON_RATES`），首次探测后结果按端点缓存。探测失败（设备不存在、
/// 正被其他程序占用且无缓存）返回空表，调用方应退回 cpal 的结果。会短暂开一个线程做 COM 调用，不阻塞调用方的 COM 模式。
pub fn exclusive_sample_rates(device_id: Option<&str>, channels: u16, rates: &[u32]) -> Vec<u32> {
    if channels == 0 {
        return Vec::new();
    }
    let device_id = device_id.map(String::from);
    let rates = rates.to_vec();
    std::thread::Builder::new()
        .name("wasapi-probe".into())
        .spawn(move || {
            if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
                return Vec::new();
            }
            let out = unsafe { probe_rates(device_id.as_deref(), channels, &rates) }.unwrap_or_default();
            unsafe { CoUninitialize() };
            out
        })
        .ok()
        .and_then(|t| t.join().ok())
        .unwrap_or_default()
}

unsafe fn endpoint_id(device: &IMMDevice) -> Option<String> {
    let p = device.GetId().ok()?;
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

unsafe fn probe_rates(device_id: Option<&str>, channels: u16, rates: &[u32]) -> Option<Vec<u32>> {
    let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    let device = find_device(&enumerator, device_id).ok()?;
    let endpoint = endpoint_id(&device)?;
    let cache = RATE_CACHE.get_or_init(Default::default);
    let mut client: Option<IAudioClient> = None;
    let mut out = Vec::new();
    for &rate in rates {
        let key = (endpoint.clone(), channels, rate);
        let cached = cache.lock().unwrap().get(&key).copied();
        let ok = match cached {
            Some(v) => v,
            None => {
                if client.is_none() {
                    client = Some(device.Activate::<IAudioClient>(CLSCTX_ALL, None).ok()?);
                }
                let c = client.as_ref()?;
                let mut ok = false;
                // 只有所有候选都明确返回“不支持格式”时才缓存否定结果；其他错误（如设备被占用）下次再探测
                let mut definitive = true;
                for (_, wf) in candidates(rate, channels) {
                    let hr = c.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, wf_ptr(&wf), None);
                    if hr == S_OK {
                        ok = true;
                        break;
                    }
                    if hr != AUDCLNT_E_UNSUPPORTED_FORMAT {
                        definitive = false;
                    }
                }
                if ok || definitive {
                    cache.lock().unwrap().insert(key, ok);
                }
                ok
            }
        };
        if ok {
            out.push(rate);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// 对外接口
// ---------------------------------------------------------------------------

/// 一条 WASAPI 独占模式输出流。渲染线程持有设备；本结构只负责发命令，Drop 时停止并 join 渲染线程。
pub struct ExclusiveStream {
    cmd_tx: mpsc::Sender<(Cmd, Reply)>,
    cmd_event: Arc<Event>,
    quit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ExclusiveStream {
    /// 以独占模式打开设备并准备好流（初始为暂停，不出声）。
    ///
    /// `device_id`: None = 系统默认输出设备；Some 为 cpal `DeviceId` 的字符串（`wasapi:<端点 ID>`），
    /// 找不到时退回默认设备。采样率与声道数必须被设备原样支持，否则返回 Err（调用方退回共享模式）。
    /// 返回 (流, 实际输出格式, 设备友好名)。
    pub fn open(
        device_id: Option<&str>,
        sample_rate: u32,
        channels: u16,
        consumer: SampleConsumer,
        shared: Arc<OutputShared>,
    ) -> Result<(Self, OutputFormat, String), String> {
        if sample_rate == 0 || channels == 0 {
            return Err("无效的输出格式".to_string());
        }
        let cmd_event = Arc::new(Event::new()?);
        let quit = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::channel();
        let args = ThreadArgs {
            device_id: device_id.map(String::from),
            sample_rate,
            channels,
            consumer,
            shared,
            cmd_rx,
            cmd_event: cmd_event.clone(),
            quit: quit.clone(),
            init_tx,
        };
        let thread = std::thread::Builder::new()
            .name("wasapi-exclusive".into())
            .spawn(move || render_thread(args))
            .map_err(|e| format!("无法创建音频渲染线程: {e}"))?;

        match init_rx.recv() {
            Ok(Ok((format, name))) => Ok((Self { cmd_tx, cmd_event, quit, thread: Some(thread) }, format, name)),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("音频渲染线程意外退出".to_string())
            }
        }
    }

    fn command(&self, cmd: Cmd) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.cmd_tx.send((cmd, tx)).map_err(|_| "独占模式输出已停止".to_string())?;
        self.cmd_event.set();
        rx.recv_timeout(COMMAND_TIMEOUT).map_err(|_| "独占模式输出没有响应".to_string())?
    }

    pub fn play(&self) -> Result<(), String> {
        self.command(Cmd::Play)
    }

    pub fn pause(&self) -> Result<(), String> {
        self.command(Cmd::Pause)
    }
}

impl Drop for ExclusiveStream {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Release);
        self.cmd_event.set();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
