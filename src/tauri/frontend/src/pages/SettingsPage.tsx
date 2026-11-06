import React, { useEffect, useState } from "react";

import { invoke } from "@tauri-apps/api/tauri";

import { open } from "@tauri-apps/api/dialog";
import { DeviceInfo, formatRate } from "../player/types";

interface PlayerSettings {
  device_id?: string | null;
  match_sample_rate: boolean;
  exclusive: boolean;
}

/** 音频输出设置：修改后立即生效（正在播放的歌从当前位置按新设置继续）。 */
const AudioOutputSettings: React.FC = () => {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [exclusiveSupported, setExclusiveSupported] = useState(false);
  const [cfg, setCfg] = useState<PlayerSettings>({ device_id: null, match_sample_rate: true, exclusive: false });
  const [error, setError] = useState<string | null>(null);

  const loadDevices = () =>
    invoke<{ devices: DeviceInfo[]; exclusive_supported: boolean }>("player_output_options")
      .then((o) => {
        setDevices(o.devices);
        setExclusiveSupported(o.exclusive_supported);
      })
      .catch((e) => setError(String(e)));

  useEffect(() => {
    invoke<{ player?: PlayerSettings }>("get_settings")
      .then((s) => s.player && setCfg(s.player))
      .catch(console.error);
    loadDevices();
  }, []);

  const apply = async (next: PlayerSettings) => {
    setCfg(next);
    setError(null);
    try {
      await invoke("player_set_output", {
        deviceId: next.device_id ?? null,
        matchSampleRate: next.match_sample_rate,
        exclusive: next.exclusive,
      });
    } catch (e) {
      setError(String(e));
    }
  };

  const selected = devices.find((d) => d.id === cfg.device_id) ?? devices.find((d) => d.is_default);
  const missing = cfg.device_id && !devices.some((d) => d.id === cfg.device_id);

  return (
    <div className="card" style={{ marginTop: 24 }}>
      <h2 className="card-title">音频输出</h2>
      <div className="form-row">
        <label>输出设备</label>
        <div style={{ display: "flex", gap: 8 }}>
          <select
            className="input"
            value={cfg.device_id ?? ""}
            onChange={(e) => apply({ ...cfg, device_id: e.target.value || null })}
          >
            <option value="">跟随系统默认设备</option>
            {devices.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
                {d.is_default ? "（系统默认）" : ""}
              </option>
            ))}
            {missing && <option value={cfg.device_id!}>已断开的设备</option>}
          </select>
          <button className="btn btn-secondary" onClick={loadDevices}>
            刷新
          </button>
        </div>
        {selected && selected.sample_rates.length > 0 && (
          <div className="hint">
            {selected.name} 支持：{selected.sample_rates.map(formatRate).join("、")}
          </div>
        )}
      </div>
      <div className="form-row">
        <label className="check">
          <input
            type="checkbox"
            checked={cfg.match_sample_rate}
            onChange={(e) => apply({ ...cfg, match_sample_rate: e.target.checked })}
          />
          <span>采样率跟随歌曲（推荐）</span>
        </label>
        <div className="hint">
          把设备切换到歌曲的原始采样率，避免系统混音器重采样；设备不支持时用高质量重采样到最接近的采样率。关闭后始终使用设备当前采样率。
        </div>
      </div>
      {exclusiveSupported && (
        <div className="form-row">
          <label className="check">
            <input type="checkbox" checked={cfg.exclusive} onChange={(e) => apply({ ...cfg, exclusive: e.target.checked })} />
            <span>独占模式（WASAPI Exclusive）</span>
          </label>
          <div className="hint">播放时独占设备、绕过系统混音器，其他程序将暂时无法出声。设备不支持时自动退回共享模式。</div>
        </div>
      )}
      <div className="hint">音量调到 100% 时播放器不做任何数字处理，配合上面的设置即可 bit-perfect 输出。</div>
      {error && <div className="error-text" style={{ marginTop: 8 }}>{error}</div>}
    </div>
  );
};



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

      await invoke("set_max_threads", { threads: maxThreads });

      await invoke("set_exclude_patterns", { patterns: excludePatterns });

      alert("设置已保存");

    } catch (err) {

      console.error(err);

      alert(`保存失败: ${err}`);

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

      <AudioOutputSettings />

    </section>

  );

};

