<div align="center">

<img src="apps/desktop/app-icon.png" alt="Termlab app icon" width="96" height="96">

# Termlab

**A desktop session manager for AI coding agents: run Claude Code, Codex CLI, Mistral Vibe and Antigravity CLI side by side, and see at a glance which one needs you.**

[![Latest release](https://img.shields.io/github/v/release/pascallieverse/termlab?label=download&sort=semver)](https://github.com/pascallieverse/termlab/releases/latest)
[![CI](https://github.com/pascallieverse/termlab/actions/workflows/ci.yml/badge.svg)](https://github.com/pascallieverse/termlab/actions/workflows/ci.yml)
![Platforms: macOS, Linux, Windows](https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows-informational)
![Built with Tauri, Rust and React](https://img.shields.io/badge/built%20with-Tauri%202%20%C2%B7%20Rust%20%C2%B7%20React-orange)

[Download](#download) · [Features](#features) · [How it works](#how-it-works) · [FAQ](#faq) · [Development](#development)

<br>

<img src="docs/screenshots/termlab.png" alt="Termlab showing several AI coding agent sessions grouped by project: a Claude Code session asking permission to run a database migration, with its terminal, changed files, context window and token usage" width="100%">

<sub>Sample data. The screenshots are retaken automatically for every release.</sub>

</div>

## Why Termlab

Running several AI coding agents at once in plain terminal tabs gets messy fast: one is waiting
for an approval, another finished ten minutes ago, a third ran out of context, and you only
find out by clicking through every tab.

Termlab runs each agent's **real CLI** in a pseudo-terminal, with the agent's own login and
subscription, and adds the management layer around it. Sessions are grouped by project,
show their live status, and ask for your attention only when they need it. They keep running
in a background service when you close the window.

Supported agents:

| Agent | Command | Resume | Fork | Live status via hooks |
|---|---|:-:|:-:|:-:|
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `claude` | ✓ | ✓ | ✓ |
| [Codex CLI](https://github.com/openai/codex) | `codex` | ✓ | ✓ | ✓ |
| Mistral Vibe | `vibe` | | | |
| Antigravity CLI | `agy` | ✓ | | |

Agents without hooks get their status from the terminal screen instead.

## Features

- **One window for every agent session.** Sessions are grouped by project folder, each with
  its full interactive terminal. Drag projects to reorder them.
- **Status at a glance.** Every session shows *Needs you*, *Running*, *Idle*, *Done* or
  *Error*, with badges per project and filters for All, Attention, Running and On hold.
- **Approvals without hunting.** When an agent asks permission to run a command or edit a
  file, Termlab shows the request with **Allow once**, **Always allow** and **Deny**, in the
  session and in Mission control.
- **Mission control.** A grid of every session with its last terminal lines, grouped by
  folder, agent or status, plus a "waiting on you" queue to review one by one.
- **Inspector.** Agent, model, branch, permission mode, context window use, tokens, cost
  (for API-key billing), changed files, a diff view and a timeline of prompts and tool calls.
- **Subagents.** See an agent's running subagents (test runners, explorers, reviewers) under
  its session.
- **Plan usage.** Your Claude and Codex plan limits in the header, so you know how much of the
  week is left.
- **Resume, fork and hand off.** Pick up an earlier session, branch it, or hand the task to a
  different agent with a summary of what was done so far.
- **Import existing sessions.** Load sessions you started outside Termlab, from one project or
  from anywhere on the machine.
- **Plain terminals** next to the agents: open your own shell in a project from New session
  or the command palette.
- **Drop files on a terminal** to type their paths, as in Terminal.app and iTerm.
- **Split view.** Two sessions next to each other.
- **Notifications** when a session needs you, finishes or fails.
- **On hold.** Park a session so it stops counting towards attention badges and notifications,
  without stopping its process.
- **Sessions survive the app.** A per-user background service owns the terminals. Close the
  window, reopen it, and the scrollback is replayed.
- **⌘K command palette** and keyboard shortcuts for everything, all rebindable.
- **12 themes**, including Tokyo Night, Catppuccin Mocha, Dracula, Gruvbox, Nord and One Dark,
  with adjustable terminal font, line spacing and cursor.
- **Per-project `.env` editor**: right-click a project and choose **Edit .env…**.
- **Updates itself** from GitHub releases, after you confirm.

<details>
<summary><b>More screenshots</b></summary>
<br>
<img src="docs/screenshots/mission-control.png" alt="Termlab Mission control: a grid of Claude Code, Codex, Mistral Vibe and Antigravity sessions grouped by project, with their status, last terminal output and an Allow button for a pending approval" width="100%">
<br><br>
<img src="docs/screenshots/focus.png" alt="Termlab Focus view: the sidebar of projects and sessions, a Claude Code terminal, and the inspector with session details and changed files" width="100%">
</details>

## Download

Get the latest version from the [**Releases page**](https://github.com/pascallieverse/termlab/releases/latest).

| Platform | Package |
|---|---|
| macOS, Apple Silicon (macOS 12 or later) | `.dmg` |
| Linux, x86-64 | `.deb`, `.rpm` |
| Windows, x86-64 | `-setup.exe` installer |

Intel Macs are not supported. Only the macOS build has been run on a real machine so far: the
Linux and Windows builds compile and are packaged, but are untested.

You also need at least one of the agents installed and signed in (`claude`, `codex`, `vibe`
or `agy`). Termlab finds them on your `PATH` and uses their own login: it never asks for your
API keys or subscription credentials.

> [!NOTE]
> If a macOS build isn't notarized yet, macOS blocks it the first time. Open
> **System Settings → Privacy & Security** and choose **Open Anyway**. On Windows, SmartScreen
> may warn before the installer runs.

## How it works

```
 Termlab.app (Tauri + React)  ⇄  termlabd (per-user daemon, owns the ptys)  ⇄  agent CLIs
        unix socket: termlabd.sock          ↑ hook.sock
                                            termlab-hook  ← run by Claude Code hooks / Codex notify
```

- `crates/termlab-core`: everything except UI.
  - Agent adapters, pty sessions with a headless screen model (`vt100`), and scrollback.
  - SQLite with migrations, status detection (hooks and screen heuristics), approvals.
  - Discovery, git, transcripts, and the app⇄daemon protocol (`ipc`).
- `crates/termlabd`: the daemon. Sessions keep running when the app closes. The app re-attaches
  and replays scrollback. The daemon exits after 10 idle minutes with no sessions.
- `crates/termlab-hook`: a tiny forwarder that agents' hooks run. It never prints and always
  exits 0.
- `apps/desktop`: the Tauri 2 app (`src-tauri/`) and React/Vite frontend (`src/`).
- `design/`: the visual spec. `docs/agent-notes.md`: every agent flag and path as verified.

Everything stays on your machine. Data lives in `~/Library/Application Support/dev.termlab.Termlab/`
(Linux: `~/.local/share/termlab/`, Windows: `%APPDATA%\termlab\Termlab\data\`):
- `termlab.sqlite`
- `scrollback/<session>.log`
- the sockets (on Windows these are named pipes instead: `\\.\pipe\termlab-…`)
- `termlabd.log`

## FAQ

**Is Termlab a replacement for Claude Code or Codex?**
No. It runs the official CLIs unchanged, in a real terminal, and adds session management
around them. Everything the CLI can do still works.

**Does it use my Claude or ChatGPT subscription, or an API key?**
Whatever the agent itself is signed in with. Termlab shows which one each session uses, and
the cost per day for sessions billed through an API key.

**What happens to running agents when I close the window or the app updates?**
They keep running in the background service (`termlabd`). Reopening Termlab reattaches to them
and replays their output.

**Can I manage sessions I started in a normal terminal?**
Yes: **Load existing sessions…** imports them, and running Claude Code background sessions
show up so you can attach to them.

**Does Termlab send my code or prompts anywhere?**
No. Your code and prompts go only to the agent CLIs on your machine (which talk to their own
providers as usual). Termlab itself connects to GitHub to check for updates; it reaches
YouTube Music only if you open its built-in music player, and nekos.best only if you turn on
the optional anime art in Settings.

## Development

```sh
cd apps/desktop
npm install
npm run tauri dev     # builds termlabd + termlab-hook, then runs Vite and the app
```

`npm run dev` alone serves the UI in a browser with a mocked daemon (sample data), for layout
work.

The app starts `termlabd` from next to its own executable. A daemon from an older build is
replaced automatically when it has no running sessions.

### Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace            # fake agent only: no real agent, no network
(cd apps/desktop && npm run typecheck)
```

To check against real agents (uses their logins and a little quota):

```sh
cargo build -p termlab-hook
cargo run -p termlab-core --example live_probe -- claude haiku   # or: codex | vibe
```

### Build the app

```sh
cd apps/desktop && npm run bundle
```

This builds `termlabd` and `termlab-hook` in release mode for the computer it runs on and
packages them next to the app (`scripts/sidecars.mjs`, `src-tauri/release.conf.json`).
Plain `npm run tauri build` only compiles the app: it leaves out the two helper programs
and makes no package.

Releases are built by GitHub Actions when a version tag is pushed: see
[`docs/releasing.md`](docs/releasing.md).

### Screenshots

The images in this README come from the UI running on its mocked daemon
(`apps/desktop/src/devMock.ts`), so they only ever show sample data. When a release is
published, [`.github/workflows/screenshots.yml`](.github/workflows/screenshots.yml) retakes
them, commits them to `docs/screenshots/` and attaches them to the release. To retake them
locally:

```sh
cd apps/desktop
npx playwright install chromium    # once; or skip it and use Chrome: PW_CHANNEL=chrome
npm run screenshots
```

To change what they show, edit the sample data in `devMock.ts` or the steps in
`scripts/screenshots.mjs`.
