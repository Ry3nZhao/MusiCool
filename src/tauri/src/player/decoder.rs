//! 用 symphonia 把音频文件解码成交错的 f32 样本。
//!
//! 整数 PCM（16/24 bit）转成 f32 是无损的（f32 有 24 位尾数），所以音量 100% 时输出与源文件逐位一致。
//! 解码器默认开启 gapless：MP3/AAC 的编码器延迟与尾部填充会被裁掉，曲间不会多出静音。

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

/// 源文件的格式信息，用于展示信号链路。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceInfo {
    /// 编码名称，如 FLAC / ALAC / MP3 / AAC / PCM
    pub codec: String,
    pub lossless: bool,
    pub sample_rate: u32,
    pub channels: u16,
    /// 位深（仅无损/PCM 格式有）
    pub bits_per_sample: Option<u32>,
    pub duration: Option<f64>,
    /// 平均音频码率（kbps）
    pub bitrate_kbps: Option<u32>,
}

pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    pub info: SourceInfo,
    buf: Vec<f32>,
    /// 最近一次解码出的声道数（极少数文件中途会变）
    pub channels: usize,
}

fn codec_label(short: &str) -> (String, bool) {
    match short {
        "flac" => ("FLAC".into(), true),
        "alac" => ("ALAC".into(), true),
        "mp3" | "mp2" | "mp1" => (short.to_uppercase(), false),
        "aac" => ("AAC".into(), false),
        "vorbis" => ("Vorbis".into(), false),
        s if s.starts_with("pcm") => ("PCM".into(), true),
        s => (s.to_uppercase(), false),
    }
}

impl TrackDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("打开文件失败: {e}"))?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| format!("无法识别的音频格式: {e}"))?;
        let track = format.default_track(TrackType::Audio).ok_or("文件中没有音频轨")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or("文件中没有音频轨")?
            .clone();
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default().gapless(true))
            .map_err(|e| format!("不支持的编码: {e}"))?;

        let (codec, lossless) = codec_label(decoder.codec_info().short_name);
        let sample_rate = params.sample_rate.ok_or("未知采样率")?;
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2) as u16;
        let duration = track.num_frames.map(|n| n as f64 / sample_rate as f64);
        let (track_id, time_base) = (track.id, track.time_base);
        // 码率优先取 lofty 解析的音频码率；按文件大小估算会把内嵌封面也算进去
        let bitrate_kbps = {
            use lofty::file::AudioFile;
            lofty::read_from_path(path)
                .ok()
                .and_then(|f| f.properties().audio_bitrate())
                .filter(|b| *b > 0)
                .or_else(|| duration.filter(|d| *d > 0.0).map(|d| (size as f64 * 8.0 / d / 1000.0).round() as u32))
        };

        Ok(Self {
            track_id,
            time_base,
            format,
            decoder,
            info: SourceInfo {
                codec,
                lossless,
                sample_rate,
                channels,
                bits_per_sample: if lossless { params.bits_per_sample } else { None },
                duration,
                bitrate_kbps,
            },
            buf: Vec::new(),
            channels: channels as usize,
        })
    }

    /// 解码下一段，返回（交错样本, 声道数）；`None` 表示播放到结尾。
    pub fn next(&mut self) -> Result<Option<(&[f32], usize)>, String> {
        if self.decode_next()? {
            let ch = self.channels;
            Ok(Some((&self.buf, ch)))
        } else {
            Ok(None)
        }
    }

    fn decode_next(&mut self) -> Result<bool, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(format!("读取音频数据失败: {e}")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // 个别损坏的包跳过即可，不中断播放
                Err(SymError::DecodeError(_)) | Err(SymError::IoError(_)) => continue,
                Err(e) => return Err(format!("解码失败: {e}")),
            };
            if decoded.frames() == 0 {
                continue;
            }
            self.channels = decoded.spec().channels().count();
            decoded.copy_to_vec_interleaved(&mut self.buf);
            if self.buf.is_empty() {
                continue;
            }
            return Ok(true);
        }
    }

    /// 跳转到指定秒数，返回实际落点（秒）。
    pub fn seek(&mut self, secs: f64) -> Result<f64, String> {
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: Time::try_from_secs_f64(secs.max(0.0)).ok_or("跳转位置无效")?,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| format!("跳转失败: {e}"))?;
        self.decoder.reset();
        Ok(self
            .time_base
            .and_then(|tb| tb.calc_time(seeked.actual_ts))
            .map(|t| t.as_secs_f64())
            .unwrap_or(secs))
    }
}
