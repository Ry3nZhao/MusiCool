import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { listen } from "@tauri-apps/api/event";
import { FileInfo, PlayerStatus, Repeat } from "./types";

type Control = "play" | "pause" | "toggle" | "next" | "prev" | "stop";

interface PlayerApi {
  status: PlayerStatus;
  /** 收到 status 的时刻（performance.now），用于在两次推送之间平滑推算进度 */
  receivedAt: number;
  queue: FileInfo[];
  current: FileInfo | null;
  playList: (tracks: FileInfo[], index: number) => Promise<void>;
  control: (action: Control) => Promise<void>;
  seek: (seconds: number) => Promise<void>;
  setVolume: (gain: number) => Promise<void>;
  setMode: (repeat: Repeat, shuffle: boolean) => Promise<void>;
}

const initial: PlayerStatus = {
  state: "stopped",
  queue_len: 0,
  queue_id: 0,
  position: 0,
  duration: 0,
  volume: 1,
  repeat: "off",
  shuffle: false,
};

const Ctx = createContext<PlayerApi | null>(null);

export const PlayerProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [status, setStatus] = useState<PlayerStatus>(initial);
  const [receivedAt, setReceivedAt] = useState(() => performance.now());
  const [queue, setQueue] = useState<FileInfo[]>([]);
  // 队列与后端 queue_id 的对应关系：发出播放请求后，后端下一个 queue_id 属于这份列表
  const queueIdRef = useRef<number | null>(null);
  const pendingQueue = useRef<FileInfo[] | null>(null);
  // 拖动音量时本地先行更新，避免被旧的推送拉回去
  const volumeOverride = useRef<{ value: number; until: number } | null>(null);

  const apply = useCallback((s: PlayerStatus) => {
    if (pendingQueue.current && queueIdRef.current !== null && s.queue_id > queueIdRef.current) {
      setQueue(pendingQueue.current);
      pendingQueue.current = null;
      queueIdRef.current = s.queue_id;
    } else if (queueIdRef.current === null) {
      queueIdRef.current = s.queue_id;
    }
    const ov = volumeOverride.current;
    if (ov && performance.now() < ov.until) s = { ...s, volume: ov.value };
    setStatus(s);
    setReceivedAt(performance.now());
  }, []);

  useEffect(() => {
    invoke<PlayerStatus>("player_status").then(apply).catch(console.error);
    const un = listen<PlayerStatus>("player-status", (e) => apply(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, [apply]);

  const playList = useCallback(async (tracks: FileInfo[], index: number) => {
    if (tracks.length === 0) return;
    pendingQueue.current = tracks;
    queueIdRef.current = queueIdRef.current ?? 0;
    await invoke("player_play_queue", { paths: tracks.map((t) => t.path), index });
  }, []);

  const control = useCallback(async (action: Control) => {
    await invoke("player_control", { action });
  }, []);

  const seek = useCallback(async (seconds: number) => {
    setStatus((s) => ({ ...s, position: seconds }));
    setReceivedAt(performance.now());
    await invoke("player_seek", { position: seconds });
  }, []);

  const setVolume = useCallback(async (gain: number) => {
    volumeOverride.current = { value: gain, until: performance.now() + 600 };
    setStatus((s) => ({ ...s, volume: gain }));
    await invoke("player_set_volume", { volume: gain });
  }, []);

  const setMode = useCallback(async (repeat: Repeat, shuffle: boolean) => {
    setStatus((s) => ({ ...s, repeat, shuffle }));
    await invoke("player_set_mode", { repeat, shuffle });
  }, []);

  const current =
    status.queue_index !== undefined && status.queue_index !== null && queue[status.queue_index]?.path === status.path
      ? queue[status.queue_index]
      : null;

  return (
    <Ctx.Provider value={{ status, receivedAt, queue, current, playList, control, seek, setVolume, setMode }}>
      {children}
    </Ctx.Provider>
  );
};

export const usePlayer = (): PlayerApi => {
  const v = useContext(Ctx);
  if (!v) throw new Error("usePlayer 必须在 PlayerProvider 内使用");
  return v;
};

/** 平滑的播放进度：在两次状态推送之间按真实时间推算。 */
export const usePosition = (): number => {
  const { status, receivedAt } = usePlayer();
  const [, tick] = useState(0);
  useEffect(() => {
    if (status.state !== "playing") return;
    // 10 fps 足够进度条和歌词使用，避免每帧重渲染
    const id = window.setInterval(() => tick((n) => n + 1), 100);
    return () => window.clearInterval(id);
  }, [status.state]);
  if (status.state !== "playing") return status.position;
  const p = status.position + (performance.now() - receivedAt) / 1000;
  return status.duration > 0 ? Math.min(p, status.duration) : p;
};
