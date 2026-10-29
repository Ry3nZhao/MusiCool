import React from "react";
import { NavLink } from "react-router-dom";

export const Sidebar: React.FC = () => {
  return (
    <aside className="sidebar">
      <div className="sidebar-header">Music_Cool</div>
      <nav className="sidebar-nav">
        <NavLink to="/library" className="sidebar-item">
          音乐库
        </NavLink>
        <NavLink to="/import" className="sidebar-item">
          导入
        </NavLink>
        <NavLink to="/settings" className="sidebar-item">
          设置
        </NavLink>
      </nav>
    </aside>
  );
};



