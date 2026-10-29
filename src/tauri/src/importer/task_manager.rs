use serde::{Deserialize, Serialize};

use std::collections::HashMap;

use std::sync::atomic::{AtomicUsize, Ordering};

use std::sync::{Arc, Mutex};

use tokio::fs;

use tokio::sync::RwLock;

use tokio::task::JoinSet;

use uuid::Uuid;

use tauri::Manager;



use super::rsync_like;

use crate::decoder;

use crate::Project;



/// 导入参数（与前端交互用）

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct ImportParams {

    pub input_path: String,

    pub output_path: String,

    pub exclude_dirs: Vec<String>,

    pub thread_count: usize,

}



/// 单个“线程”（工作单元）的进度

#[derive(Debug, Clone, Serialize, Deserialize, Default)]

pub struct ThreadProgress {

    pub thread_id: usize,

    pub current_file: Option<String>,

    pub current_size: u64,

    pub processed_size: u64,

    pub progress: f32,

}



/// 项目进度信息

#[derive(Debug, Clone, Serialize, Deserialize, Default)]

pub struct ProjectProgress {

    pub project_id: String,

    pub project_name: String,

    pub progress: f32,

    pub status: String,  // "waiting", "processing", "completed", "failed"

    pub total_files: usize,

    pub processed_files: usize,

}

/// 导入任务状态（对前端暴露）

#[derive(Debug, Clone, Serialize, Deserialize, Default)]

pub struct ImportStatus {

    pub task_id: String,

    pub stage: String,

    pub progress: f32,

    pub threads: Vec<ThreadProgress>,

    /// 当前正在处理的项目名称

    #[serde(default)]

    pub current_project: Option<String>,

    /// 每个项目的进度

    #[serde(default)]

    pub project_progress: Vec<ProjectProgress>,

    /// 详细阶段信息（如 "复制中 (3/5)"）

    #[serde(default)]

    pub current_stage_detail: String,

    /// 总文件数

    #[serde(default)]

    pub total_files: usize,

    /// 已处理文件数

    #[serde(default)]

    pub processed_files: usize,

    /// 跳过的文件数（已匹配，无需处理）

    #[serde(default)]

    pub skipped_files: usize,

    /// 新处理的文件数（新匹配的）

    #[serde(default)]

    pub new_files: usize,

}



/// 内部存储结构

#[derive(Debug, Default)]

struct ImportTaskInternal {

    pub status: ImportStatus,

}



/// 导入任务管理器

#[derive(Clone)]

pub struct ImportTaskManager {

    inner: Arc<RwLock<HashMap<String, ImportTaskInternal>>>,

    app_handle: Option<tauri::AppHandle>,

}



impl Default for ImportTaskManager {

    fn default() -> Self {

        Self {

            inner: Arc::new(RwLock::new(HashMap::new())),

            app_handle: None,

        }

    }

}



impl ImportTaskManager {

    pub fn new() -> Self {

        Self::default()

    }



    pub fn with_app_handle(app_handle: tauri::AppHandle) -> Self {

        Self {

            inner: Arc::new(RwLock::new(HashMap::new())),

            app_handle: Some(app_handle),

        }

    }



    fn emit_progress(&self, _task_id: &str, status: &ImportStatus) {
        if let Some(handle) = &self.app_handle {
            // 在 Tauri v1 中，使用 emit_all 向所有窗口发送事件
            let _ = handle.emit_all("import-progress", status.clone());
        }
    }



    /// 启动一个新的导入任务：

    /// - 先进行目录扫描和 rsync 风格比对，生成需要处理的文件列表

    /// - 后台异步执行任务，并更新进度

    pub async fn start_task(&self, params: ImportParams) -> Result<String, String> {

        let task_id = Uuid::new_v4().to_string();



        let sync_plan = rsync_like::plan_sync_async(params.clone()).await?;

        let tasks = sync_plan.tasks;
        let skipped_files = sync_plan.skipped_files;
        let new_files = tasks.len();
        let total_files = tasks.len().max(1) as f32;

        let total_files_usize = tasks.len();



        // 初始化状态

        {

            let mut map = self.inner.write().await;

            map.insert(

                task_id.clone(),

                ImportTaskInternal {

                    status: ImportStatus {

                        task_id: task_id.clone(),

                        stage: "scanning".into(),

                        progress: 0.0,

                        threads: (0..params.thread_count)

                            .map(|i| ThreadProgress {

                                thread_id: i,

                                ..Default::default()

                            })

                            .collect(),

                        current_project: None,

                        project_progress: Vec::new(),

                        current_stage_detail: format!("扫描中 (0/{})", total_files_usize),

                        total_files: total_files_usize,

                        processed_files: 0,

                        skipped_files,

                        new_files,

                    },

                },

            );

        }



        // 克隆管理器用于异步任务
        let manager = self.clone();
        let processed_count = Arc::new(AtomicUsize::new(0));

        // 克隆 task_id 供异步任务使用，避免移动原始值
        let task_id_for_spawn = task_id.clone();

        tokio::spawn(async move {
            let task_id = task_id_for_spawn;

            // 使用 tokio 的 JoinSet 来管理多个并发任务

            let mut join_set = JoinSet::new();

            let tasks_arc = Arc::new(tasks);

            let thread_count = params.thread_count;



            // 为每个线程创建一个工作循环

            for thread_id in 0..thread_count {

                let tasks_clone = tasks_arc.clone();

                let manager_clone = manager.clone();

                let task_id_clone = task_id.clone();

                let total_files = total_files;

                let processed_count_clone = processed_count.clone();



                join_set.spawn(async move {

                    let mut idx = thread_id;



                    while idx < tasks_clone.len() {

                        let file_task = &tasks_clone[idx];



                        // 获取文件大小（在处理前）

                        let file_size = fs::metadata(&file_task.input).await.map(|m| m.len()).unwrap_or(0);



                        // 更新线程状态（开始处理）

                        {

                            let mut map = manager_clone.inner.write().await;

                            if let Some(task) = map.get_mut(&task_id_clone) {

                                task.status.stage = "copying".into();



                                if thread_id < task.status.threads.len() {

                                    let th = &mut task.status.threads[thread_id];

                                    th.thread_id = thread_id;

                                    th.current_file = Some(file_task.input.to_string_lossy().to_string());

                                    th.current_size = file_size;

                                    th.processed_size = 0;

                                    th.progress = 0.0;

                                }

                            }

                        }



                        // 处理文件：NCM 解码或普通文件复制

                        let result = if file_task.is_ncm {

                            // NCM 文件：调用解码器

                            let output_dir = file_task.output.parent().unwrap().to_path_buf();

                            let progress = Arc::new(Mutex::new(ThreadProgress {

                                thread_id,

                                current_file: Some(file_task.input.to_string_lossy().to_string()),

                                current_size: file_size,

                                ..Default::default()

                            }));

                            let decode_result = decoder::decode_ncm_to_dir(&file_task.input, &output_dir, progress.clone()).await;
                            
                            decode_result

                        } else {

                            // 普通文件：直接复制

                            copy_file(&file_task.input, &file_task.output).await

                        };



                        // 更新线程状态和总进度（处理完成）

                        {

                            let mut map = manager_clone.inner.write().await;

                            if let Some(task) = map.get_mut(&task_id_clone) {

                                if thread_id < task.status.threads.len() {

                                    let th = &mut task.status.threads[thread_id];

                                    if result.success {

                                        th.processed_size = file_size;

                                        th.progress = 1.0;

                                        // 增加全局计数器

                                        let processed = processed_count_clone.fetch_add(1, Ordering::Relaxed) + 1;

                                        // 计算总进度

                                        task.status.progress = (processed as f32 / total_files).min(1.0);

                                        // 更新详细进度信息

                                        task.status.processed_files = processed;

                                        task.status.total_files = total_files as usize;

                                        task.status.current_stage_detail = format!("{} ({}/{})", task.status.stage, processed, total_files as usize);

                                    } else {

                                        th.progress = 0.0;

                                        th.processed_size = 0;

                                    }

                                }

                            }

                        }



                        // 发送进度事件

                        {

                            let map = manager_clone.inner.read().await;

                            if let Some(task) = map.get(&task_id_clone) {

                                manager_clone.emit_progress(&task_id_clone, &task.status);

                            }

                        }



                        // 跳转到下一个分配给此线程的任务

                        idx += thread_count;

                    }



                    0 // 返回值不再使用

                });

            }



            // 等待所有线程完成

            while join_set.join_next().await.is_some() {}



            // 标记完成

            let mut map = manager.inner.write().await;

            if let Some(task) = map.get_mut(&task_id) {

                task.status.stage = "finished".into();

                task.status.progress = 1.0;

                for th in &mut task.status.threads {

                    th.progress = 1.0;

                    th.current_file = None;

                }

                manager.emit_progress(&task_id, &task.status);

            }

        });



        Ok(task_id)

    }



    /// 获取任务状态

    pub async fn get_status(&self, task_id: &str) -> Option<ImportStatus> {

        let map = self.inner.read().await;

        map.get(task_id).map(|t| t.status.clone())

    }

    /// 启动多项目串行导入任务

    pub async fn start_multi_project_task(

        &self,

        projects: Vec<Project>,

        output_path: String,

        thread_count: usize,

    ) -> Result<String, String> {

        let task_id = Uuid::new_v4().to_string();

        // 初始化项目进度列表

        let project_progress: Vec<ProjectProgress> = projects.iter().map(|p| {

            ProjectProgress {

                project_id: p.id.clone(),

                project_name: p.name.clone(),

                progress: 0.0,

                status: "waiting".into(),

                total_files: 0,

                processed_files: 0,

            }

        }).collect();

        // 初始化状态

        {

            let mut map = self.inner.write().await;

            map.insert(

                task_id.clone(),

                ImportTaskInternal {

                    status: ImportStatus {

                        task_id: task_id.clone(),

                        stage: "scanning".into(),

                        progress: 0.0,

                        threads: (0..thread_count)

                            .map(|i| ThreadProgress {

                                thread_id: i,

                                ..Default::default()

                            })

                            .collect(),

                        current_project: None,

                        project_progress: project_progress.clone(),

                        current_stage_detail: "准备中".into(),

                        total_files: 0,

                        processed_files: 0,

                        skipped_files: 0,

                        new_files: 0,

                    },

                },

            );

        }

        // 克隆管理器用于异步任务

        let manager = self.clone();

        let task_id_for_spawn = task_id.clone();

        tokio::spawn(async move {

            let task_id = task_id_for_spawn;

            let total_processed = Arc::new(AtomicUsize::new(0));

            let total_files = Arc::new(AtomicUsize::new(0));

            // 串行处理每个项目

            for (project_idx, project) in projects.iter().enumerate() {

                // 更新当前项目

                {

                    let mut map = manager.inner.write().await;

                    if let Some(task) = map.get_mut(&task_id) {

                        task.status.current_project = Some(project.name.clone());

                        if let Some(proj_progress) = task.status.project_progress.get_mut(project_idx) {

                            proj_progress.status = "processing".into();

                        }

                        task.status.current_stage_detail = format!("处理项目: {}", project.name);

                    }

                }

                manager.emit_progress(&task_id, &manager.get_status(&task_id).await.unwrap_or_default());

                // 为当前项目创建 ImportParams

                let params = ImportParams {

                    input_path: project.input_path.clone(),

                    output_path: output_path.clone(),

                    exclude_dirs: project.exclude_patterns.clone(),

                    thread_count,

                };

                // 扫描文件

                let sync_plan = match rsync_like::plan_sync_async(params.clone()).await {

                    Ok(p) => p,

                    Err(e) => {

                        // 标记项目失败

                        {

                            let mut map = manager.inner.write().await;

                            if let Some(task) = map.get_mut(&task_id) {

                                if let Some(proj_progress) = task.status.project_progress.get_mut(project_idx) {

                                    proj_progress.status = "failed".into();

                                }

                            }

                        }

                        eprintln!("项目 {} 扫描失败: {}", project.name, e);

                        continue;

                    }

                };

                let tasks = sync_plan.tasks;
                let project_skipped = sync_plan.skipped_files;
                let project_total_files = tasks.len();

                // 更新统计信息
                {
                    let mut map = manager.inner.write().await;
                    if let Some(task) = map.get_mut(&task_id) {
                        task.status.skipped_files += project_skipped;
                        task.status.new_files += project_total_files;
                    }
                }

                let current_total = total_files.fetch_add(project_total_files, Ordering::Relaxed) + project_total_files;

                // 更新项目总文件数

                {

                    let mut map = manager.inner.write().await;

                    if let Some(task) = map.get_mut(&task_id) {

                        task.status.total_files = current_total;

                        if let Some(proj_progress) = task.status.project_progress.get_mut(project_idx) {

                            proj_progress.total_files = project_total_files;

                        }

                    }

                }

                // 处理当前项目的所有文件（复用现有的单项目处理逻辑）

                let tasks_arc = Arc::new(tasks);

                let processed_count = Arc::new(AtomicUsize::new(0));

                let total_processed_clone = total_processed.clone();

                let total_files_clone = total_files.clone();

                let mut join_set = JoinSet::new();

                for thread_id in 0..thread_count {

                    let tasks_clone = tasks_arc.clone();

                    let manager_clone = manager.clone();

                    let task_id_clone = task_id.clone();

                    let processed_count_clone = processed_count.clone();

                    let total_processed_clone_inner = total_processed_clone.clone();

                    let total_files_clone_inner = total_files_clone.clone();

                    let project_idx_clone = project_idx;

                    let project_total_files_f32 = project_total_files as f32;

                    join_set.spawn(async move {

                        let mut idx = thread_id;

                        while idx < tasks_clone.len() {

                            let file_task = &tasks_clone[idx];

                            let file_size = fs::metadata(&file_task.input).await.map(|m| m.len()).unwrap_or(0);

                            // 更新线程状态

                            {

                                let mut map = manager_clone.inner.write().await;

                                if let Some(task) = map.get_mut(&task_id_clone) {

                                    task.status.stage = "copying".into();

                                    if thread_id < task.status.threads.len() {

                                        let th = &mut task.status.threads[thread_id];

                                        th.current_file = Some(file_task.input.to_string_lossy().to_string());

                                        th.current_size = file_size;

                                        th.processed_size = 0;

                                        th.progress = 0.0;

                                    }

                                }

                            }

                            // 处理文件

                            let result = if file_task.is_ncm {

                                let output_dir = file_task.output.parent().unwrap().to_path_buf();

                                let progress = Arc::new(Mutex::new(ThreadProgress {

                                    thread_id,

                                    current_file: Some(file_task.input.to_string_lossy().to_string()),

                                    current_size: file_size,

                                    ..Default::default()

                                }));

                                decoder::decode_ncm_to_dir(&file_task.input, &output_dir, progress.clone()).await

                            } else {

                                copy_file(&file_task.input, &file_task.output).await

                            };

                            // 更新进度

                            {

                                let mut map = manager_clone.inner.write().await;

                                if let Some(task) = map.get_mut(&task_id_clone) {

                                    if result.success {

                                        let processed = processed_count_clone.fetch_add(1, Ordering::Relaxed) + 1;

                                        let project_processed = processed;

                                        // 更新项目进度

                                        if let Some(proj_progress) = task.status.project_progress.get_mut(project_idx_clone) {

                                            proj_progress.processed_files = project_processed;

                                            proj_progress.progress = (project_processed as f32 / project_total_files_f32).min(1.0);

                                        }

                                        // 更新总进度

                                        let global_processed = total_processed_clone_inner.fetch_add(1, Ordering::Relaxed) + 1;

                                        let global_total = total_files_clone_inner.load(Ordering::Relaxed);

                                        task.status.processed_files = global_processed;

                                        task.status.progress = (global_processed as f32 / global_total.max(1) as f32).min(1.0);

                                        task.status.current_stage_detail = format!("复制中 ({}/{})", global_processed, global_total);

                                    }

                                    if thread_id < task.status.threads.len() {

                                        let th = &mut task.status.threads[thread_id];

                                        if result.success {

                                            th.processed_size = file_size;

                                            th.progress = 1.0;

                                        } else {

                                            th.progress = 0.0;

                                            th.processed_size = 0;

                                        }

                                    }

                                }

                            }

                            manager_clone.emit_progress(&task_id_clone, &manager_clone.get_status(&task_id_clone).await.unwrap_or_default());

                            idx += thread_count;

                        }

                        0

                    });

                }

                // 等待当前项目的所有线程完成

                while join_set.join_next().await.is_some() {}

                // 标记当前项目完成

                {

                    let mut map = manager.inner.write().await;

                    if let Some(task) = map.get_mut(&task_id) {

                        if let Some(proj_progress) = task.status.project_progress.get_mut(project_idx) {

                            proj_progress.status = "completed".into();

                            proj_progress.progress = 1.0;

                        }

                    }

                }

                manager.emit_progress(&task_id, &manager.get_status(&task_id).await.unwrap_or_default());

            }

            // 所有项目完成

            {

                let mut map = manager.inner.write().await;

                if let Some(task) = map.get_mut(&task_id) {

                    task.status.stage = "finished".into();

                    task.status.progress = 1.0;

                    task.status.current_stage_detail = "完成".into();

                    task.status.current_project = None;

                    for th in &mut task.status.threads {

                        th.progress = 1.0;

                        th.current_file = None;

                    }

                }

            }

            manager.emit_progress(&task_id, &manager.get_status(&task_id).await.unwrap_or_default());

        });

        Ok(task_id)

    }

}



/// 复制文件（保留元数据）

async fn copy_file(src: &std::path::Path, dst: &std::path::Path) -> decoder::DecodeResult {

    // 确保目标目录存在

    if let Some(parent) = dst.parent() {

        if let Err(e) = fs::create_dir_all(parent).await {

            eprintln!("创建目录失败: {:?}, 错误: {}", parent, e);

            return decoder::DecodeResult { success: false };

        }

    }



    // 复制文件

    match fs::copy(src, dst).await {

        Ok(_) => {

            // 尝试复制元数据（修改时间等）

            if let (Ok(src_meta), Ok(dst_meta)) = (fs::metadata(src).await, fs::metadata(dst).await) {

                if let (Ok(_src_mtime), Ok(_dst_mtime)) = (src_meta.modified(), dst_meta.modified()) {

                    // 注意：tokio::fs 不直接支持设置修改时间，这里只做基本复制

                    // 如果需要保留修改时间，可以使用 std::fs::File::set_times（需要同步操作）

                }

            }

            decoder::DecodeResult { success: true }

        }

        Err(e) => {

            eprintln!("复制文件失败: {:?} -> {:?}, 错误: {}", src, dst, e);

            decoder::DecodeResult { success: false }

        }

    }

}


