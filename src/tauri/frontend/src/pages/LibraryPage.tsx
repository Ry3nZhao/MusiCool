import React, { useEffect, useState } from "react";

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { usePlayer } from "../player/PlayerContext";
import { FileInfo } from "../player/types";
import { PlayIcon, ShuffleIcon } from "../player/icons";






type SortField = "name" | "artist" | "album" | "size" | "duration";

type SortOrder = "asc" | "desc";



export const LibraryPage: React.FC = () => {

  const [items, setItems] = useState<FileInfo[]>([]);

  const [loading, setLoading] = useState(false);

  const [error, setError] = useState<string | null>(null);

  const [sortField, setSortField] = useState<SortField>("name");

  const [sortOrder, setSortOrder] = useState<SortOrder>("asc");

  const [searchQuery, setSearchQuery] = useState("");

  const { playList, status, setMode } = usePlayer();



  const loadLibrary = async () => {

    setLoading(true);

    setError(null);

    try {

      const result = await invoke<FileInfo[]>("list_library_files", {});

      setItems(result);

    } catch (err) {

      console.error(err);

      setError("加载音乐库失败，请检查设置中的音乐库路径。");

    } finally {

      setLoading(false);

    }

  };



  useEffect(() => {

    loadLibrary();

  }, []);



  const handleSort = (field: SortField) => {

    if (sortField === field) {

      setSortOrder(sortOrder === "asc" ? "desc" : "asc");

    } else {

      setSortField(field);

      setSortOrder("asc");

    }

  };



  const formatSize = (bytes: number): string => {

    if (bytes < 1024) return `${bytes} B`;

    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(2)} KB`;

    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;

    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;

  };



  const formatDuration = (seconds?: number): string => {

    if (!seconds) return "-";

    const mins = Math.floor(seconds / 60);

    const secs = seconds % 60;

    return `${mins}:${secs.toString().padStart(2, "0")}`;

  };



  // 计算统计信息
  const stats = React.useMemo(() => {
    const totalSongs = items.length;
    const totalSize = items.reduce((sum, item) => sum + item.size, 0);
    return {
      totalSongs,
      totalSize,
    };
  }, [items]);

  const sortedAndFilteredItems = React.useMemo(() => {

    let filtered = items;

    if (searchQuery.trim()) {

      const query = searchQuery.toLowerCase();

      filtered = items.filter((item) =>

        item.name.toLowerCase().includes(query) ||

        item.file_name.toLowerCase().includes(query) ||

        (item.artist && item.artist.toLowerCase().includes(query)) ||

        (item.album && item.album.toLowerCase().includes(query))

      );

    }

    const sorted = [...filtered].sort((a, b) => {

      let comparison = 0;

      switch (sortField) {

        case "name":

          comparison = a.name.localeCompare(b.name, "zh-CN");

          break;

        case "artist":

          comparison = (a.artist || "").localeCompare(b.artist || "", "zh-CN");

          break;

        case "album":

          comparison = (a.album || "").localeCompare(b.album || "", "zh-CN");

          break;

        case "size":

          comparison = a.size - b.size;

          break;

        case "duration":

          comparison = (a.duration || 0) - (b.duration || 0);

          break;

      }

      return sortOrder === "asc" ? comparison : -comparison;

    });

    return sorted;

  }, [items, sortField, sortOrder, searchQuery]);



  const getSortIcon = (field: SortField) => {

    if (sortField !== field) return "↕";

    return sortOrder === "asc" ? "↑" : "↓";

  };



  return (

    <section className="page page-library">

      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "20px" }}>

        <h1 className="page-title" style={{ margin: 0 }}>音乐库</h1>

        <div style={{ display: "flex", gap: "8px" }}>
          <button
            className="btn btn-primary btn-with-icon"
            disabled={sortedAndFilteredItems.length === 0}
            onClick={async () => {
              if (status.shuffle) await setMode(status.repeat, false);
              playList(sortedAndFilteredItems, 0);
            }}
          >
            <PlayIcon size={14} /> 播放全部
          </button>
          <button
            className="btn btn-secondary btn-with-icon"
            disabled={sortedAndFilteredItems.length === 0}
            onClick={async () => {
              if (!status.shuffle) await setMode(status.repeat, true);
              playList(sortedAndFilteredItems, Math.floor(Math.random() * sortedAndFilteredItems.length));
            }}
          >
            <ShuffleIcon size={14} /> 随机播放
          </button>
          <button onClick={loadLibrary} disabled={loading} className="btn btn-secondary">
            {loading ? "刷新中..." : "刷新列表"}
          </button>
        </div>

      </div>

      {/* 统计信息卡片 */}
      {!loading && items.length > 0 && (
        <div className="card" style={{ marginBottom: "20px", backgroundColor: "#f8f9fa" }}>
          <div style={{ display: "flex", gap: "32px", flexWrap: "wrap" }}>
            <div style={{ display: "flex", flexDirection: "column" }}>
              <div style={{ fontSize: "14px", color: "#666", marginBottom: "4px" }}>歌曲总数</div>
              <div style={{ fontSize: "24px", fontWeight: "bold", color: "#2196F3" }}>
                {stats.totalSongs.toLocaleString()}
              </div>
            </div>
            <div style={{ display: "flex", flexDirection: "column" }}>
              <div style={{ fontSize: "14px", color: "#666", marginBottom: "4px" }}>占用空间</div>
              <div style={{ fontSize: "24px", fontWeight: "bold", color: "#4CAF50" }}>
                {formatSize(stats.totalSize)}
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="card">

        {error && <div className="error-text" style={{ marginBottom: "16px" }}>{error}</div>}

        <div style={{ marginBottom: "16px" }}>

          <input

            type="text"

            placeholder="搜索歌名、歌手或专辑..."

            value={searchQuery}

            onChange={(e) => setSearchQuery(e.target.value)}

            className="input"

            style={{ width: "100%", maxWidth: "400px" }}

          />

        </div>

        {loading ? (

          <div style={{ textAlign: "center", padding: "40px" }}>加载中...</div>

        ) : sortedAndFilteredItems.length === 0 ? (

          <div style={{ textAlign: "center", padding: "40px", color: "#666" }}>

            {searchQuery ? "没有找到匹配的文件" : "暂无数据，请先导入音乐。"}

          </div>

        ) : (

          <div className="library-table" style={{ overflowX: "auto" }}>

            <table style={{ width: "100%", borderCollapse: "collapse" }}>

              <thead>

                <tr style={{ borderBottom: "2px solid #e0e0e0" }}>

                  <th

                    style={{

                      padding: "12px",

                      textAlign: "left",

                      cursor: "pointer",

                      userSelect: "none",

                      backgroundColor: "#f5f5f5",

                    }}

                    onClick={() => handleSort("name")}

                  >

                    歌曲 {getSortIcon("name")}

                  </th>

                  <th

                    style={{

                      padding: "12px",

                      textAlign: "left",

                      cursor: "pointer",

                      userSelect: "none",

                      backgroundColor: "#f5f5f5",

                    }}

                    onClick={() => handleSort("artist")}

                  >

                    歌手 {getSortIcon("artist")}

                  </th>

                  <th

                    style={{

                      padding: "12px",

                      textAlign: "left",

                      cursor: "pointer",

                      userSelect: "none",

                      backgroundColor: "#f5f5f5",

                    }}

                    onClick={() => handleSort("album")}

                  >

                    专辑 {getSortIcon("album")}

                  </th>

                  <th

                    style={{

                      padding: "12px",

                      textAlign: "right",

                      cursor: "pointer",

                      userSelect: "none",

                      backgroundColor: "#f5f5f5",

                    }}

                    onClick={() => handleSort("size")}

                  >

                    大小 {getSortIcon("size")}

                  </th>

                  <th

                    style={{

                      padding: "12px",

                      textAlign: "left",

                      cursor: "pointer",

                      userSelect: "none",

                      backgroundColor: "#f5f5f5",

                    }}

                    onClick={() => handleSort("duration")}

                  >

                    持续时间 {getSortIcon("duration")}

                  </th>

                </tr>

              </thead>

              <tbody>

                {sortedAndFilteredItems.map((item, index) => (
                  <tr
                    key={item.path}
                    className={`library-row ${status.path === item.path ? "is-playing" : ""}`}
                    onDoubleClick={() => playList(sortedAndFilteredItems, index)}
                    title="双击播放"
                  >

                    <td style={{ padding: "12px" }}>
                      <div className="track-cell">
                        <div className="track-cover-wrap">
                          {item.cover_path ? (
                            <img className="track-cover" src={convertFileSrc(item.cover_path)} alt="" loading="lazy" />
                          ) : (
                            <div className="track-cover track-cover-empty" />
                          )}
                          <button
                            className="track-play"
                            title="播放"
                            onClick={(e) => {
                              e.stopPropagation();
                              playList(sortedAndFilteredItems, index);
                            }}
                          >
                            {status.path === item.path && status.state === "playing" ? <span className="eq"><i /><i /><i /></span> : <PlayIcon size={14} />}
                          </button>
                        </div>
                        <span title={item.file_name}>{item.name}</span>
                        {item.lyrics_path && <span className="track-badge" title="有歌词">词</span>}
                        <span className="track-format">{item.format.toUpperCase()}</span>
                      </div>
                    </td>

                    <td style={{ padding: "12px", color: "#666" }}>{item.artist || "-"}</td>

                    <td style={{ padding: "12px", color: "#666" }}>{item.album || "-"}</td>

                    <td style={{ padding: "12px", textAlign: "right" }}>{formatSize(item.size)}</td>

                    <td style={{ padding: "12px", color: "#666" }}>{formatDuration(item.duration)}</td>

                  </tr>

                ))}

              </tbody>

            </table>

          </div>

        )}

      </div>

    </section>

  );

};

