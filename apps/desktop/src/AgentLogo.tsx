import { useId } from "react";
import type { AgentId } from "./api";

// Small marks for each agent's maker, drawn here as simplified shapes in the spirit of each
// brand's logo (Claude's burst, OpenAI's knot, Mistral's pixel M, Google's Gemini spark).
// They identify whose agent a session runs; they are not the official artwork.

const MISTRAL_ROWS: { color: string; cells: number[] }[] = [
  { color: "#FFD800", cells: [0, 4] },
  { color: "#FFAF00", cells: [0, 1, 3, 4] },
  { color: "#FF8205", cells: [0, 1, 2, 3, 4] },
  { color: "#FA500F", cells: [0, 2, 4] },
  { color: "#E10500", cells: [0, 4] },
];

export function AgentLogo({ agent, size = 14 }: { agent: AgentId; size?: number }) {
  const gradient = useId();
  const common = { width: size, height: size, viewBox: "0 0 24 24", "aria-hidden": true } as const;
  switch (agent) {
    case "claude":
      return (
        <svg {...common} fill="none" stroke="#D97757" strokeWidth="2.3" strokeLinecap="round">
          <path d="M12 9.8V2.4M13.1 10.09l2.7-4.67M13.91 10.9l6.4-3.7M14.2 12h5.4M13.91 13.1l6.4 3.7M13.1 13.91l2.7 4.67M12 14.2v7.4M10.9 13.91l-2.7 4.67M10.09 13.1l-6.4 3.7M9.8 12H4.4M10.09 10.9l-6.4-3.7M10.9 10.09 8.2 5.42" />
        </svg>
      );
    case "codex":
      // Six interlocking strokes around a hexagon.
      return (
        <svg {...common} fill="none" stroke="#E8EAED" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
          {[0, 60, 120, 180, 240, 300].map((turn) => (
            <path key={turn} transform={`rotate(${turn} 12 12)`} d="M9.3 10.45V5.5a3.75 3.75 0 0 1 6.9-2.05" />
          ))}
          <path d="M12 8.9l2.7 1.55v3.1L12 15.1l-2.7-1.55v-3.1z" strokeWidth="1.3" />
        </svg>
      );
    case "vibe":
      return (
        <svg {...common}>
          {MISTRAL_ROWS.map((row, y) =>
            row.cells.map((x) => (
              <rect key={`${x}-${y}`} x={2 + x * 4} y={2 + y * 4} width="4.1" height="4.1" fill={row.color} />
            )),
          )}
        </svg>
      );
    case "antigravity":
      return (
        <svg {...common}>
          <defs>
            <linearGradient id={gradient} x1="3" y1="20" x2="21" y2="4" gradientUnits="userSpaceOnUse">
              <stop offset="0" stopColor="#4285F4" />
              <stop offset="0.55" stopColor="#9B72CB" />
              <stop offset="1" stopColor="#D96570" />
            </linearGradient>
          </defs>
          <path
            fill={`url(#${gradient})`}
            d="M12 2c.5 5.4 4.6 9.5 10 10-5.4.5-9.5 4.6-10 10-.5-5.4-4.6-9.5-10-10 5.4-.5 9.5-4.6 10-10z"
          />
        </svg>
      );
  }
}
