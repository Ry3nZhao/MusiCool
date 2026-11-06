//! Tauri 侧的解码入口：在 blocking 线程里调用纯 Rust 解码器，并实时更新进度。

use crate::importer::ThreadProgress;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::ncm;

#[derive(Debug, Clone, Serialize)]
pub struct DecodeResult {
    pub success: bool,
    /// 成功时输出的音频文件
    pub output: Option<PathBuf>,
}

/// 在 NCM 所在目录及其上几级里找网易云的 `meta/track-<id>.jpg` 封面。
fn find_meta_cover(input: &Path, music_id: i64) -> Option<Vec<u8>> {
    input
        .ancestors()
        .skip(1)
        .take(4)
        .map(|dir| dir.join("meta").join(format!("track-{music_id}.jpg")))
        .find(|p| p.is_file())
        .and_then(|p| std::fs::read(p).ok())
}

/// 解码单个 NCM 文件到目标目录。
pub async fn decode_ncm_to_dir<P: AsRef<Path>>(
    input: P,
    output_dir: P,
    progress: Arc<Mutex<ThreadProgress>>,
) -> DecodeResult {
    let input = input.as_ref().to_path_buf();
    let output_dir = output_dir.as_ref().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let res = ncm::decode_file(&input, &output_dir, |done, total| {
            if let Ok(mut prog) = progress.lock() {
                prog.processed_size = done;
                prog.progress = if total > 0 { done as f32 / total as f32 } else { 1.0 };
            }
        });
        match res {
            Ok(decoded) => {
                let cover = decoded.meta.as_ref().and_then(|m| m.music_id()).and_then(|id| find_meta_cover(&input, id));
                if let Err(e) = ncm::write_tags(&decoded, cover) {
                    eprintln!("写入标签失败 {:?}: {}", decoded.output_path, e);
                }
                Some(decoded.output_path)
            }
            Err(e) => {
                eprintln!("解码失败 {:?}: {}", input, e);
                if let Ok(mut prog) = progress.lock() {
                    prog.progress = 0.0;
                }
                None
            }
        }
    })
    .await;

    match result {
        Ok(output) => DecodeResult { success: output.is_some(), output },
        Err(e) => {
            eprintln!("解码任务执行失败: {}", e);
            DecodeResult { success: false, output: None }
        }
    }
}
