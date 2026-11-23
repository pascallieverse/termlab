/**
 * Keys for the screen viewer: which browser key codes to send for a typed character (when a
 * modifier is held from the on-screen key bar, so ⌘ + "c" is a shortcut, not text), and the
 * Ctrl ⇄ ⌘ swap for people on a PC controlling a Mac.
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

/** With the swap on, the PC's Ctrl acts as the Mac's ⌘ (and the Windows key as Ctrl). */
export function mapCode(code: string, swapCtrlCmd: boolean): string {
    return swapCtrlCmd ? (SWAP[code] ?? code) : code;
}

export function isMacBrowser(): boolean {
    return /Mac|iPhone|iPad/.test(navigator.platform) || /Mac OS X/.test(navigator.userAgent);
}

/** Shortcuts that are hard to press in a browser (the browser or the OS takes them). */
export const SHORTCUTS: { label: string; mods: string[]; code: string }[] = [
    { label: 'Spotlight (⌘ Space)', mods: ['MetaLeft'], code: 'Space' },
    { label: 'Switch app (⌘ Tab)', mods: ['MetaLeft'], code: 'Tab' },
    { label: 'Mission Control (⌃ ↑)', mods: ['ControlLeft'], code: 'ArrowUp' },
    { label: 'Close window (⌘ W)', mods: ['MetaLeft'], code: 'KeyW' },
    { label: 'Quit app (⌘ Q)', mods: ['MetaLeft'], code: 'KeyQ' },
    { label: 'Force Quit (⌥ ⌘ Esc)', mods: ['AltLeft', 'MetaLeft'], code: 'Escape' },
    { label: 'Screenshot (⇧ ⌘ 5)', mods: ['ShiftLeft', 'MetaLeft'], code: 'Digit5' },
];
