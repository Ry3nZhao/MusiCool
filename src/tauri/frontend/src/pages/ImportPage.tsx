import React, { useState, useEffect, useRef } from "react";

import { invoke } from "@tauri-apps/api/tauri";

import { listen } from "@tauri-apps/api/event";

import { open } from "@tauri-apps/api/dialog";



interface Project {

  id: string;

  name: string;

  input_path: string;

  exclude_patterns: string[];

}



interface ImportStatus {

  task_id: string;

  stage: string;

  progress: number;

  threads: Array<{

    thread_id: number;

    current_file?: string;

    current_size: number;

    processed_size: number;

    progress: number;

  }>;

  current_project?: string;

  project_progress?: Array<{

    project_id: string;

    project_name: string;

    progress: number;

    status: string;

    total_files: number;

    processed_files: number;

  }>;

  current_stage_detail?: string;

  total_files?: number;

  processed_files?: number;

  skipped_files?: number;

  new_files?: number;

}



interface Settings {

  library_path?: string;

  max_threads?: number;

  projects?: Project[];

}



export const ImportPage: React.FC = () => {

  const [projects, setProjects] = useState<Project[]>([]);

  const [selectedProjects, setSelectedProjects] = useState<Set<string>>(new Set());

  const [threadCount, setThreadCount] = useState(4);

  const [taskId, setTaskId] = useState<string | null>(null);

  const [status, setStatus] = useState<ImportStatus | null>(null);

  const [showNewProjectModal, setShowNewProjectModal] = useState(false);

  const [showDetailModal, setShowDetailModal] = useState(false);

  const [showCompletionModal, setShowCompletionModal] = useState(false);

  const [completionStats, setCompletionStats] = useState<{ skipped: number; new: number } | null>(null);

  const [newProjectName, setNewProjectName] = useState("");

  const [newProjectPath, setNewProjectPath] = useState("");

  const [newProjectExclude, setNewProjectExclude] = useState<string[]>([]);

  const intervalRef = useRef<number | null>(null);



  useEffect(() => {

    loadProjects();

    loadSettings();

  }, []);



  const loadProjects = async () => {

    try {

      const projs = await invoke<Project[]>("get_projects");

      setProjects(projs);

    } catch (err) {

      console.error("Failed to load projects:", err);

    }

  };



  const loadSettings = async () => {

    try {

      const s = await invoke<Settings>("get_settings");

      if (s.max_threads) {

        setThreadCount(s.max_threads);

      }

    } catch (err) {

      console.error("Failed to load settings:", err);

    }

  };



  useEffect(() => {

    if (!taskId) return;



    let unlisten: (() => void) | null = null;

    let cleanupInterval: (() => void) | null = null;



    (async () => {

      try {

        unlisten = await listen<ImportStatus>("import-progress", (event) => {

          if (event.payload.task_id === taskId) {

            setStatus(event.payload);

            if (event.payload.stage === "finished" || event.payload.progress >= 1.0) {

              if (intervalRef.current) {

                clearInterval(intervalRef.current);

                intervalRef.current = null;

              }

              // 显示完成弹窗

              setCompletionStats({

                skipped: event.payload.skipped_files || 0,

                new: event.payload.new_files || 0,

              });

              setShowCompletionModal(true);

            }

          }

        });



        intervalRef.current = window.setInterval(async () => {

          try {

            const s = await invoke<ImportStatus>("get_import_status", { taskId });

            setStatus(s);

            if (s.stage === "finished" || s.progress >= 1.0) {

              if (intervalRef.current) {

                clearInterval(intervalRef.current);

                intervalRef.current = null;

              }

              // 显示完成弹窗

              setCompletionStats({

                skipped: s.skipped_files || 0,

                new: s.new_files || 0,

              });

              setShowCompletionModal(true);

            }

          } catch (err) {

            console.error("Failed to get import status:", err);

          }

        }, 500);

        cleanupInterval = () => {

          if (intervalRef.current) {

            clearInterval(intervalRef.current);

            intervalRef.current = null;

          }

        };

      } catch (err) {

        console.error("Failed to set up event listener, falling back to polling:", err);

        intervalRef.current = window.setInterval(async () => {

          try {

            const s = await invoke<ImportStatus>("get_import_status", { taskId });

            setStatus(s);

            if (s.stage === "finished" || s.progress >= 1.0) {

              if (intervalRef.current) {

                clearInterval(intervalRef.current);

                intervalRef.current = null;

              }

              // 显示完成弹窗

              setCompletionStats({

                skipped: s.skipped_files || 0,

                new: s.new_files || 0,

              });

              setShowCompletionModal(true);

            }

          } catch (err) {

            console.error("Failed to get import status:", err);

          }

        }, 500);

        cleanupInterval = () => {

          if (intervalRef.current) {

            clearInterval(intervalRef.current);

            intervalRef.current = null;

          }

        };

      }

    })();



    return () => {

      if (unlisten) unlisten();

      if (cleanupInterval) cleanupInterval();

    };

  }, [taskId]);



  const handleSelectInputPath = async () => {

    try {

      const selected = await open({

        directory: true,

        multiple: false,

        title: "选择输入目录",

      });

      if (selected && typeof selected === "string") {

        setNewProjectPath(selected);

      }

    } catch (err) {

      console.error("Failed to select input path:", err);

    }

  };

  const handleSelectExcludePath = async () => {

    try {

      const selected = await open({

        directory: true,

        multiple: false,

        title: "选择排除目录",

      });

      if (selected && typeof selected === "string") {

        // 计算相对于输入路径的相对路径

        if (newProjectPath) {

          const inputPath = newProjectPath.replace(/\\/g, "/");

          const excludePath = selected.replace(/\\/g, "/");

          if (excludePath.startsWith(inputPath)) {

            const relativePath = excludePath.substring(inputPath.length).replace(/^\//, "");

            if (relativePath && !newProjectExclude.includes(relativePath)) {

              setNewProjectExclude([...newProjectExclude, relativePath]);

            }

          } else {

            // 如果不在输入路径下，直接使用完整路径的最后一部分作为相对路径

            const pathParts = excludePath.split("/");

            const lastPart = pathParts[pathParts.length - 1];

            if (lastPart && !newProjectExclude.includes(lastPart)) {

              setNewProjectExclude([...newProjectExclude, lastPart]);

            }

          }

        } else {

          alert("请先选择输入路径");

        }

      }

    } catch (err) {

      console.error("Failed to select exclude path:", err);

    }

  };

  const handleRemoveExcludePath = (path: string) => {

    setNewProjectExclude(newProjectExclude.filter((p) => p !== path));

  };



  const handleCreateProject = async () => {

    if (!newProjectName.trim() || !newProjectPath.trim()) {

      alert("请填写项目名称和输入路径");

      return;

    }

    try {

      await invoke<Project>("create_project", {

        name: newProjectName.trim(),

        inputPath: newProjectPath.trim(),

        excludePatterns: newProjectExclude,

      });

      setShowNewProjectModal(false);

      setNewProjectName("");

      setNewProjectPath("");

      setNewProjectExclude([]);

      loadProjects();

    } catch (err) {

      console.error("Failed to create project:", err);

      alert("创建项目失败");

    }

  };



  const handleDeleteProject = async (projectId: string) => {

    if (!confirm("确定要删除这个项目吗？")) {

      return;

    }

    try {

      await invoke("delete_project", { projectId });

      loadProjects();

      setSelectedProjects((prev) => {

        const next = new Set(prev);

        next.delete(projectId);

        return next;

      });

    } catch (err) {

      console.error("Failed to delete project:", err);

      alert("删除项目失败");

    }

  };



  const handleToggleProject = (projectId: string) => {

    setSelectedProjects((prev) => {

      const next = new Set(prev);

      if (next.has(projectId)) {

        next.delete(projectId);

      } else {

        next.add(projectId);

      }

      return next;

    });

  };



  const handleStart = async () => {

    if (selectedProjects.size === 0) {

      alert("请至少选择一个项目");

      return;

    }

    try {

      const projectIds = Array.from(selectedProjects);

      const id = await invoke<string>("start_import_task", {

        projectIds,

        threadCount,

      });

      setTaskId(id);

      setStatus(null);

    } catch (err) {

      console.error("Failed to start import:", err);

      alert("启动导入失败");

    }

  };



  return (

    <section className="page page-import">

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "20px" }}>

        <h1 className="page-title" style={{ margin: 0 }}>导入音乐</h1>

        <div style={{ display: "flex", gap: "12px", alignItems: "center" }}>

          <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>

            <label>线程数:</label>

            <input

              type="number"

              value={threadCount}

              onChange={(e) => setThreadCount(Number(e.target.value))}

              className="input"

              style={{ width: "80px" }}

              min={1}

              max={16}

            />

          </div>

          <button

            onClick={() => setShowNewProjectModal(true)}

            className="btn btn-secondary"

          >

            新增项目

          </button>

        </div>

      </div>



      <div className="card">

        {projects.length === 0 ? (

          <div style={{ textAlign: "center", padding: "40px", color: "#666" }}>

            暂无项目，请点击"新增项目"按钮创建项目

          </div>

        ) : (

          <div className="projects-list">

            {projects.map((project) => (

              <div key={project.id} className="project-item" style={{ marginBottom: "12px", padding: "12px", border: "1px solid #e0e0e0", borderRadius: "4px" }}>

                <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>

                  <input

                    type="checkbox"

                    checked={selectedProjects.has(project.id)}

                    onChange={() => handleToggleProject(project.id)}

                    style={{ width: "18px", height: "18px" }}

                  />

                  <div style={{ flex: 1 }}>

                    <div style={{ fontWeight: "bold", marginBottom: "4px" }}>{project.name}</div>

                    <div style={{ fontSize: "14px", color: "#666", marginBottom: "2px" }}>路径: {project.input_path}</div>

                    {project.exclude_patterns.length > 0 && (

                      <div style={{ fontSize: "14px", color: "#666" }}>

                        排除: {project.exclude_patterns.join(", ")}

                      </div>

                    )}

                  </div>

                  <button

                    onClick={() => handleDeleteProject(project.id)}

                    className="btn btn-secondary"

                    style={{ padding: "4px 12px", fontSize: "14px" }}

                  >

                    删除

                  </button>

                </div>

              </div>

            ))}

          </div>

        )}



        <div style={{ marginTop: "20px" }}>

          <button

            onClick={handleStart}

            className="btn btn-primary"

            disabled={selectedProjects.size === 0 || taskId !== null}

          >

            开始导入

          </button>

        </div>



        {status && (

          <div className="progress-section" style={{ marginTop: "20px" }}>

            <div className="progress-info" style={{ display: "flex", justifyContent: "space-between", marginBottom: "8px" }}>

              <span>阶段: {status.current_stage_detail || status.stage}</span>

              <span>进度: {(status.progress * 100).toFixed(1)}%</span>

              {status.total_files !== undefined && status.processed_files !== undefined && (

                <span>({status.processed_files}/{status.total_files})</span>

              )}

            </div>

            <div className="progress-bar">

              <div

                className="progress-fill"

                style={{ width: `${status.progress * 100}%` }}

              />

            </div>

            <div style={{ marginTop: "12px" }}>

              <button

                onClick={() => setShowDetailModal(true)}

                className="btn btn-secondary"

                style={{ fontSize: "14px", padding: "4px 12px" }}

              >

                详情

              </button>

            </div>

          </div>

        )}

      </div>



      {/* 新增项目弹窗 */}

      {showNewProjectModal && (

        <div

          style={{

            position: "fixed",

            top: 0,

            left: 0,

            right: 0,

            bottom: 0,

            backgroundColor: "rgba(0,0,0,0.5)",

            display: "flex",

            justifyContent: "center",

            alignItems: "center",

            zIndex: 1000,

          }}

          onClick={() => setShowNewProjectModal(false)}

        >

          <div

            className="card"

            style={{ width: "500px", maxWidth: "90vw" }}

            onClick={(e) => e.stopPropagation()}

          >

            <h2 style={{ marginTop: 0 }}>新增项目</h2>

            <div className="form-row">

              <label>项目名称:</label>

              <input

                type="text"

                value={newProjectName}

                onChange={(e) => setNewProjectName(e.target.value)}

                className="input"

                placeholder="例如: CloudMusic"

              />

            </div>

            <div className="form-row">

              <label>输入路径:</label>

              <div style={{ display: "flex", gap: "8px", width: "100%" }}>

                <input

                  type="text"

                  value={newProjectPath}

                  onChange={(e) => setNewProjectPath(e.target.value)}

                  className="input"

                  style={{ flex: 1 }}

                  placeholder="选择输入目录"

                />

                <button onClick={handleSelectInputPath} className="btn btn-secondary" style={{ whiteSpace: "nowrap" }}>

                  浏览...

                </button>

              </div>

            </div>

            <div className="form-row">

              <label>排除路径:</label>

              <div style={{ display: "flex", flexDirection: "column", gap: "8px", width: "100%" }}>

                {newProjectExclude.length > 0 && (

                  <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>

                    {newProjectExclude.map((path, idx) => (

                      <div

                        key={idx}

                        style={{

                          display: "flex",

                          alignItems: "center",

                          justifyContent: "space-between",

                          padding: "6px 12px",

                          backgroundColor: "#f5f5f5",

                          borderRadius: "4px",

                        }}

                      >

                        <span style={{ flex: 1, fontSize: "14px" }}>{path}</span>

                        <button

                          onClick={() => handleRemoveExcludePath(path)}

                          style={{

                            padding: "4px 8px",

                            fontSize: "12px",

                            backgroundColor: "#ff4444",

                            color: "white",

                            border: "none",

                            borderRadius: "4px",

                            cursor: "pointer",

                          }}

                        >

                          移除

                        </button>

                      </div>

                    ))}

                  </div>

                )}

                <button

                  onClick={handleSelectExcludePath}

                  className="btn btn-secondary"

                  style={{ alignSelf: "flex-start" }}

                >

                  选择排除目录

                </button>

              </div>

            </div>

            <div style={{ display: "flex", gap: "12px", justifyContent: "flex-end", marginTop: "20px" }}>

              <button

                onClick={() => setShowNewProjectModal(false)}

                className="btn btn-secondary"

              >

                取消

              </button>

              <button onClick={handleCreateProject} className="btn btn-primary">

                创建

              </button>

            </div>

          </div>

        </div>

      )}



      {/* 详情弹窗 */}

      {showDetailModal && status && (

        <div

          style={{

            position: "fixed",

            top: 0,

            left: 0,

            right: 0,

            bottom: 0,

            backgroundColor: "rgba(0,0,0,0.5)",

            display: "flex",

            justifyContent: "center",

            alignItems: "center",

            zIndex: 1000,

          }}

          onClick={() => setShowDetailModal(false)}

        >

          <div

            className="card"

            style={{ width: "700px", maxWidth: "90vw", maxHeight: "80vh", overflow: "auto" }}

            onClick={(e) => e.stopPropagation()}

          >

            <h2 style={{ marginTop: 0 }}>导入详情</h2>

            {status.current_project && (

              <div style={{ marginBottom: "16px", padding: "8px", backgroundColor: "#f5f5f5", borderRadius: "4px" }}>

                <strong>当前项目:</strong> {status.current_project}

              </div>

            )}

            {status.project_progress && status.project_progress.length > 0 && (

              <div style={{ marginBottom: "20px" }}>

                <h3 style={{ fontSize: "16px", marginBottom: "12px" }}>项目进度</h3>

                {status.project_progress.map((proj) => (

                  <div key={proj.project_id} style={{ marginBottom: "12px", padding: "12px", border: "1px solid #e0e0e0", borderRadius: "4px" }}>

                    <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "8px" }}>

                      <span style={{ fontWeight: "bold" }}>{proj.project_name}</span>

                      <span>{proj.status === "waiting" ? "等待中" : proj.status === "processing" ? "处理中" : proj.status === "completed" ? "已完成" : "失败"}</span>

                    </div>

                    <div className="progress-bar" style={{ marginBottom: "4px" }}>

                      <div

                        className="progress-fill"

                        style={{ width: `${proj.progress * 100}%` }}

                      />

                    </div>

                    <div style={{ fontSize: "14px", color: "#666" }}>

                      {proj.processed_files}/{proj.total_files} 文件

                    </div>

                  </div>

                ))}

              </div>

            )}

            {status.threads && status.threads.length > 0 && (

              <div>

                <h3 style={{ fontSize: "16px", marginBottom: "12px" }}>线程详情</h3>

                {status.threads.map((t) => (

                  <div key={t.thread_id} style={{ marginBottom: "12px", padding: "12px", border: "1px solid #e0e0e0", borderRadius: "4px" }}>

                    <div style={{ display: "flex", justifyContent: "space-between", marginBottom: "8px" }}>

                      <span>线程 {t.thread_id}</span>

                      <span>{(t.progress * 100).toFixed(1)}%</span>

                    </div>

                    <div className="progress-bar progress-bar-small" style={{ marginBottom: "4px" }}>

                      <div

                        className="progress-fill"

                        style={{ width: `${t.progress * 100}%` }}

                      />

                    </div>

                    {t.current_file && (

                      <div style={{ fontSize: "14px", color: "#666", wordBreak: "break-all" }}>

                        {t.current_file}

                      </div>

                    )}

                  </div>

                ))}

              </div>

            )}

            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: "20px" }}>

              <button

                onClick={() => setShowDetailModal(false)}

                className="btn btn-primary"

              >

                关闭

              </button>

            </div>

          </div>

        </div>

      )}

      {/* 完成弹窗 */}
      {showCompletionModal && completionStats && (
        <div
          style={{
            position: "fixed",
            top: 0,
            left: 0,
            right: 0,
            bottom: 0,
            backgroundColor: "rgba(0,0,0,0.5)",
            display: "flex",
            justifyContent: "center",
            alignItems: "center",
            zIndex: 1000,
          }}
          onClick={() => setShowCompletionModal(false)}
        >
          <div
            className="card"
            style={{ width: "400px", maxWidth: "90vw" }}
            onClick={(e) => e.stopPropagation()}
          >
            <h2 style={{ marginTop: 0 }}>导入完成</h2>
            <div style={{ marginBottom: "20px" }}>
              <div style={{ marginBottom: "12px", padding: "12px", backgroundColor: "#f0f0f0", borderRadius: "4px" }}>
                <div style={{ fontSize: "18px", fontWeight: "bold", marginBottom: "8px" }}>统计信息</div>
                <div style={{ fontSize: "16px", marginBottom: "4px" }}>
                  <span style={{ color: "#666" }}>已匹配（跳过）:</span>{" "}
                  <span style={{ color: "#4CAF50", fontWeight: "bold" }}>{completionStats.skipped}</span> 个文件
                </div>
                <div style={{ fontSize: "16px" }}>
                  <span style={{ color: "#666" }}>新匹配（处理）:</span>{" "}
                  <span style={{ color: "#2196F3", fontWeight: "bold" }}>{completionStats.new}</span> 个文件
                </div>
              </div>
            </div>
            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: "20px" }}>
              <button
                onClick={() => {
                  setShowCompletionModal(false);
                  setCompletionStats(null);
                }}
                className="btn btn-primary"
              >
                确定
              </button>
            </div>
          </div>
        </div>
      )}

    </section>

  );

};

