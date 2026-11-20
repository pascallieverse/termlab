import type { ITheme } from '@xterm/xterm';
import { useSyncExternalStore } from 'react';
import { UI_THEMES } from './ui-themes';

/*
 * How this browser shows things: the terminal's colours, text and cursor. A per-viewer
 * preference (like the desktop app's), kept in localStorage; it never reaches the server.
 */

export type TerminalThemeId = string;

/**
 * The desktop app's themes (apps/desktop/src/themes.ts), terminal colours only: its
 * terminal background, text, selection and 16 ANSI colours. 'default' is its "Termlab" theme.
 */
export const TERMINAL_THEMES: Record<TerminalThemeId, { name: string; description: string; theme: ITheme }> = {
    default: {
        name: 'Termlab',
        description: 'The default: near-black, neutral',
        theme: {
            background: '#08090a',
            foreground: '#e6e8eb',
            cursor: '#e6e8eb',
            cursorAccent: '#08090a',
            selectionBackground: '#2f343c',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    midnight: {
        name: 'Midnight',
        description: 'Deep navy blue',
        theme: {
            background: '#070b13',
            foreground: '#e3e9f3',
            cursor: '#e3e9f3',
            cursorAccent: '#070b13',
            selectionBackground: '#26365a',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    graphite: {
        name: 'Graphite',
        description: 'Softer, lighter grey',
        theme: {
            background: '#141517',
            foreground: '#ececed',
            cursor: '#ececed',
            cursorAccent: '#141517',
            selectionBackground: '#3d4048',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    oled: {
        name: 'Pure black',
        description: 'True black, for OLED screens',
        theme: {
            background: '#000000',
            foreground: '#ededed',
            cursor: '#ededed',
            cursorAccent: '#000000',
            selectionBackground: '#2a2a2a',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    forest: {
        name: 'Forest',
        description: 'Dark green',
        theme: {
            background: '#070b08',
            foreground: '#e2ece6',
            cursor: '#e2ece6',
            cursorAccent: '#070b08',
            selectionBackground: '#25402f',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    plum: {
        name: 'Plum',
        description: 'Dark purple',
        theme: {
            background: '#0a080d',
            foreground: '#ebe6f1',
            cursor: '#ebe6f1',
            cursorAccent: '#0a080d',
            selectionBackground: '#3a2d4c',
            black: '#1d2026',
            red: '#f2716b',
            green: '#7fd17a',
            yellow: '#e8c065',
            blue: '#6aa8ff',
            magenta: '#c69bf5',
            cyan: '#5fcfd6',
            white: '#c9ced6',
            brightBlack: '#5c6370',
            brightRed: '#ff8a80',
            brightGreen: '#a5e5b0',
            brightYellow: '#ffd27a',
            brightBlue: '#a8ccff',
            brightMagenta: '#dcbcff',
            brightCyan: '#8fe6ec',
            brightWhite: '#f2f4f7',
        },
    },
    'one-dark': {
        name: 'One Dark',
        description: "Atom's classic, as in Zed",
        theme: {
            background: '#282c34',
            foreground: '#d7dae0',
            cursor: '#d7dae0',
            cursorAccent: '#282c34',
            selectionBackground: '#3e4451',
            black: '#3f4451',
            red: '#e05561',
            green: '#8cc265',
            yellow: '#d18f52',
            blue: '#4aa5f0',
            magenta: '#c162de',
            cyan: '#42b3c2',
            white: '#d7dae0',
            brightBlack: '#4f5666',
            brightRed: '#ff616e',
            brightGreen: '#a5e075',
            brightYellow: '#f0a45d',
            brightBlue: '#4dc4ff',
            brightMagenta: '#de73ff',
            brightCyan: '#4cd1e0',
            brightWhite: '#ffffff',
        },
    },
    'tokyo-night': {
        name: 'Tokyo Night',
        description: 'Neon blues and purples',
        theme: {
            background: '#1a1b26',
            foreground: '#c0caf5',
            cursor: '#c0caf5',
            cursorAccent: '#1a1b26',
            selectionBackground: '#33467c',
            black: '#15161e',
            red: '#f7768e',
            green: '#9ece6a',
            yellow: '#e0af68',
            blue: '#7aa2f7',
            magenta: '#bb9af7',
            cyan: '#7dcfff',
            white: '#a9b1d6',
            brightBlack: '#414868',
            brightRed: '#f7768e',
            brightGreen: '#9ece6a',
            brightYellow: '#e0af68',
            brightBlue: '#7aa2f7',
            brightMagenta: '#bb9af7',
            brightCyan: '#7dcfff',
            brightWhite: '#c0caf5',
        },
    },
    catppuccin: {
        name: 'Catppuccin Mocha',
        description: 'Soft pastels',
        theme: {
            background: '#1e1e2e',
            foreground: '#cdd6f4',
            cursor: '#cdd6f4',
            cursorAccent: '#1e1e2e',
            selectionBackground: '#45475a',
            black: '#45475a',
            red: '#f38ba8',
            green: '#a6e3a1',
            yellow: '#f9e2af',
            blue: '#89b4fa',
            magenta: '#f5c2e7',
            cyan: '#94e2d5',
            white: '#bac2de',
            brightBlack: '#585b70',
            brightRed: '#f38ba8',
            brightGreen: '#a6e3a1',
            brightYellow: '#f9e2af',
            brightBlue: '#89b4fa',
            brightMagenta: '#f5c2e7',
            brightCyan: '#94e2d5',
            brightWhite: '#a6adc8',
        },
    },
    dracula: {
        name: 'Dracula',
        description: 'Vivid, high contrast',
        theme: {
            background: '#282a36',
            foreground: '#f8f8f2',
            cursor: '#f8f8f2',
            cursorAccent: '#282a36',
            selectionBackground: '#44475a',
            black: '#21222c',
            red: '#ff5555',
            green: '#50fa7b',
            yellow: '#f1fa8c',
            blue: '#bd93f9',
            magenta: '#ff79c6',
            cyan: '#8be9fd',
            white: '#f8f8f2',
            brightBlack: '#6272a4',
            brightRed: '#ff6e6e',
            brightGreen: '#69ff94',
            brightYellow: '#ffffa5',
            brightBlue: '#d6acff',
            brightMagenta: '#ff92df',
            brightCyan: '#a4ffff',
            brightWhite: '#ffffff',
        },
    },
    gruvbox: {
        name: 'Gruvbox Dark',
        description: 'Warm, retro browns',
        theme: {
            background: '#282828',
            foreground: '#ebdbb2',
            cursor: '#ebdbb2',
            cursorAccent: '#282828',
            selectionBackground: '#504945',
            black: '#282828',
            red: '#cc241d',
            green: '#98971a',
            yellow: '#d79921',
            blue: '#458588',
            magenta: '#b16286',
            cyan: '#689d6a',
            white: '#a89984',
            brightBlack: '#928374',
            brightRed: '#fb4934',
            brightGreen: '#b8bb26',
            brightYellow: '#fabd2f',
            brightBlue: '#83a598',
            brightMagenta: '#d3869b',
            brightCyan: '#8ec07c',
            brightWhite: '#ebdbb2',
        },
    },
    nord: {
        name: 'Nord',
        description: 'Cool arctic blue-grey',
        theme: {
            background: '#2e3440',
            foreground: '#eceff4',
            cursor: '#eceff4',
            cursorAccent: '#2e3440',
            selectionBackground: '#434c5e',
            black: '#3b4252',
            red: '#bf616a',
            green: '#a3be8c',
            yellow: '#ebcb8b',
            blue: '#81a1c1',
            magenta: '#b48ead',
            cyan: '#88c0d0',
            white: '#e5e9f0',
            brightBlack: '#4c566a',
            brightRed: '#bf616a',
            brightGreen: '#a3be8c',
            brightYellow: '#ebcb8b',
            brightBlue: '#81a1c1',
            brightMagenta: '#b48ead',
            brightCyan: '#8fbcbb',
            brightWhite: '#eceff4',
        },
    },};

export type Appearance = {
    /** 9–20 px. */
    terminalFontSize: number;
    /** 1.0–1.6. */
    terminalLineHeight: number;
    terminalTheme: TerminalThemeId;
    cursorBlink: boolean;
    /** Shrink the font so the session's full width fits the view (with `terminalSize: 'app'`). */
    fitWidth: boolean;
    /**
     * 'browser': while you watch a session here, its terminal takes this browser's size (the
     * agent redraws for it; Termlab's window shows the smaller screen until it takes the size
     * back). 'app': keep the size of Termlab's window.
     */
    terminalSize: 'browser' | 'app';
};

export const DEFAULT_APPEARANCE: Appearance = {
    terminalFontSize: 13,
    terminalLineHeight: 1.15,
    terminalTheme: 'default',
    cursorBlink: true,
    fitWidth: true,
    terminalSize: 'browser',
};

export const FONT_SIZE_RANGE = { min: 9, max: 20 } as const;

/** The desktop app's line spacing choices. */
export const LINE_HEIGHTS = [
    { value: 1, label: 'Compact' },
    { value: 1.15, label: 'Normal' },
    { value: 1.3, label: 'Roomy' },
] as const;

const KEY = 'termlab-server.appearance';

function clean(raw: unknown): Appearance {
    const p = (raw && typeof raw === 'object' ? raw : {}) as Partial<Record<keyof Appearance, unknown>>;
    const d = DEFAULT_APPEARANCE;
    const num = (v: unknown, min: number, max: number, fallback: number) =>
        typeof v === 'number' && Number.isFinite(v) ? Math.min(max, Math.max(min, v)) : fallback;

    return {
        terminalFontSize: Math.round(num(p.terminalFontSize, FONT_SIZE_RANGE.min, FONT_SIZE_RANGE.max, d.terminalFontSize)),
        terminalLineHeight: num(p.terminalLineHeight, 1, 1.6, d.terminalLineHeight),
        terminalTheme: typeof p.terminalTheme === 'string' && p.terminalTheme in TERMINAL_THEMES ? p.terminalTheme : d.terminalTheme,
        cursorBlink: typeof p.cursorBlink === 'boolean' ? p.cursorBlink : d.cursorBlink,
        fitWidth: typeof p.fitWidth === 'boolean' ? p.fitWidth : d.fitWidth,
        terminalSize: p.terminalSize === 'app' ? 'app' : 'browser',
    };
}

function load(): Appearance {
    try {
        const raw = localStorage.getItem(KEY);

        return clean(raw ? JSON.parse(raw) : null);
    } catch {
        return DEFAULT_APPEARANCE;
    }
}

/**
 * A theme colours the whole interface and its terminals together, as in the desktop app: its
 * colours replace the --color-* tokens on the page.
 */
function applyTheme(a: Appearance) {
    const tokens = UI_THEMES[a.terminalTheme] ?? UI_THEMES.default;
    const root = document.documentElement;
    for (const [name, value] of Object.entries(tokens)) {
        root.style.setProperty(`--color-${name}`, value);
    }
    root.dataset.theme = a.terminalTheme;
    document.querySelector('meta[name="theme-color"]')?.setAttribute('content', tokens.surface);
}

let current: Appearance = load();
applyTheme(current);
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
    listeners.add(listener);
    // Another tab changed it.
    const onStorage = (e: StorageEvent) => {
        if (e.key === KEY) {
            current = load();
            applyTheme(current);
            listener();
        }
    };
    window.addEventListener('storage', onStorage);

    return () => {
        listeners.delete(listener);
        window.removeEventListener('storage', onStorage);
    };
}

/** The current appearance; re-renders when it changes (in this tab or another). */
export function useAppearance(): Appearance {
    return useSyncExternalStore(subscribe, () => current, () => DEFAULT_APPEARANCE);
}

export function setAppearance(patch: Partial<Appearance>): void {
    current = clean({ ...current, ...patch });
    try {
        localStorage.setItem(KEY, JSON.stringify(current));
    } catch {
        // Storage unavailable: it still applies in this tab.
    }
    applyTheme(current);
    listeners.forEach((l) => l());
}
