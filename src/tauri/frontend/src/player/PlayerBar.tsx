import React, { useEffect, useState } from "react";
import { Cover } from "./Cover";
import { useNavigate } from "react-router-dom";
import { usePlayer, usePosition } from "./PlayerContext";
import { fileStem, formatTime, gainToSlider, sliderToGain, sourceLabel, formatRate } from "./types";
import { NextIcon, PauseIcon, PlayIcon, PrevIcon, RepeatIcon, ShuffleIcon, VolumeIcon } from "./icons";

/** 信号链路徽章：源格式 → 输出采样率，bit-perfect 时高亮。 */
export const SignalBadge: React.FC<{ compact?: boolean }> = ({ compact }) => {
  const { status } = usePlayer();
  const sig = status.signal;
  if (!sig) return null;
  const title = [
    `源：${sourceLabel(sig.source)}${sig.source.channels !== 2 ? ` · ${sig.source.channels} 声道` : ""}`,
    `输出：${formatRate(sig.output.sample_rate)} · ${sig.output.sample_format}${sig.output.exclusive ? " · 独占" : ""}`,
    `设备：${sig.device}`,
    ...(sig.bit_perfect ? ["送到设备的数据与源文件逐位一致"] : sig.notes),
  ].join("\n");
  return (
    <span className={`signal-badge ${sig.bit_perfect ? "is-perfect" : sig.resampled ? "is-resampled" : ""}`} title={title}>
      <span className="signal-src">{sourceLabel(sig.source)}</span>
      {!compact && (
        <>
          <span className="signal-arrow">→</span>
          <span>{formatRate(sig.output.sample_rate)}</span>
        </>
      )}
      <span className="signal-tag">{sig.bit_perfect ? "Bit-perfect" : sig.resampled ? "重采样" : sig.source.lossless ? "非直通" : "有损"}</span>
    </span>
  );
};

const SeekBar: React.FC = () => {
  const { status, seek } = usePlayer();
  const position = usePosition();
  const [drag, setDrag] = useState<number | null>(null);
  const duration = status.duration || 0;
  const value = drag ?? position;
  const pct = duration > 0 ? (value / duration) * 100 : 0;
  const commit = () => {
    if (drag !== null) {
      seek(drag);
      setDrag(null);
    }
  };
  return (
    <div className="seek-row">
      <span className="time">{formatTime(value)}</span>
      <input
        type="range"
        className="range seek"
        min={0}
        max={duration || 1}
        step={0.1}
        value={Math.min(value, duration || 1)}
        disabled={status.state === "stopped" && status.queue_index === undefined}
        style={{ ["--pct" as string]: `${pct}%` }}
        onChange={(e) => setDrag(Number(e.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
      />
      <span className="time">{formatTime(duration)}</span>
    </div>
  );
};

export const PlayerBar: React.FC = () => {
  const { status, current, control, setVolume, setMode } = usePlayer();
  const navigate = useNavigate();
  const [lastVolume, setLastVolume] = useState(1);
  const playing = status.state === "playing";
  const title = current?.name ?? fileStem(status.path);
  const subtitle = current ? [current.artist, current.album].filter(Boolean).join(" · ") : "";
  const slider = gainToSlider(status.volume);

  // 空格键播放/暂停（输入框内除外）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
      if (e.code === "Space") {
        e.preventDefault();
        control("toggle");
      } else if (e.code === "ArrowRight" && (e.metaKey || e.ctrlKey)) {
        control("next");
      } else if (e.code === "ArrowLeft" && (e.metaKey || e.ctrlKey)) {
        control("prev");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [control]);

  const nextRepeat = status.repeat === "off" ? "all" : status.repeat === "all" ? "one" : "off";
  const repeatTitle = { off: "顺序播放", all: "列表循环", one: "单曲循环" }[status.repeat];

  return (
    <footer className="player-bar">
      <div className="player-now" onClick={() => navigate("/now-playing")} title="打开正在播放">
        <Cover path={current?.cover_path} className="player-cover" emptyClassName="player-cover-empty" />
        <div className="player-meta">
          <div className="player-title">{title || "未在播放"}</div>
          <div className="player-sub">
            {status.error ? <span className="error-text">{status.error}</span> : subtitle || (title ? "" : "双击音乐库中的歌曲开始播放")}
          </div>
        </div>
      </div>

      <div className="player-center">
        <div className="player-controls">
          <button
            className={`icon-btn ${status.shuffle ? "is-on" : ""}`}
            title={status.shuffle ? "随机播放：开" : "随机播放：关"}
            onClick={() => setMode(status.repeat, !status.shuffle)}
          >
            <ShuffleIcon />
          </button>
          <button className="icon-btn" title="上一首（⌘←）" onClick={() => control("prev")}>
            <PrevIcon />
          </button>
          <button className="play-btn" title="播放/暂停（空格）" onClick={() => control("toggle")}>
            {playing ? <PauseIcon /> : <PlayIcon />}
          </button>
          <button className="icon-btn" title="下一首（⌘→）" onClick={() => control("next")}>
            <NextIcon />
          </button>
          <button
            className={`icon-btn ${status.repeat !== "off" ? "is-on" : ""}`}
            title={repeatTitle}
            onClick={() => setMode(nextRepeat, status.shuffle)}
          >
            <RepeatIcon one={status.repeat === "one"} />
          </button>
        </div>
        <SeekBar />
      </div>

      <div className="player-right">
        <SignalBadge compact />
        <div className="volume">
          <button
            className="icon-btn"
            title={status.volume > 0 ? "静音" : "取消静音"}
            onClick={() => {
              if (status.volume > 0) {
                setLastVolume(status.volume);
                setVolume(0);
              } else {
                setVolume(lastVolume || 1);
              }
            }}
          >
            <VolumeIcon level={status.volume === 0 ? 0 : slider} />
          </button>
          <input
            type="range"
            className="range volume-range"
            min={0}
            max={1}
            step={0.01}
            value={slider}
            style={{ ["--pct" as string]: `${slider * 100}%` }}
            title={status.volume >= 1 ? "音量 100%（不做数字音量处理）" : `音量 ${Math.round(slider * 100)}%`}
            onChange={(e) => setVolume(sliderToGain(Number(e.target.value)))}
          />
        </div>
      </div>
    </footer>
  );
};
