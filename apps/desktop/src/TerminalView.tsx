import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";
import { api, type AgentId } from "./api";
import { THEME_EVENT, terminalTheme } from "./themes";
import { claudeNewlineHandler } from "./terminalKeys";
import { PREFS_EVENT, getPrefs } from "./prefs";
import { onFileDrop, quotePaths } from "./fileDrop";
import { arrowKeys, sgrWheelReports, wheelSteps } from "./terminalWheel";

const FONT = '"JetBrains Mono", Menlo, monospace';
const RESET = [0x1b, 0x63]; // ESC c: the daemon sends it before replaying a snapshot

/** Parses a leading `ESC [ 8 ; rows ; cols t` (the recorded size) off a replay. */
export function splitReplaySize(bytes: Uint8Array): { size: { cols: number; rows: number } | null; rest: Uint8Array } {
  const head = new TextDecoder().decode(bytes.subarray(0, 24));
  const m = /^\x1b\[8;(\d{1,4});(\d{1,4})t/.exec(head);
  if (!m) return { size: null, rest: bytes };
  const rows = Number(m[1]);
  const cols = Number(m[2]);
  const rest = bytes.subarray(m[0].length);
  return rows > 0 && cols > 0 && rows <= 500 && cols <= 1000 ? { size: { cols, rows }, rest } : { size: null, rest };
}

/**
 * Agents that attach a clipboard image on Ctrl+V and don't handle ⌘V for images (each one's
 * own docs: Codex "Paste an image with Ctrl+V", Vibe's Ctrl+V / `/paste-image`, Antigravity's
 * "Paste image/video" key). Claude Code handles ⌘V itself, so it isn't listed.
 */
const CTRL_V_IMAGE_AGENTS = new Set<AgentId>(["codex", "vibe", "antigravity"]);

/** A paste that carries an image and no text (a screenshot, an image copied in a browser). */
function isImageOnlyPaste(data: DataTransfer): boolean {
  const hasImage = Array.from(data.items).some((i) => i.kind === "file" && i.type.startsWith("image/"));
  return hasImage && data.getData("text/plain") === "";
}

/**
 * An xterm.js view of one session's pty. Keystrokes go straight to the pty. Give it a
 * `key` that changes when the session starts or stops, so it re-attaches to the new stream.
 */
export function TerminalView({
  sessionId,
  agent,
  live,
  compact = false,
  autoFocus = true,
}: {
  sessionId: string;
  agent: AgentId;
  live: boolean;
  /** One point smaller than the chosen font size (the split pane). */
  compact?: boolean;
  autoFocus?: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const liveRef = useRef(live);
  liveRef.current = live;
  const agentRef = useRef(agent);
  agentRef.current = agent;

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    let disposed = false;
    let cleanup = () => {};
    const prefs = getPrefs();
    const sizeFor = (p: typeof prefs) => Math.max(8, p.terminalFontSize - (compact ? 1 : 0));
    const fontSize = sizeFor(prefs);

    // xterm measures cell size when it opens, so the font must be loaded first.
    document.fonts.load(`${fontSize}px "JetBrains Mono"`).finally(() => {
      if (disposed) return;
      const term = new Terminal({
        fontFamily: FONT,
        fontSize,
        lineHeight: prefs.terminalLineHeight,
        scrollback: 10_000,
        scrollSensitivity: prefs.terminalScrollSpeed,
        macOptionIsMeta: true,
        cursorBlink: prefs.cursorBlink,
        cursorStyle: prefs.cursorStyle,
        allowProposedApi: false,
        theme: terminalTheme(),
      });
      const fit = new FitAddon();
      term.loadAddon(fit);
      term.open(el);
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        term.loadAddon(webgl);
      } catch {
        // WebGL unavailable: xterm falls back to its DOM renderer.
      }
      fit.fit();

      // While a snapshot replays, the terminal may answer queries embedded in old output
      // (cursor position, device attributes); those answers must not reach the agent.
      let replaying = 0;

      let resizeTimer: ReturnType<typeof setTimeout> | undefined;
      let frame = 0;
      const send = (fn: () => Promise<unknown>) => {
        if (replaying === 0 && liveRef.current) void fn().catch(() => {});
      };

      // A replay is the daemon's raw output log, trimmed at the front and drawn at whatever
      // sizes the session had back then, so a TUI's last frame often lands in the wrong rows.
      // Only a resize makes the agent repaint, and the pty usually already has this size,
      // so nudge it one row smaller and back. The second step waits for the agent to answer
      // the first (or a timeout): Node only reports a resize when the size it reads differs,
      // so two back-to-back changes could cancel out.
      let onNextOutput: (() => void) | null = null;
      let redrawTimer: ReturnType<typeof setTimeout> | undefined;
      const forceRedraw = () => {
        if (disposed || !liveRef.current || term.rows < 2) return;
        if (resizeTimer) clearTimeout(resizeTimer);
        if (redrawTimer) clearTimeout(redrawTimer);
        const restore = () => {
          onNextOutput = null;
          if (redrawTimer) clearTimeout(redrawTimer);
          redrawTimer = undefined;
          if (!disposed && liveRef.current) void api.resize(sessionId, term.cols, term.rows).catch(() => {});
        };
        void api
          .resize(sessionId, term.cols, term.rows - 1)
          .then(() => {
            if (disposed) return;
            onNextOutput = restore;
            redrawTimer = setTimeout(restore, 400);
          })
          .catch(() => {});
      };
      term.attachCustomKeyEventHandler(
        claudeNewlineHandler(
          () => agentRef.current,
          (data) => send(() => api.write(sessionId, data)),
        ),
      );
      // ⌘V with an image on the clipboard: xterm.js only pastes text, so for agents that
      // take images on Ctrl+V, send them Ctrl+V and let them read the clipboard themselves.
      const onPaste = (e: ClipboardEvent) => {
        if (!e.clipboardData || !CTRL_V_IMAGE_AGENTS.has(agentRef.current)) return;
        if (!isImageOnlyPaste(e.clipboardData)) return;
        e.preventDefault();
        e.stopImmediatePropagation();
        send(() => api.write(sessionId, "\x16"));
      };
      el.addEventListener("paste", onPaste, true);
      // Files dropped on the terminal type their paths, as a paste so agents can tell
      // them from typing (Claude Code attaches dropped images this way).
      const stopFileDrop = onFileDrop(el.parentElement ?? el, (paths) => {
        if (!liveRef.current) return;
        term.paste(quotePaths(paths));
        term.focus();
      });

      // Scroll speed for full-screen agents (see terminalWheel.ts). xterm doesn't expose which
      // mouse encoding the agent chose, so follow mode 1006 (SGR) here; with any other
      // encoding xterm keeps handling the wheel itself.
      let sgrMouse = false;
      const setMouseEncoding = (on: boolean) => (params: (number | number[])[]) => {
        if (params.includes(1006)) sgrMouse = on;
        return false; // xterm still applies the mode
      };
      const wheelCarry = { value: 0 };
      term.attachCustomWheelEventHandler((ev) => {
        const fullScreen = term.buffer.active.type === "alternate";
        // X10 mode reports clicks only, not the wheel.
        const reporting = term.modes.mouseTrackingMode !== "none" && term.modes.mouseTrackingMode !== "x10";
        // xterm's own history: scrollSensitivity covers it.
        if (!reporting && !fullScreen) return true;
        if (reporting && !sgrMouse) return true;
        const speed = getPrefs().terminalScrollSpeed;
        if (speed === 1 && !ev.altKey && !ev.ctrlKey) return true; // xterm's own behaviour
        const screen = term.element?.querySelector(".xterm-screen")?.getBoundingClientRect();
        if (!screen || screen.height === 0 || screen.width === 0) return true;
        ev.preventDefault();
        const steps = wheelSteps(ev, screen.height / term.rows, speed, wheelCarry);
        if (steps === 0) return false;
        const clamp = (n: number, max: number) => Math.min(max, Math.max(1, n));
        const col = clamp(Math.floor(((ev.clientX - screen.left) / screen.width) * term.cols) + 1, term.cols);
        const row = clamp(Math.floor(((ev.clientY - screen.top) / screen.height) * term.rows) + 1, term.rows);
        const data = reporting
          ? sgrWheelReports(steps, col, row, ev)
          : arrowKeys(steps, term.modes.applicationCursorKeysMode);
        send(() => api.write(sessionId, data));
        return false;
      });

      const subs = [
        term.parser.registerCsiHandler({ prefix: "?", final: "h" }, setMouseEncoding(true)),
        term.parser.registerCsiHandler({ prefix: "?", final: "l" }, setMouseEncoding(false)),
        term.parser.registerEscHandler({ final: "c" }, () => {
          sgrMouse = false;
          return false;
        }),
        term.onData((d) => send(() => api.write(sessionId, d))),
        term.onBinary((d) =>
          send(() => api.writeBytes(sessionId, Array.from(d, (c) => c.charCodeAt(0) & 0xff))),
        ),
        // Tell the agent its new size once the layout has settled, not on every frame of a
        // panel opening or closing.
        term.onResize(({ cols, rows }) => {
          if (resizeTimer) clearTimeout(resizeTimer);
          resizeTimer = setTimeout(() => {
            if (liveRef.current) void api.resize(sessionId, cols, rows).catch(() => {});
          }, 80);
        }),
      ];

      api
        .attach(sessionId, (bytes) => {
          const isReplay = bytes.length >= 2 && bytes[0] === RESET[0] && bytes[1] === RESET[1];
          if (isReplay) {
            replaying++;
            term.reset();
            sgrMouse = false;
            // The daemon follows the reset with the size the output was drawn at
            // (ESC [ 8 ; rows ; cols t). Replaying TUI output at any other size garbles it,
            // so the terminal takes that size before the output is written.
            const { size, rest } = splitReplaySize(bytes.subarray(2));
            if (size) term.resize(size.cols, size.rows);
            term.write(rest, () => {
              replaying--;
              // A running session then takes the window's size; the agent redraws to fit.
              // An ended one keeps the size its output was drawn at.
              if (!disposed && liveRef.current) {
                fit.fit();
                term.refresh(0, term.rows - 1);
                forceRedraw();
              }
            });
          } else {
            term.write(bytes);
            onNextOutput?.();
          }
        })
        .then(() => {
          if (!disposed && liveRef.current) void api.resize(sessionId, term.cols, term.rows).catch(() => {});
        })
        .catch((e) => term.write(`\r\n\x1b[31m[termlab] could not attach: ${e}\x1b[0m\r\n`));

      // Refit when the container changes size (window, split, inspector), then repaint: the
      // WebGL renderer in the macOS web view can keep showing a stale canvas after a resize.
      // Follow theme changes.
      const onTheme = () => {
        term.options.theme = terminalTheme();
      };
      window.addEventListener(THEME_EVENT, onTheme);
      // Follow Settings → Terminal: font size, line height, cursor and scroll speed apply in place.
      const onPrefs = () => {
        const p = getPrefs();
        const size = sizeFor(p);
        const metricsChanged = term.options.fontSize !== size || term.options.lineHeight !== p.terminalLineHeight;
        term.options.cursorBlink = p.cursorBlink;
        term.options.cursorStyle = p.cursorStyle;
        term.options.scrollSensitivity = p.terminalScrollSpeed;
        if (!metricsChanged) return;
        term.options.fontSize = size;
        term.options.lineHeight = p.terminalLineHeight;
        void document.fonts.load(`${size}px "JetBrains Mono"`).finally(() => {
          if (disposed) return;
          if (liveRef.current && replaying === 0) fit.fit();
          term.refresh(0, term.rows - 1);
        });
      };
      window.addEventListener(PREFS_EVENT, onPrefs);

      const ro = new ResizeObserver(() => {
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
          if (disposed || el.clientWidth === 0 || el.clientHeight === 0) return;
          if (liveRef.current && replaying === 0) fit.fit();
          term.refresh(0, term.rows - 1);
        });
      });
      ro.observe(el);
      if (autoFocus) term.focus();

      cleanup = () => {
        ro.disconnect();
        el.removeEventListener("paste", onPaste, true);
        stopFileDrop();
        window.removeEventListener(THEME_EVENT, onTheme);
        window.removeEventListener(PREFS_EVENT, onPrefs);
        cancelAnimationFrame(frame);
        if (resizeTimer) clearTimeout(resizeTimer);
        if (redrawTimer) clearTimeout(redrawTimer);
        subs.forEach((s) => s.dispose());
        void api.detach(sessionId).catch(() => {});
        term.dispose();
      };
    });

    return () => {
      disposed = true;
      cleanup();
    };
  }, [sessionId, compact, autoFocus]);

  // xterm's fit measures its parent's height including padding, so the padding lives on the
  // outer box and xterm gets an unpadded inner surface that's exactly the space it can use.
  return (
    <div className="terminal-host">
      <div ref={host} className="terminal-surface" role="application" aria-label="Terminal" />
    </div>
  );
}
