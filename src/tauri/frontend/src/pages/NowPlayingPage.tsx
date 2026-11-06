import React, { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { Cover } from "../player/Cover";
import { usePlayer, usePosition } from "../player/PlayerContext";
import { SignalBadge } from "../player/PlayerBar";
import { NoteIcon } from "../player/icons";
import { fileStem, formatRate, formatTime, sourceLabel } from "../player/types";

interface LyricLine {
  time: number;
  text: string;
  /** 同一时间戳的其他行（通常是翻译） */
  sub?: string;
}

/** 解析 LRC：支持一行多个时间戳、[mm:ss.xx] / [mm:ss:xx] / [mm:ss]、[offset:±ms]。 */
const parseLrc = (raw: string): LyricLine[] => {
  let offset = 0;
  const entries: { time: number; text: string }[] = [];
  for (const line of raw.split(/\r?\n/)) {
    const off = line.match(/^\[offset:\s*([+-]?\d+)\s*\]/i);
    if (off) {
      offset = Number(off[1]) / 1000;
      continue;
    }
    const stamps = [...line.matchAll(/\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g)];
    if (stamps.length === 0) continue;
    const text = line.replace(/\[[^\]]*\]/g, "").trim();
    for (const m of stamps) {
      const frac = m[3] ? Number(m[3]) / Math.pow(10, m[3].length) : 0;
      entries.push({ time: Number(m[1]) * 60 + Number(m[2]) + frac - offset, text });
    }
  }
  entries.sort((a, b) => a.time - b.time);
  const out: LyricLine[] = [];
  for (const e of entries) {
    const last = out[out.length - 1];
    if (last && Math.abs(last.time - e.time) < 0.01) {
      if (e.text && e.text !== last.text) last.sub = last.sub ? `${last.sub} / ${e.text}` : e.text;
    } else {
      out.push({ time: e.time, text: e.text });
    }
  }
  return out;
};

const Lyrics: React.FC<{ path?: string }> = ({ path }) => {
  const { seek } = usePlayer();
  const position = usePosition();
  const [lines, setLines] = useState<LyricLine[] | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const userScrollUntil = useRef(0);

  useEffect(() => {
    setLines(null);
    if (!path) return;
    let alive = true;
    invoke<string>("read_lyrics", { path })
      .then((t) => alive && setLines(parseLrc(t)))
      .catch(() => alive && setLines([]));
    return () => {
      alive = false;
    };
  }, [path]);

  // 略微提前一点高亮，抵消输出延迟带来的“慢半拍”感
  const t = position + 0.15;
  const active = useMemo(() => {
    if (!lines) return -1;
    let i = -1;
    for (let k = 0; k < lines.length; k++) {
      if (lines[k].time <= t) i = k;
      else break;
    }
    return i;
  }, [lines, t]);

  useEffect(() => {
    const box = boxRef.current;
    if (!box || active < 0 || performance.now() < userScrollUntil.current) return;
    const el = box.querySelector<HTMLElement>(`[data-i="${active}"]`);
    if (el) box.scrollTo({ top: el.offsetTop - box.clientHeight / 2 + el.clientHeight / 2, behavior: "smooth" });
  }, [active]);

  if (!path) return <div className="lyrics-empty">这首歌没有歌词</div>;
  if (lines === null) return <div className="lyrics-empty">加载歌词…</div>;
  if (lines.length === 0) return <div className="lyrics-empty">歌词文件中没有带时间轴的内容</div>;
  return (
    <div className="lyrics" ref={boxRef} onWheel={() => (userScrollUntil.current = performance.now() + 3000)}>
      <div className="lyrics-pad" />
      {lines.map((l, i) => (
        <div
          key={i}
          data-i={i}
          className={`lyric-line ${i === active ? "is-active" : ""}`}
          onClick={() => seek(Math.max(0, l.time))}
          title={`跳到 ${formatTime(l.time)}`}
        >
          <div>{l.text || "♪"}</div>
          {l.sub && <div className="lyric-sub">{l.sub}</div>}
        </div>
      ))}
      <div className="lyrics-pad" />
    </div>
  );
};

const SignalChain: React.FC = () => {
  const { status } = usePlayer();
  const sig = status.signal;
  if (!sig) return null;
  const src = sig.source;
  return (
    <div className="chain">
      <div className="chain-step">
        <div className="chain-label">源文件</div>
        <div className="chain-value">{sourceLabel(src)}</div>
        <div className="chain-detail">
          {src.lossless ? "无损" : "有损"} · {src.channels} 声道{src.bitrate_kbps ? ` · ${src.bitrate_kbps} kbps` : ""}
        </div>
      </div>
      <div className="chain-step">
        <div className="chain-label">处理</div>
        <div className="chain-value">{sig.resampled ? `重采样 → ${formatRate(sig.output.sample_rate)}` : "无重采样"}</div>
        <div className="chain-detail">{status.volume >= 1 ? "音量 100%，不做数字处理" : `数字音量 ${Math.round(Math.sqrt(status.volume) * 100)}%`}</div>
      </div>
      <div className="chain-step">
        <div className="chain-label">输出</div>
        <div className="chain-value">
          {formatRate(sig.output.sample_rate)} · {sig.output.sample_format}
        </div>
        <div className="chain-detail">
          {sig.device}
          {sig.output.exclusive ? " · 独占" : ""}
        </div>
      </div>
      <div className={`chain-verdict ${sig.bit_perfect ? "is-perfect" : ""}`}>
        {sig.bit_perfect ? "Bit-perfect：送到设备的数据与源文件逐位一致" : sig.notes.join("；")}
      </div>
    </div>
  );
};

export const NowPlayingPage: React.FC = () => {
  const { status, current, queue, playList } = usePlayer();
  const [tab, setTab] = useState<"lyrics" | "queue">("lyrics");
  const title = current?.name ?? fileStem(status.path);

  if (!status.path) {
    return (
      <section className="page now-playing-empty">
        <NoteIcon size={40} />
        <p>还没有在播放的歌曲。去音乐库双击一首歌开始吧。</p>
      </section>
    );
  }

  return (
    <section className="now-playing">
      <div className="np-left">
        <Cover path={current?.cover_path} className="np-cover" emptyClassName="np-cover-empty" iconSize={64} />
        <h1 className="np-title">{title}</h1>
        <div className="np-sub">{[current?.artist, current?.album].filter(Boolean).join(" · ")}</div>
        <div style={{ marginTop: 12 }}>
          <SignalBadge />
        </div>
        <SignalChain />
      </div>
      <div className="np-right">
        <div className="np-tabs">
          <button className={tab === "lyrics" ? "is-on" : ""} onClick={() => setTab("lyrics")}>
            歌词
          </button>
          <button className={tab === "queue" ? "is-on" : ""} onClick={() => setTab("queue")}>
            播放队列 {queue.length > 0 && <span className="np-count">{queue.length}</span>}
          </button>
        </div>
        {tab === "lyrics" ? (
          <Lyrics key={status.path} path={current?.lyrics_path} />
        ) : (
          <div className="queue-list">
            {queue.map((t, i) => (
              <div
                key={`${t.path}-${i}`}
                className={`queue-item ${i === status.queue_index ? "is-current" : ""}`}
                onDoubleClick={() => playList(queue, i)}
                title="双击播放"
              >
                <span className="queue-no">{i === status.queue_index ? "♪" : i + 1}</span>
                <span className="queue-name">{t.name}</span>
                <span className="queue-artist">{t.artist ?? ""}</span>
                <span className="queue-dur">{formatTime(t.duration)}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
};
