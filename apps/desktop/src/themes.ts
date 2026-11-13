// Dark themes. Each one colours the whole app *and* its terminals (background, text and
// the 16 ANSI colours), so the two always match. Status colours (Needs you / Running / Done
// / Error), the accent and agent monograms stay the same in every theme so their meaning
// never changes. The default theme's values match :root in styles.css.

export type ThemeId =
  | "termlab"
  | "midnight"
  | "graphite"
  | "oled"
  | "forest"
  | "plum"
  | "one-dark"
  | "tokyo-night"
  | "catppuccin"
  | "dracula"
  | "gruvbox"
  | "nord";

type Tokens = Record<
  | "bg"
  | "panel"
  | "card"
  | "raised"
  | "selected"
  | "terminal"
  | "border"
  | "border-strong"
  | "text"
  | "text-2"
  | "muted"
  | "faint"
  | "hover"
  | "row-hover"
  | "active"
  | "selected-border"
  | "outline-strong"
  | "dashed"
  | "focus-border"
  | "terminal-dim"
  | "hunk"
  | "divider"
  | "line-number"
  | "selection",
  string
>;


/** The 16 ANSI colours, in xterm.js's naming. */
export interface AnsiColors {
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

export interface Theme {
  id: ThemeId;
  name: string;
  description: string;
  /** The terminal's 16 colours; its background and text come from the tokens. */
  ansi: AnsiColors;
  tokens: Tokens;
}

/** Terminal colours for Termlab's own themes. */
const TERMLAB_ANSI: AnsiColors = {
  black: "#1d2026", red: "#f2716b", green: "#7fd17a", yellow: "#e8c065",
  blue: "#6aa8ff", magenta: "#c69bf5", cyan: "#5fcfd6", white: "#c9ced6",
  brightBlack: "#5c6370", brightRed: "#ff8a80", brightGreen: "#a5e5b0", brightYellow: "#ffd27a",
  brightBlue: "#a8ccff", brightMagenta: "#dcbcff", brightCyan: "#8fe6ec", brightWhite: "#f2f4f7",
};

export const THEMES: Theme[] = [
  {
    id: "termlab",
    name: "Termlab",
    description: "The default: near-black, neutral",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#0b0c0e", panel: "#0f1113", card: "#111316", raised: "#15181c", selected: "#1c1f24",
      terminal: "#08090a", border: "#23262c", "border-strong": "#2a2e35", text: "#e6e8eb",
      "text-2": "#c9ced6", muted: "#9aa1ac", faint: "#8b929c", hover: "#1e2127", "row-hover": "#181b20",
      active: "#262a31", "selected-border": "#2f343c", "outline-strong": "#3a3f47", dashed: "#343941",
      "focus-border": "#3a4250", "terminal-dim": "#0d0e10", hunk: "#10151d", divider: "#16191d",
      "line-number": "#5c6370", selection: "#2f343c",
    },
  },
  {
    id: "midnight",
    name: "Midnight",
    description: "Deep navy blue",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#0a0e17", panel: "#0d1320", card: "#111827", raised: "#152033", selected: "#1b2840",
      terminal: "#070b13", border: "#1f2a3e", "border-strong": "#28354d", text: "#e3e9f3",
      "text-2": "#c2cde0", muted: "#91a0b9", faint: "#7e8ca5", hover: "#1a2539", "row-hover": "#142035",
      active: "#22304a", "selected-border": "#2c3b58", "outline-strong": "#34456a", dashed: "#2f3d58",
      "focus-border": "#3b5280", "terminal-dim": "#0b1120", hunk: "#0f1a30", divider: "#131c2c",
      "line-number": "#5a6b88", selection: "#26365a",
    },
  },
  {
    id: "graphite",
    name: "Graphite",
    description: "Softer, lighter grey",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#18191b", panel: "#1d1e21", card: "#222327", raised: "#27292d", selected: "#2f3136",
      terminal: "#141517", border: "#34363b", "border-strong": "#3d4046", text: "#ececed",
      "text-2": "#d0d2d6", muted: "#a5a9b0", faint: "#91969d", hover: "#2c2e33", "row-hover": "#25272b",
      active: "#34373c", "selected-border": "#41444b", "outline-strong": "#4a4e55", dashed: "#45484f",
      "focus-border": "#566070", "terminal-dim": "#1a1b1d", hunk: "#1b2230", divider: "#24262a",
      "line-number": "#6b7079", selection: "#3d4048",
    },
  },
  {
    id: "oled",
    name: "Pure black",
    description: "True black, for OLED screens",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#000000", panel: "#060606", card: "#0b0b0b", raised: "#111111", selected: "#191919",
      terminal: "#000000", border: "#1d1d1d", "border-strong": "#262626", text: "#ededed",
      "text-2": "#cccccc", muted: "#9b9b9b", faint: "#878787", hover: "#1a1a1a", "row-hover": "#121212",
      active: "#222222", "selected-border": "#2c2c2c", "outline-strong": "#363636", dashed: "#2e2e2e",
      "focus-border": "#3f4a5c", "terminal-dim": "#070707", hunk: "#0a1220", divider: "#121212",
      "line-number": "#5a5a5a", selection: "#2a2a2a",
    },
  },
  {
    id: "forest",
    name: "Forest",
    description: "Dark green",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#0a0f0c", panel: "#0d1410", card: "#111914", raised: "#152019", selected: "#1b2a21",
      terminal: "#070b08", border: "#1e2c24", "border-strong": "#27382e", text: "#e2ece6",
      "text-2": "#c2d3c9", muted: "#92a69a", faint: "#7e9387", hover: "#192820", "row-hover": "#132019",
      active: "#21332a", "selected-border": "#2b4135", "outline-strong": "#334b3e", dashed: "#2e4337",
      "focus-border": "#3d6150", "terminal-dim": "#0a120d", hunk: "#0e1a1f", divider: "#121c16",
      "line-number": "#5a6f62", selection: "#25402f",
    },
  },
  {
    id: "plum",
    name: "Plum",
    description: "Dark purple",
    ansi: TERMLAB_ANSI,
    tokens: {
      bg: "#0e0b12", panel: "#130f18", card: "#17131d", raised: "#1d1725", selected: "#251d30",
      terminal: "#0a080d", border: "#2a2234", "border-strong": "#332a40", text: "#ebe6f1",
      "text-2": "#d0c8db", muted: "#a499b3", faint: "#8f849f", hover: "#231c2d", "row-hover": "#1c1625",
      active: "#2c2338", "selected-border": "#382c47", "outline-strong": "#433553", dashed: "#3b2f4a",
      "focus-border": "#5a4880", "terminal-dim": "#100c14", hunk: "#161428", divider: "#1a1521",
      "line-number": "#6b6079", selection: "#3a2d4c",
    },
  },
  {
    id: "one-dark",
    name: "One Dark",
    description: "Atom's classic, as in Zed",
    ansi: {
      black: "#3f4451", red: "#e05561", green: "#8cc265", yellow: "#d18f52",
      blue: "#4aa5f0", magenta: "#c162de", cyan: "#42b3c2", white: "#d7dae0",
      brightBlack: "#4f5666", brightRed: "#ff616e", brightGreen: "#a5e075", brightYellow: "#f0a45d",
      brightBlue: "#4dc4ff", brightMagenta: "#de73ff", brightCyan: "#4cd1e0", brightWhite: "#ffffff",
    },
    tokens: {
      bg: "#21252b", panel: "#1e2227", card: "#262a31", raised: "#2c313a", selected: "#323842",
      terminal: "#282c34", border: "#181a1f", "border-strong": "#3a3f4b", text: "#d7dae0",
      "text-2": "#abb2bf", muted: "#9199a6", faint: "#848c99", hover: "#2c313a", "row-hover": "#282c34",
      active: "#353b45", "selected-border": "#3e4451", "outline-strong": "#4b5263", dashed: "#3e4451",
      "focus-border": "#4d78cc", "terminal-dim": "#23272e", hunk: "#2a3245", divider: "#2a2f37",
      "line-number": "#636d83", selection: "#3e4451",
    },
  },
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    description: "Neon blues and purples",
    ansi: {
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#f7768e", brightGreen: "#9ece6a", brightYellow: "#e0af68",
      brightBlue: "#7aa2f7", brightMagenta: "#bb9af7", brightCyan: "#7dcfff", brightWhite: "#c0caf5",
    },
    tokens: {
      bg: "#16161e", panel: "#13131a", card: "#1f2030", raised: "#24283b", selected: "#292e42",
      terminal: "#1a1b26", border: "#101014", "border-strong": "#2f334d", text: "#c0caf5",
      "text-2": "#a9b1d6", muted: "#8a92bb", faint: "#7a82a8", hover: "#232433", "row-hover": "#1e1f2b",
      active: "#2a2e42", "selected-border": "#3b4261", "outline-strong": "#414868", dashed: "#3b4261",
      "focus-border": "#3d59a1", "terminal-dim": "#16161e", hunk: "#1f2a48", divider: "#1f2030",
      "line-number": "#545c7e", selection: "#33467c",
    },
  },
  {
    id: "catppuccin",
    name: "Catppuccin Mocha",
    description: "Soft pastels",
    ansi: {
      black: "#45475a", red: "#f38ba8", green: "#a6e3a1", yellow: "#f9e2af",
      blue: "#89b4fa", magenta: "#f5c2e7", cyan: "#94e2d5", white: "#bac2de",
      brightBlack: "#585b70", brightRed: "#f38ba8", brightGreen: "#a6e3a1", brightYellow: "#f9e2af",
      brightBlue: "#89b4fa", brightMagenta: "#f5c2e7", brightCyan: "#94e2d5", brightWhite: "#a6adc8",
    },
    tokens: {
      bg: "#181825", panel: "#11111b", card: "#1e1e2e", raised: "#232334", selected: "#313244",
      terminal: "#1e1e2e", border: "#26273a", "border-strong": "#313244", text: "#cdd6f4",
      "text-2": "#bac2de", muted: "#a6adc8", faint: "#9399b2", hover: "#2a2b3c", "row-hover": "#232334",
      active: "#313244", "selected-border": "#45475a", "outline-strong": "#585b70", dashed: "#45475a",
      "focus-border": "#5a76b0", "terminal-dim": "#181825", hunk: "#1f2740", divider: "#232334",
      "line-number": "#6c7086", selection: "#45475a",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    description: "Vivid, high contrast",
    ansi: {
      black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c",
      blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2",
      brightBlack: "#6272a4", brightRed: "#ff6e6e", brightGreen: "#69ff94", brightYellow: "#ffffa5",
      brightBlue: "#d6acff", brightMagenta: "#ff92df", brightCyan: "#a4ffff", brightWhite: "#ffffff",
    },
    tokens: {
      bg: "#21222c", panel: "#191a21", card: "#282a36", raised: "#2d2f3d", selected: "#343746",
      terminal: "#282a36", border: "#2b2d3a", "border-strong": "#3c3f51", text: "#f8f8f2",
      "text-2": "#e2e2dc", muted: "#a7abbe", faint: "#959ab5", hover: "#303241", "row-hover": "#2a2c39",
      active: "#3a3d4e", "selected-border": "#44475a", "outline-strong": "#545869", dashed: "#44475a",
      "focus-border": "#6272a4", "terminal-dim": "#23242f", hunk: "#2c3350", divider: "#2b2d3a",
      "line-number": "#6272a4", selection: "#44475a",
    },
  },
  {
    id: "gruvbox",
    name: "Gruvbox Dark",
    description: "Warm, retro browns",
    ansi: {
      black: "#282828", red: "#cc241d", green: "#98971a", yellow: "#d79921",
      blue: "#458588", magenta: "#b16286", cyan: "#689d6a", white: "#a89984",
      brightBlack: "#928374", brightRed: "#fb4934", brightGreen: "#b8bb26", brightYellow: "#fabd2f",
      brightBlue: "#83a598", brightMagenta: "#d3869b", brightCyan: "#8ec07c", brightWhite: "#ebdbb2",
    },
    tokens: {
      bg: "#1d2021", panel: "#191b1c", card: "#282828", raised: "#2e2c2b", selected: "#3c3836",
      terminal: "#282828", border: "#32302f", "border-strong": "#3c3836", text: "#ebdbb2",
      "text-2": "#d5c4a1", muted: "#bdae93", faint: "#a89984", hover: "#32302f", "row-hover": "#2a2827",
      active: "#3c3836", "selected-border": "#504945", "outline-strong": "#665c54", dashed: "#504945",
      "focus-border": "#4f6f6d", "terminal-dim": "#222222", hunk: "#253033", divider: "#2a2827",
      "line-number": "#7c6f64", selection: "#504945",
    },
  },
  {
    id: "nord",
    name: "Nord",
    description: "Cool arctic blue-grey",
    ansi: {
      black: "#3b4252", red: "#bf616a", green: "#a3be8c", yellow: "#ebcb8b",
      blue: "#81a1c1", magenta: "#b48ead", cyan: "#88c0d0", white: "#e5e9f0",
      brightBlack: "#4c566a", brightRed: "#bf616a", brightGreen: "#a3be8c", brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1", brightMagenta: "#b48ead", brightCyan: "#8fbcbb", brightWhite: "#eceff4",
    },
    tokens: {
      bg: "#2a2f3a", panel: "#262b35", card: "#2e3440", raised: "#343a47", selected: "#3b4252",
      terminal: "#2e3440", border: "#232831", "border-strong": "#3b4252", text: "#eceff4",
      "text-2": "#d8dee9", muted: "#a9b1c2", faint: "#98a1b3", hover: "#353b49", "row-hover": "#303643",
      active: "#3b4252", "selected-border": "#434c5e", "outline-strong": "#4c566a", dashed: "#434c5e",
      "focus-border": "#5e81ac", "terminal-dim": "#2a2f3a", hunk: "#2f3a4f", divider: "#333a47",
      "line-number": "#616e88", selection: "#434c5e",
    },
  },
];

const THEME_KEY = "termlab.theme";
/** Set by an earlier version that chose terminal colours separately. */
const LEGACY_TERMINAL_KEY = "termlab.terminalScheme";
export const THEME_EVENT = "termlab-theme";

export function themeById(id: string | null): Theme | undefined {
  return THEMES.find((t) => t.id === id);
}

/** The chosen theme (a per-viewer preference, kept in localStorage). */
export function currentTheme(): Theme {
  try {
    return (
      themeById(localStorage.getItem(THEME_KEY)) ??
      themeById(localStorage.getItem(LEGACY_TERMINAL_KEY)) ??
      THEMES[0]
    );
  } catch {
    return THEMES[0];
  }
}

/** Sets the theme's tokens on the document and tells the terminals to repaint. */
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  for (const [name, value] of Object.entries(theme.tokens)) {
    root.style.setProperty(`--${name}`, value);
  }
  root.dataset.theme = theme.id;
  window.dispatchEvent(new Event(THEME_EVENT));
}

export function chooseTheme(id: ThemeId) {
  const theme = themeById(id) ?? THEMES[0];
  try {
    localStorage.setItem(THEME_KEY, theme.id);
    localStorage.removeItem(LEGACY_TERMINAL_KEY);
  } catch {
    // Storage unavailable: the theme still applies to this window.
  }
  applyTheme(theme);
}

/** The colours an xterm.js terminal should use for the current theme. */
export function terminalTheme() {
  const { ansi, tokens: t } = currentTheme();
  return {
    ...ansi,
    background: t.terminal,
    foreground: t.text,
    cursor: t.text,
    cursorAccent: t.terminal,
    selectionBackground: t.selection,
  };
}
