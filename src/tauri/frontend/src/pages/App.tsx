import React from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { Sidebar } from "../ui/Sidebar";
import { LibraryPage } from "./LibraryPage";
import { ImportPage } from "./ImportPage";
import { SettingsPage } from "./SettingsPage";

export const App: React.FC = () => {
  return (
    <div className="app-root">
      <Sidebar />
      <main className="app-main">
        <Routes>
          <Route path="/library" element={<LibraryPage />} />
          <Route path="/import" element={<ImportPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/library" replace />} />
        </Routes>
      </main>
    </div>
  );
};



