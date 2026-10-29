//! 网易云音乐 NCM 文件格式解析。
//!
//! 文件布局（整数均为小端 u32）：
//!
//! ```text
//! magic "CTENFDAM" (8) | gap (2)
//! key_len (4)   | key_data   (key_len, 每字节 ^0x64, AES-128-ECB, PKCS7)
//! meta_len (4)  | meta_data  (meta_len, 每字节 ^0x63, 去掉 "163 key(Don't modify):" 后 base64 + AES-128-ECB)
//! crc32 (4)     | gap (1)
//! cover_frame_len (4) | cover_len (4) | cover (cover_len) | 填充 (cover_frame_len - cover_len)
//! audio (RC4 变体加密，直到文件末尾)
//! ```
//!
//! 注意：旧的 C++ 实现跳过了 `cover_frame_len`，把它前面的 5 字节当成了 "gap"，
//! 把 `cover_len` 当成了整个封面区长度，于是封面区存在填充时（很常见），
//! 音频起点会错位，导致整个音频流的密钥流对不上，输出文件损坏。

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use aes::Aes128;
use base64::Engine;
use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"CTENFDAM";
const CORE_KEY: &[u8; 16] = b"hzHRAmso5kInbaxW";
const META_KEY: &[u8; 16] = b"#14ljk_!\\]&0U<'(";
const KEY_PREFIX: &[u8] = b"neteasecloudmusic";
const META_PREFIX: &[u8] = b"163 key(Don't modify):";
const MUSIC_PREFIX: &[u8] = b"music:";

#[derive(Debug, thiserror::Error)]
pub enum NcmError {
    #[error("I/O 错误: {0}")]
    Io(#[from] io::Error),
    #[error("不是有效的 NCM 文件")]
    BadMagic,
    #[error("文件结构损坏: {0}")]
    Corrupted(&'static str),
}

/// NCM 内嵌的歌曲元数据（只取用得到的字段）。
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NcmMeta {
    #[serde(default)]
    pub music_name: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    /// `[["歌手名", id], ...]`
    #[serde(default)]
    pub artist: Vec<(String, serde_json::Value)>,
    #[serde(default)]
    pub format: Option<String>,
}

impl NcmMeta {
    pub fn artists(&self) -> Option<String> {
        if self.artist.is_empty() {
            None
        } else {
            Some(self.artist.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join("/"))
        }
    }
}

/// 解码输出信息。
#[derive(Debug, Clone)]
pub struct DecodedFile {
    pub output_path: PathBuf,
    pub meta: Option<NcmMeta>,
    pub cover: Option<Vec<u8>>,
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, NcmError> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_vec<R: Read>(r: &mut R, len: u32) -> Result<Vec<u8>, NcmError> {
    let mut v = vec![0u8; len as usize];
    r.read_exact(&mut v)?;
    Ok(v)
}

fn aes128_ecb_decrypt(key: &[u8; 16], data: &mut Vec<u8>) -> Result<(), NcmError> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(NcmError::Corrupted("AES 数据长度不是 16 的倍数"));
    }
    let cipher = Aes128::new(GenericArray::from_slice(key));
    for block in data.chunks_exact_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    let pad = *data.last().unwrap() as usize;
    if pad == 0 || pad > 16 || data[data.len() - pad..].iter().any(|&b| b as usize != pad) {
        return Err(NcmError::Corrupted("PKCS7 填充无效"));
    }
    data.truncate(data.len() - pad);
    Ok(())
}

/// NCM 使用的 RC4 变体：只做 KSA，密钥流按音频内偏移计算（与位置相关，与前文无关）。
struct KeyStream {
    stream: [u8; 256],
}

impl KeyStream {
    fn new(key: &[u8]) -> Self {
        let mut sbox = [0u8; 256];
        for (i, b) in sbox.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut j: u8 = 0;
        for i in 0..256 {
            j = j.wrapping_add(sbox[i]).wrapping_add(key[i % key.len()]);
            sbox.swap(i, j as usize);
        }
        let mut stream = [0u8; 256];
        for (i, s) in stream.iter_mut().enumerate() {
            let k = (i + 1) & 0xff;
            let a = sbox[k];
            *s = sbox[(a as usize + sbox[(a as usize + k) & 0xff] as usize) & 0xff];
        }
        Self { stream }
    }

    /// `offset` 是 `buf[0]` 在音频数据中的偏移。
    fn apply(&self, offset: u64, buf: &mut [u8]) {
        let base = (offset & 0xff) as usize;
        for (i, b) in buf.iter_mut().enumerate() {
            *b ^= self.stream[(base + i) & 0xff];
        }
    }
}

/// 根据文件头识别音频格式，比元数据里的 `format` 字段更可靠。
fn sniff_format(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"fLaC") {
        Some("flac")
    } else if head.starts_with(b"ID3") || (head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0) {
        Some("mp3")
    } else if head.starts_with(b"OggS") {
        Some("ogg")
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        Some("m4a")
    } else {
        None
    }
}

fn parse_meta(mut raw: Vec<u8>) -> Option<NcmMeta> {
    raw.iter_mut().for_each(|b| *b ^= 0x63);
    let b64 = raw.strip_prefix(META_PREFIX)?;
    let mut data = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    aes128_ecb_decrypt(META_KEY, &mut data).ok()?;
    let json = data.strip_prefix(MUSIC_PREFIX)?;
    serde_json::from_slice(json).ok()
}

/// 解码一个 NCM 文件到 `output_dir`，文件名沿用输入文件名，扩展名按实际格式决定。
///
/// 先写入临时文件，成功后再重命名，避免中途失败留下半截文件。
/// `on_progress(已处理字节, 总字节)` 会在解码过程中被调用。
pub fn decode_file(
    input: &Path,
    output_dir: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<DecodedFile, NcmError> {
    let file = File::open(input)?;
    let total = file.metadata()?.len();
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(NcmError::BadMagic);
    }
    r.seek(SeekFrom::Current(2))?;

    // 音频密钥
    let key_len = read_u32(&mut r)?;
    let mut key = read_vec(&mut r, key_len)?;
    key.iter_mut().for_each(|b| *b ^= 0x64);
    aes128_ecb_decrypt(CORE_KEY, &mut key)?;
    let key = key
        .strip_prefix(KEY_PREFIX)
        .filter(|k| !k.is_empty())
        .ok_or(NcmError::Corrupted("音频密钥格式错误"))?;
    let ks = KeyStream::new(key);

    // 元数据（可能为空，解析失败也不影响音频解码）
    let meta_len = read_u32(&mut r)?;
    let meta = if meta_len > 0 { parse_meta(read_vec(&mut r, meta_len)?) } else { None };

    // crc32 + 1 字节间隔
    r.seek(SeekFrom::Current(5))?;

    // 封面区：frame_len 是整块长度，cover_len 是图片实际长度，后面可能有填充
    let frame_len = read_u32(&mut r)?;
    let cover_len = read_u32(&mut r)?;
    if cover_len > frame_len && frame_len != 0 {
        return Err(NcmError::Corrupted("封面长度大于封面区长度"));
    }
    let cover = if cover_len > 0 { Some(read_vec(&mut r, cover_len)?) } else { None };
    r.seek(SeekFrom::Current(frame_len.saturating_sub(cover_len) as i64))?;

    let audio_start = r.stream_position()?;
    if audio_start >= total {
        return Err(NcmError::Corrupted("没有音频数据"));
    }

    // 先解出文件头判断格式
    let mut buf = vec![0u8; 0x10000];
    let n = r.read(&mut buf)?;
    ks.apply(0, &mut buf[..n]);
    let format = sniff_format(&buf[..n])
        .map(str::to_string)
        .or_else(|| meta.as_ref().and_then(|m| m.format.clone()))
        .unwrap_or_else(|| "mp3".into());

    std::fs::create_dir_all(output_dir)?;
    let stem = input.file_stem().ok_or(NcmError::Corrupted("文件名无效"))?;
    let output_path = output_dir.join(stem).with_extension(&format);
    let tmp_path = output_path.with_extension(format!("{format}.part"));

    let result = (|| -> Result<(), NcmError> {
        let mut w = BufWriter::new(File::create(&tmp_path)?);
        w.write_all(&buf[..n])?;
        let mut offset = n as u64;
        on_progress(audio_start + offset, total);
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ks.apply(offset, &mut buf[..n]);
            w.write_all(&buf[..n])?;
            offset += n as u64;
            on_progress(audio_start + offset, total);
        }
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    std::fs::rename(&tmp_path, &output_path)?;

    Ok(DecodedFile { output_path, meta, cover })
}

/// 把 NCM 里的标题/歌手/专辑/封面写入输出文件的标签（尽力而为）。
pub fn write_tags(decoded: &DecodedFile) -> Result<(), lofty::error::LoftyError> {
    use lofty::file::TaggedFileExt;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, Tag, TagExt};

    let mut tagged = Probe::open(&decoded.output_path)?.read()?;
    let tag_type = tagged.primary_tag_type();
    if tagged.tag(tag_type).is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged.tag_mut(tag_type).unwrap();

    if let Some(meta) = &decoded.meta {
        if let Some(t) = &meta.music_name {
            tag.set_title(t.clone());
        }
        if let Some(a) = meta.artists() {
            tag.set_artist(a);
        }
        if let Some(a) = &meta.album {
            tag.set_album(a.clone());
        }
    }
    if let Some(cover) = &decoded.cover {
        let mime = if cover.starts_with(b"\x89PNG") { MimeType::Png } else { MimeType::Jpeg };
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, cover.clone()));
    }
    tag.save_to_path(&decoded.output_path, lofty::config::WriteOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff() {
        assert_eq!(sniff_format(b"fLaC\0\0"), Some("flac"));
        assert_eq!(sniff_format(b"ID3\x04"), Some("mp3"));
        assert_eq!(sniff_format(&[0xff, 0xfb, 0x90]), Some("mp3"));
        assert_eq!(sniff_format(b"\0\0\0\x20ftypM4A "), Some("m4a"));
    }

    #[test]
    fn keystream_is_position_based() {
        let ks = KeyStream::new(b"0123456789");
        let mut whole = vec![0u8; 1000];
        ks.apply(0, &mut whole);
        let mut split = vec![0u8; 1000];
        let (a, b) = split.split_at_mut(333);
        ks.apply(0, a);
        ks.apply(333, b);
        assert_eq!(whole, split);
    }
}

#[cfg(test)]
mod real_files {
    /// 用真实 NCM 文件做回归测试：`NCM_DIR=... OUT_DIR_TEST=... cargo test -- --ignored`
    #[test]
    #[ignore]
    fn decode_dir() {
        let dir = std::env::var("NCM_DIR").expect("NCM_DIR");
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR_TEST").expect("OUT_DIR_TEST"));
        let mut failed = 0;
        for e in walkdir::WalkDir::new(dir).into_iter().flatten() {
            if e.path().extension().map_or(true, |x| x != "ncm") {
                continue;
            }
            match super::decode_file(e.path(), &out, |_, _| {}) {
                Ok(d) => {
                    if let Err(err) = super::write_tags(&d) {
                        println!("TAG FAIL {:?}: {}", d.output_path, err);
                    }
                }
                Err(err) => {
                    failed += 1;
                    println!("FAIL {:?}: {}", e.path(), err);
                }
            }
        }
        assert_eq!(failed, 0);
    }
}
