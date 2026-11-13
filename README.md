# Termlab

A desktop session manager for AI coding agents: Claude Code, Codex CLI, Mistral Vibe
and Antigravity CLI. It runs each agent's real CLI in a pseudo-terminal, with the agent's
own login, and adds the management layer around it:
- grouping by project folder (drag the grip beside a project to reorder; Alt+↑/↓ on the grip also works)
- status (Needs you, Running, Idle, Done, Error)
- approvals
- resume, fork and hand-off
- notifications
- a ⌘K palette

Right-click a project folder and choose **Edit .env…** to edit its environment file
inside Termlab. Save creates the file if needed; ⌘S / Ctrl+S also saves.

## How it fits together

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

Data lives in `~/Library/Application Support/dev.termlab.Termlab/`:
- `termlab.sqlite`
- `scrollback/<session>.log`
- the sockets
- `termlabd.log`

## Develop

```sh
cd apps/desktop
npm install
npm run tauri dev     # builds termlabd + termlab-hook, then runs Vite and the app
```

`npm run dev` alone serves the UI in a browser with a mocked daemon (sample data), for layout
work.

The app starts `termlabd` from next to its own executable. A daemon from an older build is
replaced automatically when it has no running sessions.

## Checks

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

## Build the app

```sh
cd apps/desktop && npm run tauri build
```

This builds `termlabd` and `termlab-hook` in release mode and bundles them into
`Termlab.app/Contents/MacOS/` (see `bundle.macOS.files` in `tauri.conf.json`).
