//! 解码结果到输出格式的转换：声道映射 + （必要时）采样率转换。
//!
//! 源与输出采样率、声道一致时（绝大多数情况）这里什么都不做，样本原样通过。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

/// 把 `in_ch` 声道的交错样本映射到 `out_ch` 声道，追加到 `dst`。
pub fn map_channels(src: &[f32], in_ch: usize, out_ch: usize, dst: &mut Vec<f32>) {
    if in_ch == out_ch {
        dst.extend_from_slice(src);
        return;
    }
    dst.reserve(src.len() / in_ch * out_ch);
    for f in src.chunks_exact(in_ch) {
        if in_ch == 1 {
            // 单声道：左右声道各一份，其余声道静音
            for c in 0..out_ch {
                dst.push(if c < 2 { f[0] } else { 0.0 });
            }
        } else if in_ch > 2 && out_ch == 2 {
            // 多声道下混到立体声（ITU-R BS.775，声道顺序 FL FR FC LFE BL BR SL SR），按系数和归一化防止削波
            const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
            let g = |i: usize| f.get(i).copied().unwrap_or(0.0);
            let surround_l = g(4) + g(6);
            let surround_r = g(5) + g(7);
            let norm = 1.0 / (1.0 + K + if in_ch > 4 { K } else { 0.0 });
            dst.push((g(0) + K * g(2) + K * surround_l) * norm);
            dst.push((g(1) + K * g(2) + K * surround_r) * norm);
        } else {
            for c in 0..out_ch {
                dst.push(if c < in_ch { f[c] } else { 0.0 });
            }
        }
    }
}

/// 高质量定比重采样（rubato FFT，带抗混叠滤波），去掉滤波器延迟，尾部补齐，输出长度精确等于 输入×比例。
pub struct Resampling {
    rs: Fft<f32>,
    ch: usize,
    from: u64,
    to: u64,
    inbuf: Vec<f32>,
    outbuf: Vec<f32>,
    /// 还需丢弃的输出帧（滤波器延迟）
    skip: usize,
    in_frames: u64,
    out_frames: u64,
}

impl Resampling {
    pub fn new(from: u32, to: u32, ch: usize) -> Result<Self, String> {
        let rs = Fft::<f32>::new(from as usize, to as usize, 1024, ch, FixedSync::Input)
            .map_err(|e| format!("创建重采样器失败: {e}"))?;
        Ok(Self {
            ch,
            from: from as u64,
            to: to as u64,
            outbuf: vec![0.0; rs.output_frames_max() * ch],
            skip: rs.output_delay(),
            rs,
            inbuf: Vec::new(),
            in_frames: 0,
            out_frames: 0,
        })
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.inbuf.extend_from_slice(input);
        self.in_frames += (input.len() / self.ch) as u64;
        while self.inbuf.len() >= self.rs.input_frames_next() * self.ch {
            self.run(None, out);
        }
    }

    /// 输入结束：处理剩余样本并冲出滤波器里的尾巴。
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let rem = self.inbuf.len() / self.ch;
        if rem > 0 {
            self.run(Some(rem), out);
        }
        let expected = self.expected();
        let mut guard = 0;
        while self.out_frames < expected && guard < 64 {
            self.run(Some(0), out);
            guard += 1;
        }
    }

    fn expected(&self) -> u64 {
        // 整数运算，避免浮点误差多出一帧
        (self.in_frames * self.to + self.from / 2) / self.from
    }

    fn run(&mut self, partial: Option<usize>, out: &mut Vec<f32>) {
        let need = self.rs.input_frames_next();
        if self.inbuf.len() < need * self.ch {
            self.inbuf.resize(need * self.ch, 0.0);
        }
        let max_out = self.outbuf.len() / self.ch;
        let result = {
            let input = InterleavedSlice::new(&self.inbuf[..need * self.ch], self.ch, need).expect("输入缓冲长度");
            let mut output = InterleavedSlice::new_mut(&mut self.outbuf[..], self.ch, max_out).expect("输出缓冲长度");
            let indexing = partial.map(|n| Indexing::new().partial_len(n));
            self.rs.process_into_buffer(&input, &mut output, indexing.as_ref())
        };
        let consumed = match partial {
            Some(_) => self.inbuf.len() / self.ch,
            None => need,
        };
        self.inbuf.drain(..(consumed * self.ch).min(self.inbuf.len()));
        let Ok((_, produced)) = result else { return };
        let limit = self.expected();
        for frame in self.outbuf[..produced * self.ch].chunks_exact(self.ch) {
            if self.skip > 0 {
                self.skip -= 1;
                continue;
            }
            if self.out_frames >= limit {
                break;
            }
            out.extend_from_slice(frame);
            self.out_frames += 1;
        }
    }
}

/// 一首歌的转换管线。
pub struct Converter {
    out_ch: usize,
    resampler: Option<Resampling>,
    tmp: Vec<f32>,
}

impl Converter {
    pub fn new(src_rate: u32, out_rate: u32, out_ch: usize) -> Result<Self, String> {
        let resampler = if src_rate != out_rate { Some(Resampling::new(src_rate, out_rate, out_ch)?) } else { None };
        Ok(Self { out_ch, resampler, tmp: Vec::new() })
    }

    pub fn resampling(&self) -> bool {
        self.resampler.is_some()
    }

    pub fn process(&mut self, src: &[f32], in_ch: usize, out: &mut Vec<f32>) {
        match &mut self.resampler {
            None => map_channels(src, in_ch, self.out_ch, out),
            Some(rs) => {
                self.tmp.clear();
                map_channels(src, in_ch, self.out_ch, &mut self.tmp);
                rs.process(&self.tmp, out);
            }
        }
    }

    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if let Some(rs) = &mut self.resampler {
            rs.finish(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo() {
        let mut out = Vec::new();
        map_channels(&[0.5, -0.25], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn passthrough_is_bit_exact() {
        let src: Vec<f32> = (0..4096).map(|i| ((i * 7919) % 16777216) as f32 / 8388608.0 - 1.0).collect();
        let mut c = Converter::new(96000, 96000, 2).unwrap();
        let mut out = Vec::new();
        c.process(&src, 2, &mut out);
        c.finish(&mut out);
        assert_eq!(out, src);
    }

    #[test]
    fn resample_length_and_tone() {
        // 44.1k → 48k 的 1 kHz 正弦：长度精确，且幅度基本保持（无延迟错位、无爆音）
        let n = 44100;
        let src: Vec<f32> = (0..n)
            .flat_map(|i| {
                let v = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin() * 0.5;
                [v, v]
            })
            .collect();
        let mut c = Converter::new(44100, 48000, 2).unwrap();
        let mut out = Vec::new();
        for chunk in src.chunks(2 * 1152) {
            c.process(chunk, 2, &mut out);
        }
        c.finish(&mut out);
        assert_eq!(out.len() / 2, 48000);
        let peak = out[2000..90000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.01, "peak={peak}");
        // 与理想 48k 正弦对齐（检查没有相位/延迟错位）
        let err = (2400..2500)
            .map(|i| {
                let ideal = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin() * 0.5;
                (out[i * 2] - ideal).abs()
            })
            .fold(0f32, f32::max);
        assert!(err < 0.01, "err={err}");
    }
}
