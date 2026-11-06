import React from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { Sidebar } from "../ui/Sidebar";
import { LibraryPage } from "./LibraryPage";
import { ImportPage } from "./ImportPage";
import { SettingsPage } from "./SettingsPage";
import { NowPlayingPage } from "./NowPlayingPage";
import { PlayerProvider } from "../player/PlayerContext";
import { PlayerBar } from "../player/PlayerBar";

export const App: React.FC = () => {
  return (
    <PlayerProvider>
      <div className="app-root">
        <div className="app-body">
          <Sidebar />
          <main className="app-main">
            <Routes>
              <Route path="/library" element={<LibraryPage />} />
              <Route path="/now-playing" element={<NowPlayingPage />} />
              <Route path="/import" element={<ImportPage />} />
              <Route path="/settings" element={<SettingsPage />} />
              <Route path="*" element={<Navigate to="/library" replace />} />
            </Routes>
          </main>
        </div>
        <PlayerBar />
      </div>
    </PlayerProvider>
  );
};
