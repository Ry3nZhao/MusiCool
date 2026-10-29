use super::ImportParams;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// 单个文件同步任务
#[derive(Debug, Clone, Serialize)]
pub struct FileTask {
    pub input: PathBuf,
    pub output: PathBuf,
    pub is_ncm: bool,
}

/// 同步计划结果，包含任务列表和统计信息
#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub tasks: Vec<FileTask>,
    pub skipped_files: usize,
    #[allow(dead_code)]
    pub total_scanned: usize,
}

fn is_excluded(rel: &Path, exclude_dirs: &[String]) -> bool {
    let rel_str = rel.to_string_lossy();
    exclude_dirs.iter().any(|pat| !pat.is_empty() && rel_str.starts_with(pat))
}

/// 计算需要同步的任务列表（只新增/更新，不删除多余文件）
/// `plan_sync` 的异步版本：目录遍历是同步 I/O，放到 blocking 线程，避免占住 tokio 工作线程。
pub async fn plan_sync_async(params: ImportParams) -> Result<SyncPlan, String> {
    tokio::task::spawn_blocking(move || plan_sync(&params))
        .await
        .map_err(|e| format!("扫描任务执行失败: {}", e))?
}

pub fn plan_sync(params: &ImportParams) -> Result<SyncPlan, String> {
    let input_root = Path::new(&params.input_path);
    let output_root = Path::new(&params.output_path);

    if !input_root.is_dir() {
        return Err(format!("输入路径不是有效目录: {}", params.input_path));
    }
    if !output_root.exists() {
        fs::create_dir_all(output_root).map_err(|e| format!("创建输出目录失败: {e}"))?;
    }

    let mut tasks = Vec::new();
    let mut skipped_files = 0;
    let mut total_scanned = 0;

    for entry in WalkDir::new(input_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = match path.strip_prefix(input_root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        total_scanned += 1;

        if is_excluded(rel, &params.exclude_dirs) {
            continue;
        }

        let is_ncm = path.extension().map(|e| e.to_ascii_lowercase())
            .and_then(|e| e.to_str().map(|s| s == "ncm")).unwrap_or(false);

        // 对于 NCM 文件，输出路径是解码后的音频文件（文件名由解码器从元数据确定）
        // 对于普通文件，输出路径保持相同的相对路径结构
        let out_path = if is_ncm {
            // NCM 文件：输出目录保持相对路径结构，但文件名会在解码时确定
            // 这里先使用原路径作为占位，实际解码后的文件路径由解码器决定
            output_root.join(rel)
        } else {
            // 普通文件：直接映射到输出目录
            output_root.join(rel)
        };

        // rsync 风格比对：只新增/更新，不删除
        if !is_ncm {
            // 对于普通文件：如果目标存在且大小 & mtime 相同，则跳过
            if out_path.exists() {
                if let (Ok(src_meta), Ok(dst_meta)) = (fs::metadata(path), fs::metadata(&out_path)) {
                    let same_size = src_meta.len() == dst_meta.len();
                    // 比较修改时间，允许1秒的误差（某些文件系统时间精度问题）
                    let same_mtime = src_meta.modified()
                        .ok()
                        .and_then(|t1| {
                            dst_meta.modified().ok().map(|t2| {
                                let diff = t1.duration_since(t2).unwrap_or_default()
                                    .max(t2.duration_since(t1).unwrap_or_default());
                                diff.as_secs() <= 1 // 允许1秒误差
                            })
                        })
                        .unwrap_or(false);
                    if same_size && same_mtime {
                        skipped_files += 1;
                        continue; // 文件未变化，跳过
                    }
                }
            }
        } else {
            // 对于 NCM 文件：尝试预测输出文件名并比较
            // 输出文件名格式：原文件名（不含扩展名）+ 格式扩展名（如.mp3）
            // 解码器会在输出目录下创建文件，文件名是 stem + extname
            // 由于需要解析NCM元数据才能确定格式，这里先检查常见的输出格式
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let common_extensions = ["mp3", "flac", "m4a", "ogg", "wav"];
            let mut should_skip = false;
            
            // 输出目录是相对于输出根目录的，与输入文件的相对路径相同
            let output_dir = output_root.join(rel.parent().unwrap_or(rel));
            
            for ext in &common_extensions {
                let predicted_output = output_dir.join(format!("{}.{}", stem, ext));
                
                if predicted_output.exists() {
                    // 检查NCM文件的修改时间和预测输出文件的修改时间
                    if let (Ok(ncm_meta), Ok(output_meta)) = (fs::metadata(path), fs::metadata(&predicted_output)) {
                        let ncm_mtime = ncm_meta.modified().ok();
                        let output_mtime = output_meta.modified().ok();
                        
                        // 如果输出文件比NCM文件新或相同（允许1秒误差），则认为已解码
                        if let (Some(ncm_time), Some(out_time)) = (ncm_mtime, output_mtime) {
                            if out_time >= ncm_time || 
                               ncm_time.duration_since(out_time).map(|d| d.as_secs() <= 1).unwrap_or(false) {
                                should_skip = true;
                                break;
                            }
                        }
                    }
                }
            }
            
            if should_skip {
                skipped_files += 1;
                continue; // NCM文件已解码且未变化，跳过
            }
        }

        tasks.push(FileTask {
            input: path.to_path_buf(),
            output: out_path,
            is_ncm,
        });
    }

    Ok(SyncPlan {
        tasks,
        skipped_files,
        total_scanned,
    })
}



