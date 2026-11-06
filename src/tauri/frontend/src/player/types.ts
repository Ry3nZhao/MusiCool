// 音乐库中的一首歌（每首歌一条，歌词与封面是附属字段）
export interface FileInfo {
  path: string;
  name: string; // 标题，没有标签时为文件名
  file_name: string;
  music_id?: number;
  size: number;
  artist?: string;
  album?: string;
  duration?: number; // 秒
  format: string;
  lyrics_path?: string;
  cover_path?: string;
}

export type Repeat = "off" | "all" | "one";
export type PlayState = "stopped" | "playing" | "paused";

export interface SourceInfo {
  codec: string;
  lossless: boolean;
  sample_rate: number;
  channels: number;
  bits_per_sample?: number;
  duration?: number;
  bitrate_kbps?: number;
}

export interface OutputFormat {
  sample_rate: number;
  channels: number;
  sample_format: string;
  lossless_bits: number;
  exclusive: boolean;
}

export interface SignalPath {
  source: SourceInfo;
  output: OutputFormat;
  device: string;
  resampled: boolean;
  bit_perfect: boolean;
  notes: string[];
}

export interface PlayerStatus {
  state: PlayState;
  queue_index?: number;
  queue_len: number;
  queue_id: number;
  path?: string;
  position: number;
  duration: number;
  volume: number;
  repeat: Repeat;
  shuffle: boolean;
  signal?: SignalPath;
  error?: string;
}

export interface DeviceInfo {
  id: string;
  name: string;
  is_default: boolean;
  sample_rates: number[];
}

export const formatTime = (seconds?: number): string => {
  if (!seconds || !isFinite(seconds) || seconds < 0) return "0:00";
  const s = Math.floor(seconds);
  const m = Math.floor(s / 60);
  return `${m}:${(s % 60).toString().padStart(2, "0")}`;
};

export const formatRate = (hz: number): string => {
  const k = hz / 1000;
  return `${Number.isInteger(k) ? k : k.toFixed(1)} kHz`;
};

/** 例如 "FLAC 24/192" 或 "MP3 320k" */
export const sourceLabel = (s: SourceInfo): string => {
  const k = s.sample_rate / 1000;
  const rate = Number.isInteger(k) ? `${k}` : k.toFixed(1);
  if (s.bits_per_sample) return `${s.codec} ${s.bits_per_sample}/${rate}`;
  return `${s.codec} ${rate} kHz${s.bitrate_kbps ? ` · ${s.bitrate_kbps}k` : ""}`;
};

/** 音量滑块（0–1，线性手感）与实际增益的换算：平方曲线更符合听感；滑块到头即 1.0（bit-perfect）。 */
export const sliderToGain = (x: number) => x * x;
export const gainToSlider = (g: number) => Math.sqrt(Math.max(0, g));

export const fileStem = (path?: string) => (path ? path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "" : "");
