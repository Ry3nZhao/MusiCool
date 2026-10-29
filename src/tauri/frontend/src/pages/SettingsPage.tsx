import React, { useEffect, useState } from "react";

import { invoke } from "@tauri-apps/api/tauri";

import { open } from "@tauri-apps/api/dialog";



interface Settings {

  library_path?: string;

  max_threads?: number;

  exclude_patterns?: string[];

}



export const SettingsPage: React.FC = () => {

  const [libraryPath, setLibraryPath] = useState("");

  const [maxThreads, setMaxThreads] = useState(4);

  const [excludePatterns, setExcludePatterns] = useState<string[]>([]);

  const [newExclude, setNewExclude] = useState("");



  useEffect(() => {

    invoke<Settings>("get_settings")

      .then((s) => {

        if (s.library_path) setLibraryPath(s.library_path);

        if (s.max_threads) setMaxThreads(s.max_threads);

        if (s.exclude_patterns) setExcludePatterns(s.exclude_patterns);

      })

      .catch(console.error);

  }, []);



  const handleSave = async () => {

    try {

      await invoke("set_library_path", { path: libraryPath });

      await invoke("set_max_threads", { maxThreads });

      await invoke("set_exclude_patterns", { patterns: excludePatterns });

      alert("设置已保存");

    } catch (err) {

      console.error(err);

      alert("保存失败，请查看控制台日志");

    }

  };



  const handleAddExclude = () => {

    const trimmed = newExclude.trim();

    if (!trimmed) return;

    setExcludePatterns((prev) => Array.from(new Set([...prev, trimmed])));

    setNewExclude("");

  };



  const handleRemoveExclude = (pattern: string) => {

    setExcludePatterns((prev) => prev.filter((p) => p !== pattern));

  };

  const handleSelectLibraryPath = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择音乐库目录",
      });
      if (selected && typeof selected === "string") {
        setLibraryPath(selected);
      }
    } catch (err) {
      console.error("Failed to select library path:", err);
    }
  };

  return (

    <section className="page page-settings">

      <h1 className="page-title">设置</h1>



      <div className="card">

        <div className="form-row">

          <label>音乐库路径:</label>

          <div style={{ display: "flex", gap: "8px", width: "100%" }}>

            <input

              type="text"

              value={libraryPath}

              onChange={(e) => setLibraryPath(e.target.value)}

              className="input"

              style={{ flex: 1 }}

            />

            <button onClick={handleSelectLibraryPath} className="btn btn-secondary" style={{ whiteSpace: "nowrap" }}>

              浏览...

            </button>

          </div>

        </div>



        <div className="form-row">

          <label>最大线程数:</label>

          <input

            type="number"

            min={1}

            max={16}

            value={maxThreads}

            onChange={(e) => setMaxThreads(Number(e.target.value))}

            className="input"

          />

        </div>



        <div className="form-row">

          <label>排除规则:</label>

          <div className="exclude-list">

            {excludePatterns.map((p) => (

              <div key={p} className="exclude-item">

                <span>{p}</span>

                <button onClick={() => handleRemoveExclude(p)}>移除</button>

              </div>

            ))}

          </div>

          <div className="exclude-input-row">

            <input

              type="text"

              value={newExclude}

              onChange={(e) => setNewExclude(e.target.value)}

              className="input"

              placeholder="输入新的排除规则"

            />

            <button onClick={handleAddExclude}>添加</button>

          </div>

        </div>



        <button onClick={handleSave} className="btn btn-primary">

          保存设置

        </button>

      </div>

    </section>

  );

};

