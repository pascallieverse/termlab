/**
 * Keys for the screen viewer: which browser key codes to send for a typed character (when a
 * modifier is held from the on-screen key bar, so ⌘ + "c" is a shortcut, not text), the
 * Ctrl ⇄ ⌘ swap between a PC and a Mac (either way round), and per-system modifier labels
 * and shortcuts. The computer's system comes from the session's hello (`os`).
 */

const PUNCT: Record<string, [string, boolean]> = {
    ' ': ['Space', false],
    '-': ['Minus', false],
    _: ['Minus', true],
    '=': ['Equal', false],
    '+': ['Equal', true],
    '[': ['BracketLeft', false],
    '{': ['BracketLeft', true],
    ']': ['BracketRight', false],
    '}': ['BracketRight', true],
    '\\': ['Backslash', false],
    '|': ['Backslash', true],
    ';': ['Semicolon', false],
    ':': ['Semicolon', true],
    "'": ['Quote', false],
    '"': ['Quote', true],
    ',': ['Comma', false],
    '<': ['Comma', true],
    '.': ['Period', false],
    '>': ['Period', true],
    '/': ['Slash', false],
    '?': ['Slash', true],
    '`': ['Backquote', false],
    '~': ['Backquote', true],
    '\n': ['Enter', false],
    '\t': ['Tab', false],
};

const SHIFTED_DIGITS = ')!@#$%^&*(';

/** The key (and whether Shift is needed) for a character on a US layout. */
export function codeForChar(ch: string): { code: string; shift: boolean } | null {
    if (/^[a-z]$/.test(ch)) return { code: `Key${ch.toUpperCase()}`, shift: false };
    if (/^[A-Z]$/.test(ch)) return { code: `Key${ch}`, shift: true };
    if (/^[0-9]$/.test(ch)) return { code: `Digit${ch}`, shift: false };
    const shifted = SHIFTED_DIGITS.indexOf(ch);
    if (shifted >= 0) return { code: `Digit${shifted}`, shift: true };
    const p = PUNCT[ch];

    return p ? { code: p[0], shift: p[1] } : null;
}

const SWAP: Record<string, string> = {
    ControlLeft: 'MetaLeft',
    ControlRight: 'MetaRight',
    MetaLeft: 'ControlLeft',
    MetaRight: 'ControlRight',
};

/** With the swap on, Ctrl and ⌘ trade places: a PC's Ctrl acts as a Mac's ⌘, or a Mac's ⌘ as a PC's Ctrl. */
export function mapCode(code: string, swapCtrlCmd: boolean): string {
    return swapCtrlCmd ? (SWAP[code] ?? code) : code;
}

export function isMacBrowser(): boolean {
    return /Mac|iPhone|iPad/.test(navigator.platform) || /Mac OS X/.test(navigator.userAgent);
}

/** The computer runs macOS (older Termlabs only shared Macs, so no system means a Mac). */
export function isMacTarget(os: string | undefined): boolean {
    return !os || os === 'macos';
}

/** Swap Ctrl and ⌘ when the browser's computer and the shared one differ. */
export function defaultSwap(os: string | undefined): boolean {
    return isMacBrowser() !== isMacTarget(os);
}

type Shortcut = { label: string; mods: string[]; code: string };

/** Shortcuts that are hard to press in a browser (the browser or the OS takes them), per system. */
export function shortcuts(os: string | undefined): Shortcut[] {
    if (os === 'windows') return WINDOWS_SHORTCUTS;
    if (os === 'linux') return LINUX_SHORTCUTS;

    return SHORTCUTS;
}

/** The on-screen modifier keys, labelled the way the shared computer's keyboard is. */
export function modifiers(os: string | undefined): { code: string; label: string }[] {
    if (isMacTarget(os)) {
        return [
            { code: 'MetaLeft', label: '⌘' },
            { code: 'AltLeft', label: '⌥' },
            { code: 'ControlLeft', label: '⌃' },
            { code: 'ShiftLeft', label: '⇧' },
        ];
    }

    return [
        { code: 'ControlLeft', label: 'Ctrl' },
        { code: 'AltLeft', label: 'Alt' },
        { code: 'MetaLeft', label: os === 'windows' ? 'Win' : 'Super' },
        { code: 'ShiftLeft', label: '⇧' },
    ];
}

// Windows itself takes Ctrl+Alt+Del and Win+L, so they can't be sent.
const WINDOWS_SHORTCUTS: Shortcut[] = [
    { label: 'Start menu (Win)', mods: [], code: 'MetaLeft' },
    { label: 'Switch app (Alt Tab)', mods: ['AltLeft'], code: 'Tab' },
    { label: 'Task view (Win Tab)', mods: ['MetaLeft'], code: 'Tab' },
    { label: 'Close window (Alt F4)', mods: ['AltLeft'], code: 'F4' },
    { label: 'Show desktop (Win D)', mods: ['MetaLeft'], code: 'KeyD' },
    { label: 'File Explorer (Win E)', mods: ['MetaLeft'], code: 'KeyE' },
    { label: 'Task Manager (Ctrl Shift Esc)', mods: ['ControlLeft', 'ShiftLeft'], code: 'Escape' },
    { label: 'Screenshot (Win Shift S)', mods: ['MetaLeft', 'ShiftLeft'], code: 'KeyS' },
];

const LINUX_SHORTCUTS: Shortcut[] = [
    { label: 'Activities (Super)', mods: [], code: 'MetaLeft' },
    { label: 'Switch app (Alt Tab)', mods: ['AltLeft'], code: 'Tab' },
    { label: 'Close window (Alt F4)', mods: ['AltLeft'], code: 'F4' },
    { label: 'Terminal (Ctrl Alt T)', mods: ['ControlLeft', 'AltLeft'], code: 'KeyT' },
    { label: 'Lock screen (Super L)', mods: ['MetaLeft'], code: 'KeyL' },
];

const SHORTCUTS: Shortcut[] = [
    { label: 'Spotlight (⌘ Space)', mods: ['MetaLeft'], code: 'Space' },
    { label: 'Switch app (⌘ Tab)', mods: ['MetaLeft'], code: 'Tab' },
    { label: 'Mission Control (⌃ ↑)', mods: ['ControlLeft'], code: 'ArrowUp' },
    { label: 'Close window (⌘ W)', mods: ['MetaLeft'], code: 'KeyW' },
    { label: 'Quit app (⌘ Q)', mods: ['MetaLeft'], code: 'KeyQ' },
    { label: 'Force Quit (⌥ ⌘ Esc)', mods: ['AltLeft', 'MetaLeft'], code: 'Escape' },
    { label: 'Screenshot (⇧ ⌘ 5)', mods: ['ShiftLeft', 'MetaLeft'], code: 'Digit5' },
];
