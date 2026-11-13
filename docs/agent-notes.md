# Agent CLI notes

What Termlab relies on from each agent CLI, as verified on the development Mac on 2026-11-12:
with `--help`, the CLIs' own files, the binaries themselves, and **live runs through Termlab**
(`crates/termlab-core/examples/live_probe.rs`). **"Differs"** marks places where the original
build prompt was out of date.

| Agent | Version | Installed | Live-tested through Termlab |
|---|---|---|---|
| Claude Code (`claude`) | 2.1.322 | yes | yes: launch, trust dialog, permission prompt, hooks, resume id, transcript usage |
| Codex CLI (`codex`) | 0.167.0 | yes | yes: launch, trust dialog, sandbox approval, `notify`, rollout discovery, usage |
| Mistral Vibe (`vibe`) | 3.0.3 | yes | yes: launch, approval dialog, status |
| Antigravity CLI (`agy`) | 1.5.6 | yes | no: not signed in on the dev machine |

**Differs:** the prompt asked for Gemini CLI. Google ended Gemini CLI sign-in for individual
accounts ("This client is no longer supported for Gemini Code Assist for individuals… migrate to
the Antigravity suite"), so the Gemini adapter was replaced by the Antigravity CLI (`agy`). Its
monogram is `AG`, in the Gemini colours.

## Answering permission prompts (verified live)

All three agents tested show prompts as a list with a selection marker. Termlab reads the list
off its headless screen, moves the marker with the arrow keys to the wanted option and presses
Enter; it denies with Esc. It never guesses positions: if the option isn't on screen, the
button errors out.

- **Claude Code**, folder-trust dialog (first run in a folder). It is *unnumbered*, and the
  default is the negative option:
  ```
   ❯ No, exit
     Yes, I trust this folder
   Enter to confirm · Esc to cancel
  ```
  Pressing Enter blindly would exit Claude. "Allow once" sends ↓ then Enter.
- **Claude Code**, tool permission (`Do you want to proceed?`): numbered `❯ 1. Yes`,
  `2. Yes, and always allow access to … from this project`, `3. Yes, and switch to auto mode`,
  `4. No`, then `Esc to cancel · Tab to amend`. Option 3 changes the session's mode, so no button
  maps to it.
- **Codex**, trust dialog: `› 1. Trust and continue` / `2. Back to Agent Command Center`.
- **Codex**, sandbox escalation: `Would you like to run the following command?`, a
  `Reason: …?` line, the command, then `› 1. Yes, proceed (y)`,
  `2. Yes, and don't ask again for commands that start with … (p)`,
  `3. No, and tell Codex what to do differently (esc)`. **Codex keeps redrawing an unchanged
  screen while it waits**, so a prompt on screen beats "recent output" in the heuristics, and
  activity means "the visible text changed", not "bytes arrived".
- **Vibe**: a framed dialog with `↑↓/jk navigate  Enter select  Esc reject`. Vibe also redraws
  while idle.

These screens are test fixtures in `crates/termlab-core/src/status.rs`.

## Loading sessions Termlab didn't start

"Load existing sessions…" (sidebar, palette, empty state) opens the Resume tab with
**Everywhere on this Mac**. Everything below is read-only.

- **Past sessions:**
  - Claude Code: every `~/.claude/projects/*/*.jsonl`. The folder is the transcript's own
    `cwd`, because the directory-name encoding is lossy.
  - Codex: every top-level rollout in `~/.codex/sessions`.
  - Mistral Vibe: `~/.vibe/logs/session/session_*/meta.json` (`session_id`,
    `environment.working_directory`) plus the first user message in `messages.jsonl`.
- **Import** adds them to Termlab as ended sessions, adding each folder as needed. Sessions
  whose folder no longer exists are skipped. **Resume here** then runs the agent's resume
  command (`claude --resume`, `codex resume`, `vibe --resume`).
- **Running now:** `claude agents --json` (2.1.322) lists live Claude sessions:
  `[{"id":"<short>","sessionId","cwd","kind":"background"|"interactive","name","state":"working"|"blocked"|"done",…}]`.
  - Background sessions get **Attach**, which runs `claude attach <short id>` in a Termlab
    terminal. The session keeps running when that terminal closes.
  - Interactive sessions open in another terminal window can't be attached to.
- **Double-client guard:** Termlab refuses to resume a conversation that is running elsewhere,
  because two clients on one conversation would interleave.
- Claude's per-process files in `~/.claude/sessions/` hold tokens; Termlab doesn't read them and
  uses `claude agents --json` instead.

## Claude Code

- Model: `--model <alias|full name>` (aliases `opus`, `sonnet`, `haiku`, `fable`).
- Permissions: `--permission-mode` takes `acceptEdits`, `auto`, `bypassPermissions`, `manual`,
  `dontAsk`, `plan`.
  - **Differs:** there is no `default` any more. `manual` replaced it; the CLI maps `manual` to
    `default` internally.
  - Mapping: Ask first → `manual`, Auto-edit → `acceptEdits`, Full auto → `bypassPermissions`.
- **New:** `--session-id <uuid>` fixes Claude's session id at launch. Termlab passes its own
  session UUID, so the resume id is known before any hook fires.
- Resume: `--resume <id>`. Fork: `--resume <id> --fork-session`; the new id arrives via the
  SessionStart hook.
- First message: positional `[prompt]`; Termlab inserts `--` first when it starts with `-`.
- Hooks: `--settings <file-or-json>` loads extra settings for this run only. Termlab writes its
  own `hooks/claude-settings.json` in its data dir and never touches `~/.claude/settings.json`.
  - Registered events (all present in 2.1.322's hook schema): `SessionStart`,
    `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Notification`,
    `Stop`, `SessionEnd`.
  - **New, used:** `PermissionRequest` fires when the permission dialog is shown, with
    `tool_name` and `tool_input`; that's the approval bar's detail line (`$ touch probe.txt`).
  - `Notification` has a `notification_type`: `permission_prompt` means Needs you, while
    `idle_prompt` ("Claude is waiting for your input") means Idle.
  - The payload carries `session_id` and `transcript_path`. `Stop` carries
    `last_assistant_message`.
  - Hook stdout is fed back to the model for some events, so `termlab-hook` never prints.
- Transcripts: `~/.claude/projects/<encoded>/<session-id>.jsonl`.
  - Encoding: every character that isn't `[A-Za-z0-9]` becomes `-`.
  - `"type":"ai-title"` lines carry a title.
  - Usage repeats on every content-block line of a message, so it is de-duplicated by message id.
- **Inherited-session markers.** When the process that launched Termlab is itself a Claude Code
  session, variables like `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ID` and
  `CLAUDE_CODE_MESSAGING_TOKEN` leak into the environment. Claude then prints "Transcript saving
  is off — inherited CLAUDE_CODE_CHILD_SESSION marker". Termlab strips those markers (and
  Codex's equivalents) before launching agents, and keeps configuration variables.
- Billing: `claude auth status --json` reports `authMethod: "claude.ai"` and
  `subscriptionType` for a Pro/Max login. `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` in the
  environment mean API-key billing; Termlab shows the warning and then a dollar cost from
  first-party list prices.
- Context window: current Claude models have 1M tokens (Haiku 4.5 has 200K).

## Codex CLI

- Model: `-m, --model`. Models listed in `~/.codex/models_cache.json` (non-hidden `slug`s) are
  offered.
- Approvals:
  - **Differs:** there is no `--full-auto`.
  - `-a` takes only `on-request` or `never`; the sandbox is
    `-s read-only|workspace-write|danger-full-access`.
  - Mapping: Ask first → `-s read-only -a on-request`; Auto-edit →
    `-s workspace-write -a on-request`; Full auto → `-s workspace-write -a never`.
- Resume: `codex resume [options] <SESSION_ID> [PROMPT]`. Fork: `codex fork <SESSION_ID>`.
- Status: `-c 'notify=["<termlab-hook>", "codex", "notify"]'` overrides `notify` for that run
  only; `~/.codex/config.toml` is untouched.
  - Codex calls it with a JSON argument: `{"type":"agent-turn-complete","thread-id":…}`.
  - **`notify` also fires for side threads** (e.g. title generation), with thread ids that have
    no rollout. Termlab ignores notifications for any thread but the session's own, and a
    hook's "Idle" yields once the agent prints again.
  - Codex's `hooks` feature exists but needs persisted hook trust, so Termlab doesn't use it.
- Session id: Codex picks its own. Termlab finds it in the newest top-level rollout file in the
  folder created after the session started (`~/.codex/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl`,
  whose first line is `session_meta`). Subagent rollouts (`thread_source: "subagent"`) are
  skipped. Titles come from `~/.codex/session_index.jsonl`.
- Usage: the last `event_msg`/`token_count` gives `last_token_usage`, `total_token_usage` and
  `model_context_window`.
- Billing: `codex login status` prints "Logged in using ChatGPT" for a subscription. With
  `OPENAI_API_KEY`/`CODEX_API_KEY` set, Termlab warns but shows no dollar figure, because it has
  no verified OpenAI price table.
- Code-mode `exec` tool calls wrap shell commands as `tools.exec_command({cmd:"…"})`; the
  Timeline extracts the command.

## Mistral Vibe

- **Differs:** no model flag. The model is `active_model` in `~/.vibe/config.toml`, so the New
  session dialog shows "Set in Mistral Vibe's own config".
- Approvals: `--agent ask|accept-edits|auto-approve`. In Vibe's runtime, `ask` is the policy that
  asks before every tool. `accept-edits` is the default.
- Resume: `--resume <SESSION_ID>`. Vibe picks its own id. Termlab finds it in Vibe 3's session
  store, `~/.vibe/logs/session/unified/<session_id>/meta.json`, which has `session_id`,
  `start_time`, `environment.working_directory` and a generated `title`. The older
  `session_<date>_<time>_<id>/` layout is read too. Vibe writes a conversation only once it
  has a message, so a session with no messages can't be resumed.
- Status: screen heuristics. The live run showed `[Subscription] Pro` in Vibe's banner.

## Antigravity CLI (`agy`)

- Installed with Google's script `https://antigravity.google/cli/install.sh`. The script was read
  before running. It verifies a SHA-512 from Google's manifest, installs `~/.local/bin/agy`, and
  runs `agy install`, which **appended a PATH line to `~/.zprofile`, `~/.bash_profile` and
  `~/.profile`**.
- Model: `--model`. `agy models` needs a signed-in CLI.
- Permissions: Ask first → no flag (the default asks); Auto-edit → `--mode accept-edits`;
  Full auto → `--dangerously-skip-permissions`.
- First message: `-i/--prompt-interactive <text>`. Resume: `--conversation <ID>`; the id isn't
  known at launch.
- **Hooks aren't used.** `agy` reads hooks only from `~/.gemini/config/hooks.json` (shared with
  the Antigravity desktop app) or `.agents/hooks.json` in the project, and `--settings` only
  takes settings.json keys. Using either file would mean editing global config or the user's
  repo, which hard requirement 5 rules out, so status comes from the screen heuristics.
- API-key variables: `GEMINI_API_KEY`, `GOOGLE_API_KEY`.

## What Termlab reads from Claude Code's own files and status line

Read-only, verified against Claude Code 2.1.296 to 2.1.323:

- **`/rename` and `/color`**: Claude appends `{"type":"custom-title","customTitle":…}` and
  `{"type":"agent-color","agentColor":…}` lines to the session's transcript. Termlab follows
  the transcript incrementally and applies the newest of each. Colours: red, blue, green,
  yellow, purple, orange, pink, cyan; `default` clears it.
- **Sub-agents**: `<session id>/subagents/agent-<id>.jsonl` next to the transcript, each with
  an `agent-<id>.meta.json` (`name`, `description`, `agentType`, `model`). A transcript whose
  last line is an assistant message with `stop_reason: end_turn` has reported back.
  Codex: a child rollout's `session_meta.source.subagent.thread_spawn` names its
  `parent_thread_id` and `agent_nickname`; a rollout ending in `task_complete` is done.
- **Plan limits** (what `/usage` shows): Claude Code passes `rate_limits` to the status line
  command (`five_hour` / `seven_day`, each with `used_percentage` and `resets_at` in Unix
  seconds; subscribers only, after a session's first response). Termlab's own settings file
  (the one passed with `--settings`) registers `termlab-hook claude StatusLine`, which
  forwards the data and prints a one-line summary. If the user has a status line of their
  own, Termlab's command forwards quietly and then runs theirs. Per-model weekly limits
  aren't in this data. Sessions Termlab attaches to (started by Claude's own background
  service) don't use Termlab's settings, so they don't report.

## Plan limits: where each agent's numbers come from

- **Claude Code** (verified against 2.1.323): the full report, including per-model weeks,
  comes from a `get_usage` control request. Termlab starts
  `claude -p --input-format stream-json --output-format stream-json --verbose
  --no-session-persistence --strict-mcp-config --settings '{"disableAllHooks":true}'`,
  writes `{"type":"control_request","request_id":…,"request":{"subtype":"get_usage"}}` and
  reads the `control_response`. No prompt is sent. `response.rate_limits.limits[]` holds
  `{kind: weekly_all | weekly_scoped, percent, resets_at, scope.model.display_name}`.
  Claude marks this report experimental, so unknown kinds are shown under their own name.
  Asked every 5 minutes while a Claude session is live, and when the usage panel opens.
  The status line (above) updates the 5-hour and weekly numbers in between.
- **Codex** (verified against 0.169): asked directly through `codex app-server` (JSON-RPC
  over stdio): `initialize`, `initialized`, then `account/rateLimits/read`, which answers
  with `rateLimits.primary` / `.secondary` (`usedPercent`, `windowDurationMins`, `resetsAt`).
  No thread is started. Asked every 2 minutes while a Codex session is live, and when the
  usage panel opens. Fallback: every `token_count` event in a rollout carries
  `rate_limits.primary` / `.secondary` with `used_percent`, `window_minutes` (300 = 5 hours,
  10080 = a week) and `resets_at` in Unix seconds. Termlab reads the newest rollout.
- **Mistral Vibe** (3.0.3): the plan's name comes from Vibe's own cache of Mistral's
  `/api/vibe/whoami` (`~/.vibe/whoami_cache.json`, newest entry, `plan_name`). No usage
  percentage reaches Vibe. Mistral's documented Admin API has Vibe usage endpoints
  (`/v1/admin/analytics/vibe/usage/…`), but it is Enterprise-only, needs an admin key and
  returns token counts. The percentage is only on Mistral's signed-in web page
  (console.mistral.ai, Code › CLI › View organization usage), which Termlab links to.
- **Antigravity** (1.5.6): `agy models` answers "Please sign in…" when signed out, which is
  how Termlab tells. agy passes a `quota` object (per-model buckets with a used percentage
  and reset time) to a `statusLine` command set through `--settings`; that was seen in the
  binary and in a signed-out probe of the status-line input, but not yet with a signed-in
  account, so no percentages are read.
- **Sub-agent steps**: a Claude sub-agent's transcript marks every line `isSidechain`; a
  Codex sub-agent's rollout is read only if its `parent_thread_id` is the session's.

## Sessions survive a background-service restart

An agent runs in a terminal owned by `termlabd`, and a terminal can't outlive its owner. When
the service stops (quit, update, crash), it marks the sessions that were running, and its next
start resumes each one with the agent's own resume command, at its last terminal size. Attached
Claude background sessions are re-attached instead. Resuming only reopens the conversation and
sends no prompt. Sessions whose agent never saved a conversation (no session id) stay ended,
and the UI says why.

## Aider

Removed at the user's request (2026-11-13). It was never installed or verified.

## Environment

- Termlab passes the user's environment through and never sets API-key variables.
- A Finder-launched app has a bare `PATH`, so `termlabd` reads the login shell's environment
  once at startup (`$SHELL -l -i -c 'env -0'`, 8 s timeout, falling back to its own environment).
- **Added:**
  - `TERM=xterm-256color`
  - `COLORTERM=truecolor` if unset
  - `LANG=en_US.UTF-8` if no locale is set
  - `TERMLAB_SESSION_ID` and `TERMLAB_HOOK_SOCKET`
- **Removed:**
  - `TERM_PROGRAM*` and `*_SESSION_ID` variables from the terminal Termlab was started from
  - inherited agent-session markers (see Claude Code above)

## Side effects of the live tests

Answering the trust dialogs in the probe's scratch folders made the agents record those folders
as trusted in their own files (Claude Code in `~/.claude.json`, Codex in
`~/.codex/config.toml`). The agents wrote those entries, not Termlab, and the folders
have since been deleted. They are harmless, and Termlab doesn't edit those files.
