import React from "react";

type P = { size?: number };
const svg = (size: number, children: React.ReactNode) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
    {children}
  </svg>
);

export const PlayIcon = ({ size = 20 }: P) => svg(size, <path d="M8 5.14v13.72a1 1 0 0 0 1.5.86l11.04-6.86a1 1 0 0 0 0-1.72L9.5 4.28A1 1 0 0 0 8 5.14z" />);
export const PauseIcon = ({ size = 20 }: P) =>
  svg(size, <path d="M7 4h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm7 0h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z" />);
export const NextIcon = ({ size = 18 }: P) => svg(size, <path d="M5 5.5v13a1 1 0 0 0 1.53.85L16 13.2V18a1 1 0 0 0 2 0V6a1 1 0 0 0-2 0v4.8L6.53 4.65A1 1 0 0 0 5 5.5z" />);
export const PrevIcon = ({ size = 18 }: P) => svg(size, <path d="M19 5.5v13a1 1 0 0 1-1.53.85L8 13.2V18a1 1 0 0 1-2 0V6a1 1 0 0 1 2 0v4.8l9.47-6.15A1 1 0 0 1 19 5.5z" />);
export const ShuffleIcon = ({ size = 16 }: P) =>
  svg(
    size,
    <path d="M17 3l4 4-4 4V8h-1.6a3 3 0 0 0-2.4 1.2l-1.5 2-1.25-1.67 1.15-1.53A5 5 0 0 1 15.4 6H17V3zM3 6h3.6a5 5 0 0 1 4 2l5.4 7.2a3 3 0 0 0 2.4 1.2H17v-3l4 4-4 4v-3h-1.6a5 5 0 0 1-4-2L6 9.2A3 3 0 0 0 3.6 8H3V6zm0 10h.6a3 3 0 0 0 2.4-1.2l1.5-2 1.25 1.67-1.15 1.53A5 5 0 0 1 3.6 18H3v-2z" />
  );
export const RepeatIcon = ({ size = 16, one = false }: P & { one?: boolean }) =>
  svg(
    size,
    <>
      <path d="M17 1l4 4-4 4V6H7a2 2 0 0 0-2 2v3H3V8a4 4 0 0 1 4-4h10V1zM7 23l-4-4 4-4v3h10a2 2 0 0 0 2-2v-3h2v3a4 4 0 0 1-4 4H7v3z" />
      {one && <text x="12" y="15.5" fontSize="8" fontWeight="700" textAnchor="middle">1</text>}
    </>
  );
export const VolumeIcon = ({ size = 16, level = 1 }: P & { level?: number }) =>
  svg(
    size,
    <>
      <path d="M4 9h3l5-4v14l-5-4H4a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z" />
      {level > 0 && <path d="M15.5 8.5a5 5 0 0 1 0 7" stroke="currentColor" strokeWidth="2" fill="none" strokeLinecap="round" />}
      {level > 0.5 && <path d="M18.5 5.5a9 9 0 0 1 0 13" stroke="currentColor" strokeWidth="2" fill="none" strokeLinecap="round" />}
      {level === 0 && <path d="M16 9l5 6m0-6l-5 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />}
    </>
  );
export const NoteIcon = ({ size = 20 }: P) => svg(size, <path d="M9 18.5A2.5 2.5 0 1 1 6.5 16c.17 0 .34.02.5.05V5.6a1 1 0 0 1 .76-.97l10-2.5A1 1 0 0 1 19 3.1V15.5A2.5 2.5 0 1 1 16.5 13c.17 0 .34.02.5.05V6.28l-8 2V18.5z" />);
