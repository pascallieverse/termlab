// Settings: a sidebar of sections (General, Notifications, Appearance, Terminal, Agents,
// Shortcuts, About). Every choice applies at once; there is nothing to save.

import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { PERMISSIONS, defaultPermission, setDefaultPermission } from "./agents";
import type { PermissionPreset } from "./api";
import { animeArtEnabled, setAnimeArtEnabled } from "./AnimeCard";
import { Dialog } from "./Dialog";
import { musicEnabled, setMusicEnabled } from "./MusicCard";
import {
  DEFAULT_PREFS,
  FONT_SIZE_RANGE,
  LINE_HEIGHTS,
  SCROLL_SPEEDS,
  setPrefs,
  usePrefs,
  type CursorStyle,
} from "./prefs";
import { useStore } from "./store";
import { THEMES, THEME_EVENT, chooseTheme, currentTheme, type ThemeId } from "./themes";
import { Icon, Kbd, Logo, Monogram } from "./ui";
import { UpdateSettings, useAppUpdate } from "./updates";
import { SHORTCUTS, comboFromEvent, comboProblem, formatCombo, type Combo, type ShortcutId } from "./keybindings";
import {
  IS_MAC,
  MOD_LABEL,
  isDefaultBinding,
  resetAllBindings,
  resetBinding,
  setBinding,
  setRecordingShortcut,
  useBindings,
} from "./shortcuts";

type SectionId = "general" | "notifications" | "appearance" | "terminal" | "agents" | "shortcuts" | "about";

const SECTIONS: { id: SectionId; label: string; icon: Parameters<typeof Icon>[0]["name"]; desc: string }[] = [
  { id: "general", label: "General", icon: "sliders", desc: "How new sessions start, and the background service." },
  { id: "notifications", label: "Notifications", icon: "bell", desc: "When Termlab gets your attention." },
  { id: "appearance", label: "Appearance", icon: "palette", desc: "Colours for the app and its terminals, and the sidebar extras." },
  { id: "terminal", label: "Terminal", icon: "terminal", desc: "Text, cursor and scrolling in every session's terminal." },
  { id: "agents", label: "Agents", icon: "bot", desc: "The coding agents Termlab found on this computer." },
  { id: "shortcuts", label: "Shortcuts", icon: "keyboard", desc: "Keyboard shortcuts, and the keys that trigger them." },
  { id: "about", label: "About", icon: "info", desc: "Version and updates." },
];

const MOD = MOD_LABEL;
const SHIFT = IS_MAC ? "⇧" : "Shift+";

/** The section shown when Settings opens again in this window. */
let lastSection: SectionId = "general";

export function SettingsDialog() {
  const store = useStore();
  const close = () => store.setSettingsOpen(false);
  const [section, setSection] = useState<SectionId>(lastSection);
  const [version, setVersion] = useState<string | null>(null);
  const update = useAppUpdate();
  const tabs = useRef<Record<string, HTMLButtonElement | null>>({});

  useEffect(() => {
    getVersion().then(setVersion, () => {});
  }, []);

  function go(id: SectionId, focus = false) {
    lastSection = id;
    setSection(id);
    if (focus) tabs.current[id]?.focus();
  }

  function onNavKey(e: KeyboardEvent) {
    const i = SECTIONS.findIndex((s) => s.id === section);
    const to =
      e.key === "ArrowDown" ? (i + 1) % SECTIONS.length
      : e.key === "ArrowUp" ? (i - 1 + SECTIONS.length) % SECTIONS.length
      : e.key === "Home" ? 0
      : e.key === "End" ? SECTIONS.length - 1
      : -1;
    if (to < 0) return;
    e.preventDefault();
    go(SECTIONS[to].id, true);
  }

  const current = SECTIONS.find((s) => s.id === section) ?? SECTIONS[0];
  const installed = store.agents?.filter((a) => a.installed && a.agent !== "shell").length;
  const badge: Partial<Record<SectionId, ReactNode>> = {
    general: !store.daemon.connected ? <span className="settings-tab-dot warn" title="Background service not connected" /> : null,
    agents: store.agents ? <span className="settings-tab-count">{installed}</span> : null,
    about: update.check?.version ? <span className="settings-tab-dot" title="Update available" /> : null,
  };

  return (
    <Dialog onClose={close} width={900} className="settings-dialog" labelledBy="st-title">
      <div className="settings-layout">
        <nav className="settings-nav" aria-labelledby="st-title">
          <h1 id="st-title" className="settings-nav-title">
            Settings
          </h1>
          <div role="tablist" aria-orientation="vertical" aria-labelledby="st-title" className="settings-tabs" onKeyDown={onNavKey}>
            {SECTIONS.map((s) => (
              <button
                key={s.id}
                ref={(el) => {
                  tabs.current[s.id] = el;
                }}
                id={`st-tab-${s.id}`}
                role="tab"
                aria-selected={section === s.id}
                aria-controls="st-panel"
                data-autofocus={section === s.id ? true : undefined}
                className={`settings-tab${section === s.id ? " on" : ""}`}
                onClick={() => go(s.id)}
              >
                <Icon name={s.icon} size={16} />
                <span className="settings-tab-label">{s.label}</span>
                {badge[s.id]}
              </button>
            ))}
          </div>
          <div className="settings-nav-foot">
            <span className="mono">Termlab {version ?? ""}</span>
          </div>
        </nav>

        <div className="settings-main">
          <div className="settings-main-head">
            <div className="settings-main-title">
              <h2>{current.label}</h2>
              <p className="muted small">{current.desc}</p>
            </div>
            <button className="icon-btn" aria-label="Close settings" onClick={close}>
              <Icon name="close" size={16} />
            </button>
          </div>
          <div key={section} id="st-panel" role="tabpanel" aria-labelledby={`st-tab-${section}`} className="settings-content">
            {section === "general" && <GeneralSection />}
            {section === "notifications" && <NotificationsSection />}
            {section === "appearance" && <AppearanceSection />}
            {section === "terminal" && <TerminalSection />}
            {section === "agents" && <AgentsSection />}
            {section === "shortcuts" && <ShortcutsSection />}
            {section === "about" && <AboutSection version={version} />}
          </div>
        </div>
      </div>
    </Dialog>
  );
}

// ---- building blocks ----

function Group({ title, note, children }: { title: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section className="settings-group">
      <h3 className="section-heading">{title}</h3>
      <div className="settings-card">{children}</div>
      {note && <p className="muted small settings-note">{note}</p>}
    </section>
  );
}

function Row({ title, desc, labelId, children }: { title: ReactNode; desc?: ReactNode; labelId?: string; children?: ReactNode }) {
  return (
    <div className="settings-row">
      <div className="settings-row-text">
        <span className="settings-row-title" id={labelId}>
          {title}
        </span>
        {desc && <span className="muted small">{desc}</span>}
      </div>
      {children && <div className="settings-row-control">{children}</div>}
    </div>
  );
}

function SwitchRow({
  title,
  desc,
  checked,
  onChange,
}: {
  title: string;
  desc?: ReactNode;
  checked: boolean;
  onChange(on: boolean): void;
}) {
  return (
    <label className="settings-row clickable">
      <span className="settings-row-text">
        <span className="settings-row-title">{title}</span>
        {desc && <span className="muted small">{desc}</span>}
      </span>
      <input type="checkbox" role="switch" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

function Segmented<T extends string | number>({
  labelledBy,
  value,
  options,
  onChange,
}: {
  labelledBy: string;
  value: T;
  options: readonly { value: T; label: string }[];
  onChange(v: T): void;
}) {
  return (
    <div role="radiogroup" aria-labelledby={labelledBy} className="segmented dark">
      {options.map((o) => (
        <button
          key={String(o.value)}
          role="radio"
          aria-checked={value === o.value}
          className={value === o.value ? "on" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

// ---- sections ----

function GeneralSection() {
  const store = useStore();
  return (
    <>
      <Group title="New sessions" note="Each session can still pick its own level in the New session dialog.">
        <div className="settings-block">
          <DefaultPermissionPicker />
        </div>
      </Group>
      <Group title="Background service">
        <Row
          title={
            <span className="settings-status">
              <span className={`dot${store.daemon.connected ? " ok" : " bad"}`} aria-hidden="true" />
              {store.daemon.connected ? "Connected" : "Not connected"}
            </span>
          }
          desc={
            store.daemon.connected
              ? "Sessions keep running when you close Termlab, and reopen where you left them."
              : (store.daemon.message ?? "Termlab can't reach its background service, so sessions can't start.")
          }
        />
      </Group>
    </>
  );
}

function NotificationsSection() {
  const prefs = usePrefs();
  const badgeName = IS_MAC ? "Dock badge" : "Taskbar badge";
  return (
    <>
      <Group
        title="System notifications"
        note="Sessions started with notifications turned off stay quiet either way."
      >
        <SwitchRow
          title="When a session needs you"
          desc="An approval or a question. Skipped while you're looking at that session."
          checked={prefs.notifyNeedsYou}
          onChange={(on) => setPrefs({ notifyNeedsYou: on })}
        />
        <SwitchRow
          title="When a session finishes or fails"
          desc="Only while Termlab is in the background."
          checked={prefs.notifyFinished}
          onChange={(on) => setPrefs({ notifyFinished: on })}
        />
      </Group>
      <Group title="App icon">
        <SwitchRow
          title={badgeName}
          desc="Show how many sessions are waiting on you."
          checked={prefs.dockBadge}
          onChange={(on) => setPrefs({ dockBadge: on })}
        />
      </Group>
    </>
  );
}

function AppearanceSection() {
  const [music, setMusic] = useState(musicEnabled);
  const [anime, setAnime] = useState(animeArtEnabled);
  return (
    <>
      <Group
        title="Theme"
        note="A theme colours the app and its terminals together. Agents that draw with their own exact colours (Claude Code's default themes do) keep those for that text."
      >
        <div className="settings-block">
          <ThemePicker />
        </div>
      </Group>
      <Group title="Sidebar">
        <SwitchRow
          title="YouTube Music player"
          desc="Lofi and other radio stations at the bottom of the sidebar."
          checked={music}
          onChange={(on) => {
            setMusic(on);
            setMusicEnabled(on);
          }}
        />
        <SwitchRow
          title="Anime art"
          desc="A random picture in the sidebar's empty space, from nekos.best."
          checked={anime}
          onChange={(on) => {
            setAnime(on);
            setAnimeArtEnabled(on);
          }}
        />
      </Group>
    </>
  );
}

const CURSOR_STYLES: { value: CursorStyle; label: string }[] = [
  { value: "block", label: "Block" },
  { value: "bar", label: "Bar" },
  { value: "underline", label: "Underline" },
];

function TerminalSection() {
  const prefs = usePrefs();
  const size = prefs.terminalFontSize;
  const isDefault =
    prefs.terminalFontSize === DEFAULT_PREFS.terminalFontSize &&
    prefs.terminalLineHeight === DEFAULT_PREFS.terminalLineHeight &&
    prefs.cursorStyle === DEFAULT_PREFS.cursorStyle &&
    prefs.cursorBlink === DEFAULT_PREFS.cursorBlink &&
    prefs.terminalScrollSpeed === DEFAULT_PREFS.terminalScrollSpeed;
  return (
    <>
      <TerminalPreview />
      <Group title="Text" note="The split pane uses one point smaller.">
        <Row title="Font size" labelId="st-font" desc="JetBrains Mono">
          <div className="stepper" role="group" aria-labelledby="st-font">
            <button
              className="icon-btn small"
              aria-label="Smaller"
              disabled={size <= FONT_SIZE_RANGE.min}
              onClick={() => setPrefs({ terminalFontSize: size - 1 })}
            >
              <span aria-hidden="true">−</span>
            </button>
            <output className="mono" aria-live="polite">
              {size}px
            </output>
            <button
              className="icon-btn small"
              aria-label="Larger"
              disabled={size >= FONT_SIZE_RANGE.max}
              onClick={() => setPrefs({ terminalFontSize: size + 1 })}
            >
              <Icon name="plus" />
            </button>
          </div>
        </Row>
        <Row title="Line spacing" labelId="st-lh">
          <Segmented
            labelledBy="st-lh"
            value={prefs.terminalLineHeight}
            options={LINE_HEIGHTS}
            onChange={(v) => setPrefs({ terminalLineHeight: v })}
          />
        </Row>
      </Group>
      <Group title="Cursor">
        <Row title="Shape" labelId="st-cursor">
          <Segmented
            labelledBy="st-cursor"
            value={prefs.cursorStyle}
            options={CURSOR_STYLES}
            onChange={(v) => setPrefs({ cursorStyle: v })}
          />
        </Row>
        <SwitchRow title="Blink" checked={prefs.cursorBlink} onChange={(on) => setPrefs({ cursorBlink: on })} />
      </Group>
      <Group title="Scrolling" note="Hold ⌥ Option (Alt) while scrolling to go five times faster still.">
        <Row title="Scroll speed" labelId="st-scroll" desc="Mouse wheel and trackpad">
          <Segmented
            labelledBy="st-scroll"
            value={prefs.terminalScrollSpeed}
            options={SCROLL_SPEEDS}
            onChange={(v) => setPrefs({ terminalScrollSpeed: v })}
          />
        </Row>
      </Group>
      <div>
        <button
          className="btn"
          disabled={isDefault}
          onClick={() =>
            setPrefs({
              terminalFontSize: DEFAULT_PREFS.terminalFontSize,
              terminalLineHeight: DEFAULT_PREFS.terminalLineHeight,
              cursorStyle: DEFAULT_PREFS.cursorStyle,
              cursorBlink: DEFAULT_PREFS.cursorBlink,
              terminalScrollSpeed: DEFAULT_PREFS.terminalScrollSpeed,
            })
          }
        >
          <Icon name="refresh" /> Reset terminal settings
        </button>
      </div>
    </>
  );
}

/** A few lines drawn like a terminal, with the current theme, font size, spacing and cursor. */
function TerminalPreview() {
  const prefs = usePrefs();
  const [theme, setTheme] = useState(currentTheme);
  useEffect(() => {
    const on = () => setTheme(currentTheme());
    window.addEventListener(THEME_EVENT, on);
    return () => window.removeEventListener(THEME_EVENT, on);
  }, []);
  const a = theme.ansi;
  return (
    <div
      className="term-preview mono"
      aria-label="Preview"
      role="img"
      style={{
        background: theme.tokens.terminal,
        color: theme.tokens.text,
        fontSize: prefs.terminalFontSize,
        lineHeight: prefs.terminalLineHeight,
      }}
    >
      <div>
        <span style={{ color: a.green }}>~/code/billing-api</span> <span style={{ color: a.blue }}>main</span>
      </div>
      <div>
        <span style={{ color: a.magenta }}>❯</span> npm test
      </div>
      <div>
        <span style={{ color: a.green }}>✓</span> 42 passed <span style={{ color: a.brightBlack }}>(1.8s)</span>
      </div>
      <div>
        <span style={{ color: a.yellow }}>!</span> 1 skipped · <span style={{ color: a.red }}>0 failed</span>
      </div>
      <div>
        <span style={{ color: a.magenta }}>❯</span>{" "}
        <span
          className={`term-preview-cursor ${prefs.cursorStyle}${prefs.cursorBlink ? " blink" : ""}`}
          style={{ ["--cursor" as string]: theme.tokens.text }}
        >
          {" "}
        </span>
      </div>
    </div>
  );
}

function AgentsSection() {
  const store = useStore();
  const [checking, setChecking] = useState(false);
  return (
    <>
      <Group
        title="Detected agents"
        note="Termlab runs each agent's own CLI, with its own login. Install one and check again to use it here."
      >
        {store.agents === null ? (
          <Row title="Looking…" />
        ) : (
          store.agents.filter((a) => a.agent !== "shell").map((a) => (
            <div key={a.agent} className={`settings-row agent-row${a.installed ? "" : " missing"}`}>
              <Monogram agent={a.agent} size={30} />
              <div className="settings-row-text">
                <span className="settings-row-title">
                  {a.name}
                  {a.installed && (
                    <span className="mono muted small agent-version" title={a.installed.path}>
                      {a.installed.version ?? "installed"}
                    </span>
                  )}
                </span>
                <span className="muted small">
                  {!a.installed
                    ? `${a.binary} isn't on your PATH`
                    : a.installed.loginBilling === "subscription"
                      ? "Signed in with a subscription"
                      : a.installed.loginBilling === "api_key"
                        ? "Signed in with an API key"
                        : <span className="mono">{a.installed.path}</span>}
                </span>
              </div>
              <div className="agent-caps">
                {a.installed ? (
                  <>
                    {a.hooks && <span className="tag" title="Termlab sees when it needs you and what it's doing">Live status</span>}
                    {a.canResume && <span className="tag">Resume</span>}
                    {a.canFork && <span className="tag">Fork</span>}
                  </>
                ) : (
                  <span className="tag">Not installed</span>
                )}
              </div>
            </div>
          ))
        )}
      </Group>
      <div>
        <button
          className="btn"
          disabled={checking}
          onClick={async () => {
            setChecking(true);
            await store.refreshAgents();
            setChecking(false);
          }}
        >
          <Icon name="refresh" /> {checking ? "Checking…" : "Check again"}
        </button>
      </div>
    </>
  );
}

const FIXED_SHORTCUTS: { group: string; items: { keys: string[]; label: string }[] }[] = [
  {
    group: "Terminal",
    items: [
      { keys: [`${SHIFT}↵`], label: "New line in Claude Code's prompt" },
      { keys: [`${MOD}V`], label: "Paste a screenshot into Codex, Vibe or Antigravity" },
    ],
  },
  {
    group: "Dialogs and search",
    items: [
      { keys: [`${MOD}↵`], label: "Start, send, or open beside the current session" },
      { keys: ["↑", "↓"], label: "Move through search results" },
      { keys: ["Esc"], label: "Close" },
    ],
  },
];

function ShortcutsSection() {
  const bindings = useBindings();
  const [notice, setNotice] = useState<{ id: ShortcutId; text: string; error?: boolean } | null>(null);
  const allDefault = SHORTCUTS.every((s) => isDefaultBinding(s.id, bindings));
  const groups = [...new Set(SHORTCUTS.map((s) => s.group))];
  return (
    <>
      <p className="muted small settings-intro">
        Click a shortcut, then press the new keys. <Kbd>Esc</Kbd> cancels, <Kbd>{IS_MAC ? "⌫" : "Backspace"}</Kbd>{" "}
        turns it off.
      </p>
      {groups.map((g) => (
        <Group key={g} title={g}>
          {SHORTCUTS.filter((s) => s.group === g).map((s) => (
            <div key={s.id} className="settings-row shortcut-row">
              <div className="settings-row-text">
                <span id={`sc-${s.id}`}>{s.label}</span>
                {notice?.id === s.id && (
                  <span className={`small${notice.error ? " error-text" : " muted"}`} role={notice.error ? "alert" : "status"}>
                    {notice.text}
                  </span>
                )}
              </div>
              <div className="shortcut-edit">
                <KeyRecorder
                  labelledBy={`sc-${s.id}`}
                  combo={bindings[s.id]}
                  onRecord={(combo) => {
                    const problem = combo && comboProblem(combo, IS_MAC);
                    if (problem) {
                      setNotice({ id: s.id, text: problem, error: true });
                      return;
                    }
                    const displaced = setBinding(s.id, combo);
                    const lost = SHORTCUTS.find((x) => x.id === displaced);
                    setNotice(lost ? { id: s.id, text: `Taken from “${lost.label}”, which now has no shortcut.` } : null);
                  }}
                  onCancel={() => setNotice((n) => (n?.id === s.id && n.error ? null : n))}
                />
                <button
                  className="icon-btn small"
                  aria-label={`Reset “${s.label}” to ${formatCombo(s.default, IS_MAC)}`}
                  title={`Reset to ${formatCombo(s.default, IS_MAC)}`}
                  style={{ visibility: isDefaultBinding(s.id, bindings) ? "hidden" : undefined }}
                  onClick={() => {
                    const displaced = resetBinding(s.id);
                    const lost = SHORTCUTS.find((x) => x.id === displaced);
                    setNotice(lost ? { id: s.id, text: `Taken from “${lost.label}”, which now has no shortcut.` } : null);
                  }}
                >
                  <Icon name="refresh" />
                </button>
              </div>
            </div>
          ))}
        </Group>
      ))}
      <div>
        <button
          className="btn"
          disabled={allDefault}
          onClick={() => {
            resetAllBindings();
            setNotice(null);
          }}
        >
          <Icon name="refresh" /> Reset all shortcuts
        </button>
      </div>
      {FIXED_SHORTCUTS.map((g) => (
        <Group key={g.group} title={g.group} note={g.group === "Terminal" ? "These belong to the agents and the terminal, so they can't be changed." : undefined}>
          {g.items.map((s) => (
            <div key={s.label} className="settings-row shortcut-row">
              <span>{s.label}</span>
              <span className="shortcut-keys">
                {s.keys.map((k) => (
                  <Kbd key={k}>{k}</Kbd>
                ))}
              </span>
            </div>
          ))}
        </Group>
      ))}
    </>
  );
}

/**
 * A button that shows a shortcut and, once clicked, records the next key press as the new one.
 * Keys are read on the window, not the button: WebKit (the macOS app's web view) doesn't
 * focus a button when it's clicked, so the button itself would never see the press.
 */
function KeyRecorder({
  combo,
  labelledBy,
  onRecord,
  onCancel,
}: {
  combo: Combo | null;
  labelledBy: string;
  onRecord(combo: Combo | null): void;
  onCancel(): void;
}) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState<string>("");
  const button = useRef<HTMLButtonElement>(null);
  const callbacks = useRef({ onRecord, onCancel });
  callbacks.current = { onRecord, onCancel };

  useEffect(() => {
    if (!recording) return;
    setRecordingShortcut(true);
    const finish = (result?: { combo: Combo | null }) => {
      setRecording(false);
      setHeld("");
      if (result) callbacks.current.onRecord(result.combo);
      else callbacks.current.onCancel();
    };
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      const noMods = !e.metaKey && !e.ctrlKey && !e.altKey;
      // Tab still moves focus (and ends recording).
      if (e.key === "Tab" && noMods) {
        finish();
        return;
      }
      // Nothing else reaches the dialog, the app's shortcuts or the terminal.
      e.preventDefault();
      e.stopPropagation();
      e.stopImmediatePropagation();
      if (e.repeat || e.isComposing) return;
      if (e.key === "Escape" && noMods && !e.shiftKey) return finish();
      if ((e.key === "Backspace" || e.key === "Delete") && noMods && !e.shiftKey) return finish({ combo: null });
      const next = comboFromEvent(e, IS_MAC);
      if (!next) {
        // Only modifiers so far: show them while the key is awaited.
        const mods = { mod: IS_MAC ? e.metaKey : e.ctrlKey, alt: e.altKey, shift: e.shiftKey, key: "" };
        setHeld(formatCombo(mods, IS_MAC).replace(/\+$/, ""));
        return;
      }
      finish({ combo: next });
    };
    const onKeyUp = (e: globalThis.KeyboardEvent) => {
      e.stopPropagation();
      if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) setHeld("");
    };
    // A click anywhere else ends recording; a click on the button itself is its own toggle.
    const onPointerDown = (e: PointerEvent) => {
      if (!button.current?.contains(e.target as Node)) finish();
    };
    const onWindowBlur = () => finish();
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("blur", onWindowBlur);
    return () => {
      setRecordingShortcut(false);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("blur", onWindowBlur);
    };
  }, [recording]);

  return (
    <button
      ref={button}
      className={`key-recorder${recording ? " recording" : ""}${combo ? "" : " unset"}`}
      aria-labelledby={labelledBy}
      aria-pressed={recording}
      onClick={(e) => {
        e.currentTarget.focus();
        if (recording) {
          setRecording(false);
          setHeld("");
          onCancel();
        } else setRecording(true);
      }}
    >
      {recording ? (
        <span className="key-recorder-prompt" aria-live="polite">
          {held || "Press keys…"}
        </span>
      ) : combo ? (
        <Kbd>{formatCombo(combo, IS_MAC)}</Kbd>
      ) : (
        <span className="muted small">Not set</span>
      )}
    </button>
  );
}

function AboutSection({ version }: { version: string | null }) {
  return (
    <>
      <div className="about-hero">
        <span className="about-logo">
          <Logo />
        </span>
        <div>
          <div className="about-name">Termlab</div>
          <div className="muted small">
            Version <span className="mono">{version ?? "…"}</span>
          </div>
        </div>
      </div>
      <Group title="Updates" note="Termlab also checks on its own shortly after launch and every six hours.">
        <div className="settings-block">
          <UpdateSettings version={version} hideVersion />
        </div>
      </Group>
    </>
  );
}

// ---- pickers ----

const SWATCH_KEYS = ["red", "green", "yellow", "blue", "magenta", "cyan"] as const;

/** One choice for the app and its terminals; each preview shows a sidebar beside a terminal. */
function ThemePicker() {
  const [current, setCurrent] = useState<ThemeId>(currentTheme().id);
  return (
    <div role="radiogroup" aria-label="Theme" className="theme-grid">
      {THEMES.map((t) => (
        <button
          key={t.id}
          role="radio"
          aria-checked={current === t.id}
          className={`theme-option${current === t.id ? " on" : ""}`}
          onClick={() => {
            chooseTheme(t.id);
            setCurrent(t.id);
          }}
        >
          <span
            className="theme-swatch"
            aria-hidden="true"
            style={{ background: t.tokens.bg, borderColor: t.tokens["border-strong"] }}
          >
            <span className="theme-swatch-side" style={{ background: t.tokens.panel }}>
              <span style={{ background: t.tokens.selected }} />
            </span>
            <span className="theme-swatch-term mono" style={{ background: t.tokens.terminal, color: t.tokens.text }}>
              <span>
                <span style={{ color: t.ansi.green }}>❯</span> npm test
              </span>
              <span className="term-swatch-colors">
                {SWATCH_KEYS.map((k) => (
                  <span key={k} style={{ background: t.ansi[k] }} />
                ))}
              </span>
            </span>
          </span>
          <span className="theme-name">{t.name}</span>
          <span className="muted small">{t.description}</span>
        </button>
      ))}
    </div>
  );
}

/** Which permissions the New session dialog starts on. Each session can still choose its own. */
function DefaultPermissionPicker() {
  const [current, setCurrent] = useState<PermissionPreset>(defaultPermission);
  return (
    <>
      <span className="label" id="set-perm">
        Default permissions
      </span>
      <div role="radiogroup" aria-labelledby="set-perm" className="perm-cards">
        {PERMISSIONS.map((p) => (
          <button
            key={p.key}
            role="radio"
            aria-checked={current === p.key}
            className={`perm-card${current === p.key ? " on" : ""}`}
            onClick={() => {
              setDefaultPermission(p.key);
              setCurrent(p.key);
            }}
          >
            <span className="perm-label">{p.label}</span>
            <span className="muted small">{p.desc}</span>
          </button>
        ))}
      </div>
      {current === "full_auto" && (
        <p className="small settings-warning">
          <Icon name="warning" /> New sessions will start without permission prompts: the agent edits files and runs
          commands on its own.
        </p>
      )}
    </>
  );
}
