import React, { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { NoteIcon } from "./icons";

/** 封面：没有封面或加载失败（文件被删、无权限）时显示占位图。 */
export const Cover: React.FC<{ path?: string; className: string; emptyClassName: string; iconSize?: number }> = ({
  path,
  className,
  emptyClassName,
  iconSize = 20,
}) => {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [path]);
  if (!path || failed) {
    return (
      <div className={`${className} ${emptyClassName}`}>
        <NoteIcon size={iconSize} />
      </div>
    );
  }
  return <img className={className} src={convertFileSrc(path)} alt="" onError={() => setFailed(true)} />;
};
