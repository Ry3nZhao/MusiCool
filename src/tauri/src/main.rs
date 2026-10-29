#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]



mod importer;

mod decoder;



use importer::ImportStatus;

use serde::{Deserialize, Serialize};

use std::path::PathBuf;

use tauri::{State, Manager};

use tokio::fs;

use tokio::sync::RwLock;

use uuid::Uuid;



/// 项目配置（输入路径 + 排除规则）

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct Project {

    pub id: String,  // UUID

    pub name: String,  // 项目名称（可编辑）

    pub input_path: String,  // 输入路径

    pub exclude_patterns: Vec<String>,  // 排除规则列表

}

/// 应用设置

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct AppSettings {

    pub library_path: Option<String>,

    /// 默认最大线程数（用于导入任务）

    #[serde(default = "default_max_threads")]

    pub max_threads: usize,

    /// 全局默认排除规则（相对路径前缀列表）

    #[serde(default)]

    pub exclude_patterns: Vec<String>,

    /// 导入页面的默认输入路径（临时保存，方便下次使用）

    #[serde(default)]

    pub last_input_path: Option<String>,

    /// 导入页面的默认输出路径（临时保存，方便下次使用）

    #[serde(default)]

    pub last_output_path: Option<String>,

    /// 项目列表

    #[serde(default)]

    pub projects: Vec<Project>,

}



impl Default for AppSettings {

    fn default() -> Self {

        Self {

            library_path: None,

            max_threads: default_max_threads(),

            exclude_patterns: Vec::new(),

            last_input_path: None,

            last_output_path: None,

            projects: Vec::new(),

        }

    }

}



fn default_max_threads() -> usize {

    // 默认使用 CPU 核心数，但至少为 1

    std::thread::available_parallelism()

        .map(|n| n.get())

        .unwrap_or(4)

        .max(1)

}



/// 全局应用状态

pub struct AppState {

    pub settings: RwLock<AppSettings>,

    pub import_manager: importer::ImportTaskManager,

    pub config_path: PathBuf,

}



impl AppState {

    pub async fn new(app_handle: tauri::AppHandle) -> Result<Self, String> {

        // 开发版：使用项目根目录的 config.json（与 Python 版本保持一致）
        // 优先使用当前工作目录，如果不存在则尝试从可执行文件路径推断
        // 发布版：存到系统的应用配置目录（如 macOS 的 ~/Library/Application Support/com.musiccool.app）。
        // 从 Finder 启动时工作目录是 "/"，按工作目录推断会得到不可写的 /config.json，导致保存失败。
        let release_config = if cfg!(debug_assertions) {
            None
        } else {
            tauri::api::path::app_config_dir(&app_handle.config()).map(|d| d.join("config.json"))
        };

        let config_path: PathBuf = if let Some(p) = release_config {
            p
        } else if let Ok(cwd) = std::env::current_dir() {
            // 检查当前工作目录是否是项目根目录
            if cwd.join("src/tauri").exists() || cwd.join("config.json").exists() {
                cwd.join("config.json")
            } else {
                // 尝试从当前目录向上查找项目根目录
                let mut test_path = cwd.clone();
                let mut found = false;
                let mut result_path = cwd.join("config.json");
                for _ in 0..5 {
                    if test_path.join("src/tauri").exists() || test_path.join("config.json").exists() {
                        result_path = test_path.join("config.json");
                        found = true;
                        break;
                    }
                    if !test_path.pop() {
                        break;
                    }
                }
                if !found {
                    // 如果找不到，使用当前目录
                    cwd.join("config.json")
                } else {
                    result_path
                }
            }
        } else {
            // 如果无法获取当前目录，尝试从可执行文件路径推断
            let exe_path = std::env::current_exe()
                .map_err(|e| format!("无法获取可执行文件路径: {}", e))?;
            let mut test_path = exe_path.clone();
            let mut found = false;
            let mut result_path = exe_path.parent().unwrap().join("config.json");
            for _ in 0..5 {
                test_path.pop();
                if test_path.join("src/tauri").exists() || test_path.join("config.json").exists() {
                    result_path = test_path.join("config.json");
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("无法找到项目根目录".to_string());
            }
            result_path
        };



        // 尝试加载现有设置

        let settings = match fs::read_to_string(&config_path).await {

            Ok(content) => {

                serde_json::from_str::<AppSettings>(&content)

                    .unwrap_or_default()

            }

            Err(_) => AppSettings::default(),

        };



        Ok(Self {

            settings: RwLock::new(settings),

            import_manager: importer::ImportTaskManager::with_app_handle(app_handle),

            config_path,

        })

    }



    async fn save_settings(&self) -> Result<(), String> {

        let settings = self.settings.read().await;

        let json = serde_json::to_string_pretty(&*settings)

            .map_err(|e| format!("序列化设置失败: {}", e))?;

        

        // 确保目录存在

        if let Some(parent) = self.config_path.parent() {

            fs::create_dir_all(parent).await

                .map_err(|e| format!("创建配置目录失败: {}", e))?;

        }



        fs::write(&self.config_path, json).await

            .map_err(|e| format!("保存设置失败: {}", e))?;

        

        Ok(())

    }

}



/// 从配置中读取设置

#[tauri::command]

async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {

    let settings = state.settings.read().await;

    Ok(settings.clone())

}



/// 更新音乐库路径

#[tauri::command]

async fn set_library_path(path: String, state: State<'_, AppState>) -> Result<(), String> {

    {

        let mut settings = state.settings.write().await;

        settings.library_path = Some(path);

    }

    // 持久化到文件

    state.save_settings().await?;

    Ok(())

}



/// 更新设置（支持部分更新）

#[tauri::command]

async fn update_settings(

    settings_update: AppSettings,

    state: State<'_, AppState>,

) -> Result<(), String> {

    {

        let mut settings = state.settings.write().await;

        // 部分更新：只更新提供的字段

        if let Some(lib_path) = settings_update.library_path {

            settings.library_path = Some(lib_path);

        }

        if settings_update.max_threads > 0 {

            settings.max_threads = settings_update.max_threads;

        }

        if !settings_update.exclude_patterns.is_empty() {

            settings.exclude_patterns = settings_update.exclude_patterns;

        }

        // 更新导入路径（如果提供）

        if let Some(input_path) = settings_update.last_input_path {

            settings.last_input_path = Some(input_path);

        }

        if let Some(output_path) = settings_update.last_output_path {

            settings.last_output_path = Some(output_path);

        }

    }

    // 持久化到文件

    state.save_settings().await?;

    Ok(())

}



/// 设置最大线程数

#[tauri::command]

async fn set_max_threads(threads: usize, state: State<'_, AppState>) -> Result<(), String> {

    if threads == 0 {

        return Err("线程数必须大于 0".to_string());

    }

    {

        let mut settings = state.settings.write().await;

        settings.max_threads = threads;

    }

    state.save_settings().await?;

    Ok(())

}



/// 设置排除规则

#[tauri::command]

async fn set_exclude_patterns(

    patterns: Vec<String>,

    state: State<'_, AppState>,

) -> Result<(), String> {

    {

        let mut settings = state.settings.write().await;

        settings.exclude_patterns = patterns;

    }

    state.save_settings().await?;

    Ok(())

}

/// 获取所有项目

#[tauri::command]

async fn get_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {

    let settings = state.settings.read().await;

    Ok(settings.projects.clone())

}

/// 创建项目

#[tauri::command]

async fn create_project(

    name: String,

    input_path: String,

    exclude_patterns: Vec<String>,

    state: State<'_, AppState>,

) -> Result<Project, String> {

    let project = Project {

        id: Uuid::new_v4().to_string(),

        name,

        input_path,

        exclude_patterns,

    };

    {

        let mut settings = state.settings.write().await;

        settings.projects.push(project.clone());

    }

    state.save_settings().await?;

    Ok(project)

}

/// 删除项目

#[tauri::command]

async fn delete_project(

    project_id: String,

    state: State<'_, AppState>,

) -> Result<(), String> {

    {

        let mut settings = state.settings.write().await;

        settings.projects.retain(|p| p.id != project_id);

    }

    state.save_settings().await?;

    Ok(())

}

/// 更新项目

#[tauri::command]

async fn update_project(

    project_id: String,

    name: Option<String>,

    input_path: Option<String>,

    exclude_patterns: Option<Vec<String>>,

    state: State<'_, AppState>,

) -> Result<(), String> {

    {

        let mut settings = state.settings.write().await;

        if let Some(project) = settings.projects.iter_mut().find(|p| p.id == project_id) {

            if let Some(n) = name {

                project.name = n;

            }

            if let Some(path) = input_path {

                project.input_path = path;

            }

            if let Some(patterns) = exclude_patterns {

                project.exclude_patterns = patterns;

            }

        } else {

            return Err(format!("项目不存在: {}", project_id));

        }

    }

    state.save_settings().await?;

    Ok(())

}



/// 文件信息结构（用于音乐库列表）

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct FileInfo {

    pub path: String,

    pub name: String,

    pub size: u64,

    pub artist: Option<String>,

    pub album: Option<String>,

    pub duration: Option<u64>,  // 持续时间（秒）

}



/// 提取音频文件元数据（歌手、专辑、持续时间）

fn extract_audio_metadata(path: &std::path::Path) -> (Option<String>, Option<String>, Option<u64>) {

    use lofty::{read_from_path, prelude::TaggedFileExt};

    use lofty::file::AudioFile;

    use lofty::tag::{Tag, Accessor};

    use std::borrow::Cow;

    match read_from_path(path) {

        Ok(tagged_file) => {

            let tag = tagged_file.primary_tag();

            let artist = tag

                .and_then(|t: &Tag| t.artist())

                .map(|s: Cow<'_, str>| s.to_string());

            let album = tag

                .and_then(|t: &Tag| t.album())

                .map(|s: Cow<'_, str>| s.to_string());

            let duration = tagged_file.properties().duration().as_secs();

            (artist, album, Some(duration))

        }

        Err(_) => (None, None, None),

    }

}



/// 列出音乐库中的文件

#[tauri::command]

async fn list_library_files(state: State<'_, AppState>) -> Result<Vec<FileInfo>, String> {

    use walkdir::WalkDir;



    // 只在读取路径时持有锁，扫描期间不阻塞其他命令
    let Some(library_path) = state.settings.read().await.library_path.clone() else {
        return Ok(Vec::new());
    };

    let root = PathBuf::from(&library_path);
    if !root.is_dir() {
        return Err(format!("音乐库路径不是有效目录: {}", library_path));
    }

    // 目录遍历和标签读取都是同步 I/O，放到 blocking 线程，避免占住 tokio 工作线程
    let result = tokio::task::spawn_blocking(move || {
        let mut result = Vec::new();
        for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            // 尝试提取音频元数据
            let (artist, album, duration) = extract_audio_metadata(path);
            result.push(FileInfo {
                path: path.to_string_lossy().to_string(),
                name: entry.file_name().to_string_lossy().to_string(),
                size: metadata.len(),
                artist,
                album,
                duration,
            });
        }
        result
    })
    .await
    .map_err(|e| format!("扫描音乐库失败: {}", e))?;

    Ok(result)

}



/// 启动导入任务（支持多项目串行导入）

#[tauri::command]

async fn start_import_task(

    project_ids: Vec<String>,

    thread_count: usize,

    state: State<'_, AppState>,

) -> Result<String, String> {

    // 获取选中的项目和音乐库路径

    let (selected_projects, output_path) = {

        let settings = state.settings.read().await;

        let projects: Vec<_> = settings.projects.iter()

            .filter(|p| project_ids.contains(&p.id))

            .cloned()

            .collect();

        let output = settings.library_path.clone()

            .ok_or("请先在设置中配置音乐库路径")?;

        (projects, output)

    };

    if selected_projects.is_empty() {

        return Err("请至少选择一个项目".to_string());

    }

    // 创建任务并串行处理每个项目

    let task_id = state.import_manager.start_multi_project_task(

        selected_projects,

        output_path,

        thread_count,

    ).await?;

    Ok(task_id)

}



/// 查询导入任务状态

#[tauri::command]

async fn get_import_status(task_id: String, state: State<'_, AppState>) -> Result<ImportStatus, String> {

    match state.import_manager.get_status(&task_id).await {

        Some(status) => Ok(status),

        None => Err(format!("任务不存在: {}", task_id)),

    }

}



fn main() {

    tauri::Builder::default()

        .setup(|app| {

            let app_handle = app.handle().clone();

            let state = tauri::async_runtime::block_on(AppState::new(app_handle))

                .expect("初始化应用状态失败");

            app.manage(state);

            Ok(())

        })

        .invoke_handler(tauri::generate_handler![

            get_settings,

            set_library_path,

            update_settings,

            set_max_threads,

            set_exclude_patterns,

            get_projects,

            create_project,

            delete_project,

            update_project,

            list_library_files,

            start_import_task,

            get_import_status

        ])

        .run(tauri::generate_context!())

        .expect("error while running Music_Cool Tauri application");

}


