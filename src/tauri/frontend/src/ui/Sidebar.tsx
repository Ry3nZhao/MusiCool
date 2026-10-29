import React from "react";
import { NavLink } from "react-router-dom";

export const Sidebar: React.FC = () => {
  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <img src="/logo.svg" alt="" className="sidebar-logo" />
        <span>
          <span className="brand-music">Music</span>
          <span className="brand-cool">Cool</span>
        </span>
      </div>
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



