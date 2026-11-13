// Development only: when the UI runs in a plain browser (`npm run dev`, no Tauri), mock the
// daemon with sample data so screens can be checked visually. Never loaded inside the app.

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Channel } from "@tauri-apps/api/core";
import type { AgentInfo, Folder, Session } from "./api";

const now = Date.now();
const min = 60_000;

const folders: Folder[] = [
  { id: 1, name: "billing-api", path: "/Users/demo/code/billing-api", addedAt: now, sortOrder: 0 },
  { id: 2, name: "web-dashboard", path: "/Users/demo/clients/northwind/web-dashboard", addedAt: now, sortOrder: 1 },
  { id: 3, name: "infra", path: "/Users/demo/work/platform/infra", addedAt: now, sortOrder: 2 },
];

function session(p: Partial<Session> & Pick<Session, "id" | "folderId" | "agent" | "title" | "status">): Session {
  return {
    model: null,
    permissionMode: "ask_first",
    isolation: "checkout",
    agentSessionId: "a41f0c2e-0000-0000-0000-00000000c9c2",
    exitCode: null,
    createdAt: now - 30 * min,
    lastActivityAt: now - 2 * min,
    billing: "subscription",
    startCommit: "abc123",
    transcriptPath: null,
    notify: true,
    origin: "termlab",
    live: true,
    preview: [],
    approval: null,
    usage: { contextTokens: 124_000, contextWindow: 1_000_000, totalTokens: 184_000, costUsd: null, model: "claude-opus-5-5" },
    contextPercent: 12,
    apiKeyVars: [],
    ...p,
  };
}

const sessions: Session[] = [
  session({
    id: "s1", folderId: 1, agent: "claude", title: "Move invoice retries to a queue worker", status: "needs_you",
    color: "purple",
    subagents: [
      { id: "a1", name: "test-runner", description: "Run the billing test suite", model: "sonnet", running: true, startedAt: Date.now() - 190_000, lastActivityAt: Date.now() - 4_000 },
      { id: "a2", name: "Explore", description: "Find every caller of retryInvoice", model: "haiku", running: true, startedAt: Date.now() - 45_000, lastActivityAt: Date.now() - 1_000 },
      { id: "a3", name: "code-reviewer", description: "Review the queue worker diff", model: "opus", running: false, startedAt: Date.now() - 600_000, lastActivityAt: Date.now() - 240_000 },
    ],
    model: "opus",
    preview: ["● Write migrations/0042_retry_queue.sql", "● Bash npm test -- billing", "  ✓ 41 passed · 0 failed", "Do you want to proceed?"],
    approval: {
      question: "Do you want to proceed?",
      detail: "$ psql -d billing_dev -f migrations/0042_retry_queue.sql",
      options: [
        { number: 1, label: "Yes", choice: "allow_once" },
        { number: 2, label: "Yes, and don't ask again for psql commands", choice: "allow_always" },
        { number: 3, label: "No", choice: "deny" },
      ],
    },
    contextPercent: 62,
  }),
  session({
    id: "s2", folderId: 1, agent: "codex", title: "Idempotency keys for webhook handlers", status: "running",
    subagents: [
      { id: "c1", name: "McClintock", description: "webhook signature audit", model: null, running: true, startedAt: Date.now() - 80_000, lastActivityAt: Date.now() - 2_000 },
    ],
    model: "gpt-6-sol", lastActivityAt: now,
    preview: ["• Edited src/db/idempotency.ts (+58)", "• Edited src/webhooks/handler.ts (+31 −12)", "• Running npm test -- webhooks", "  Working (12s • esc to interrupt)"],
    contextPercent: 38,
  }),
  session({
    id: "s3", folderId: 2, agent: "vibe", title: "Dark mode for settings pages", status: "running",
    preview: ["◆ write_file styles/tokens.css", "◆ search_replace billing/page.tsx (9 edits)", "◆ Checking contrast on 14 components…"],
    contextPercent: 54, billing: "api_key", apiKeyVars: ["MISTRAL_API_KEY"],
    usage: { contextTokens: 216_000, contextWindow: 400_000, totalTokens: 121_000, costUsd: 0.42, model: "mistral-large-4" },
  }),
  session({
    id: "s4", folderId: 2, agent: "claude", title: "Fix flaky checkout e2e test", status: "done", live: false,
    color: "green",
    exitCode: 0, lastActivityAt: now - 18 * min, preview: ["  ✓ 20 of 20 passed", "Done. Waits on the network response now."],
    contextPercent: 41,
  }),
  session({
    id: "s5", folderId: 2, agent: "antigravity", title: "Upgrade to React 19", status: "error", live: false,
    exitCode: 1, lastActivityAt: now - 25 * min, preview: ["  npm ERR! peer react@\"^18\" from react-beautiful-dnd", "✕ Process exited (code 1)"],
    contextPercent: 88, usage: null,
  }),
  session({
    id: "s6", folderId: 3, agent: "codex", title: "Split prod and staging Terraform state", status: "idle",
    lastActivityAt: now - 60 * min, preview: ["• Proposal: 3 steps using terraform state mv", "Waiting for your next message."],
    contextPercent: 22,
  }),
];

const agents: AgentInfo[] = [
  { agent: "claude", name: "Claude Code", binary: "claude", installed: { agent: "claude", path: "/Users/demo/.local/bin/claude", version: "2.1.322", loginBilling: "subscription" }, modelFlag: true, modelSuggestions: ["opus", "sonnet", "haiku", "fable"], canResume: true, canFork: true, hooks: true, verified: true },
  { agent: "codex", name: "Codex", binary: "codex", installed: { agent: "codex", path: "/Users/demo/.local/bin/codex", version: "0.167.0", loginBilling: "subscription" }, modelFlag: true, modelSuggestions: ["gpt-6.1-sol", "gpt-6-astra"], canResume: true, canFork: true, hooks: true, verified: true },
  { agent: "vibe", name: "Mistral Vibe", binary: "vibe", installed: { agent: "vibe", path: "/Users/demo/.local/bin/vibe", version: "3.0.3", loginBilling: "unknown" }, modelFlag: false, modelSuggestions: [], canResume: false, canFork: false, hooks: false, verified: true },
  { agent: "antigravity", name: "Antigravity", binary: "agy", installed: { agent: "antigravity", path: "/Users/demo/.local/bin/agy", version: "1.5.6", loginBilling: "unknown" }, modelFlag: true, modelSuggestions: [], canResume: true, canFork: false, hooks: false, verified: true },
];

const DIFF = `diff --git a/src/billing/retry.ts b/src/billing/retry.ts
index 1111111..2222222 100644
--- a/src/billing/retry.ts
+++ b/src/billing/retry.ts
@@ -10,7 +10,7 @@ export async function scheduleRetry(id: string, attempt: number) {
   const backoff = computeBackoff(attempt);
   log.info("retrying invoice", { id, attempt });
-  setTimeout(() => retryInvoice(id), backoff);
+  await retryQueue.add("invoice", { id, attempt }, { delay: backoff });
   return backoff;
 }
`;

const SCREEN =
  "\x1bc\x1b[8;24;100t\x1b[1m› Move invoice retries off setTimeout into a queue-backed worker.\x1b[0m\r\n\r\n" +
  "\x1b[90m● Read src/billing/retry.ts (214 lines)\x1b[0m\r\n" +
  "\x1b[34m● Edit src/billing/retry.ts\x1b[0m\r\n" +
  "   \x1b[31m- setTimeout(() => retryInvoice(id), backoff(attempt))\x1b[0m\r\n" +
  "   \x1b[32m+ await retryQueue.add(\"invoice\", { id, attempt }, { delay: backoff(attempt) })\x1b[0m\r\n" +
  "\x1b[90m● Bash npm test -- billing\x1b[0m\r\n   \x1b[32m✓ 41 passed · 0 failed (3.8s)\x1b[0m\r\n\r\n" +
  "\x1b[33m Do you want to proceed?\x1b[0m\r\n ❯ 1. Yes\r\n   2. Yes, and don't ask again for psql commands\r\n   3. No\r\n";

export function install() {
  const envFiles = new Map<string, string>();
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      if (cmd === "read_project_env") return envFiles.get(a.cwd as string) ?? null;
      if (cmd === "save_project_env") {
        if ((envFiles.get(a.cwd as string) ?? null) !== a.original) throw "The .env file changed on disk. Reopen it before saving.";
        envFiles.set(a.cwd as string, a.content as string);
        return null;
      }
      if (cmd === "daemon_status") return { connected: true, message: null };
      if (cmd === "attach_session") {
        const ch = a.onOutput as Channel<ArrayBuffer>;
        setTimeout(() => ch.onmessage(new TextEncoder().encode(SCREEN).buffer as ArrayBuffer), 50);
        return { exited: null };
      }
      if (cmd === "detach_session" || cmd === "open_in_terminal" || cmd === "youtube_live_video") return null;
      if (cmd === "system_volume") return 24;
      if (cmd.startsWith("youtube_music_") || cmd === "open_external" || cmd === "set_system_volume") return null;
      if (cmd.startsWith("plugin:notification")) return true;
      if (cmd !== "daemon_call") return null;
      const req = a.request as { method: string; params?: Record<string, unknown> };
      const id = req.params?.id as string | undefined;
      switch (req.method) {
        case "listFolders": return [...folders];
        case "moveFolder": {
          const source = folders.findIndex((f) => f.id === req.params?.id);
          const before = req.params?.before;
          if (source < 0 || (before !== null && !folders.some((f) => f.id === before))) throw "Project no longer exists";
          if (before === req.params?.id) return null;
          const [folder] = folders.splice(source, 1);
          const target = before === null ? folders.length : folders.findIndex((f) => f.id === before);
          folders.splice(target, 0, folder);
          folders.forEach((f, i) => { f.sortOrder = i; });
          return null;
        }
        case "listSessions": return sessions;
        case "session": {
          const session = sessions.find((s) => s.id === id);
          if (!session) throw "Session not found";
          return { ...session };
        }
        case "stop":
        case "restart": {
          const session = sessions.find((s) => s.id === id);
          if (!session) throw "Session not found";
          session.live = req.method === "restart";
          session.status = session.live ? "running" : "done";
          session.exitCode = session.live ? null : 0;
          return req.method === "restart" ? { ...session } : null;
        }
        case "agents": case "refreshAgents": return agents;
        case "planUsage": case "refreshPlanUsage": return [
          {
            agent: "claude", connected: true, plan: "max", note: null,
            updatedAt: Date.now() - 95_000,
            windows: [
              { id: "seven_day", label: null, usedPercentage: 66, resetsAt: Date.now() + 26 * 3_600_000 },
              { id: "seven_day:Fable", label: "Current week (Fable)", usedPercentage: 29, resetsAt: Date.now() + 26 * 3_600_000 },
            ],
          },
          {
            agent: "codex", connected: true, plan: "prolite", note: null,
            updatedAt: Date.now() - 40_000,
            windows: [{ id: "seven_day", label: null, usedPercentage: 0, resetsAt: Date.now() + 7 * 24 * 3_600_000 }],
          },
          {
            agent: "vibe", connected: true, plan: "Individual", updatedAt: Date.now() - 30_000, windows: [],
            note: null,
          },
          {
            agent: "antigravity", connected: false, plan: null, updatedAt: Date.now() - 30_000, windows: [],
            note: "Not signed in. Start Antigravity once and sign in to connect.",
          },
        ];
        case "subagentTimeline": return [
          { at: Date.now() - 180_000, kind: "prompt", text: "Run the billing test suite and report any failures." },
          { at: Date.now() - 170_000, kind: "tool", text: "$ npm test -- billing" },
          { at: Date.now() - 60_000, kind: "tool", text: "Read src/billing/retry.test.ts" },
          { at: Date.now() - 20_000, kind: "message", text: "41 passed, 0 failed. The retry queue tests cover the new worker." },
        ];
        case "details": {
          const s = sessions.find((x) => x.id === id);
          return {
            cwd: folders[0].path, branch: "feat/retry-queue",
            files: [
              { path: "src/billing/retry.ts", added: 48, deleted: 71, untracked: false },
              { path: "src/workers/invoice-retry.worker.ts", added: 112, deleted: 0, untracked: true },
              { path: "migrations/0042_retry_queue.sql", added: 24, deleted: 0, untracked: true },
            ],
            usage: s?.usage ?? null, contextPercent: s?.contextPercent ?? null,
            resumeCommand: "cd ~/code/billing-api && claude --resume a41f…c9c2", canFork: true, canResume: true,
          };
        }
        case "diff": return DIFF;
        case "timeline": return [
          { at: now - 9 * min, kind: "prompt", text: "Move invoice retries off setTimeout into a queue-backed worker." },
          { at: now - 8 * min, kind: "tool", text: "Read src/billing/retry.ts" },
          { at: now - 6 * min, kind: "tool", text: "$ npm test -- billing" },
          { at: now - 2 * min, kind: "hook", text: "PermissionRequest: Asked permission: $ psql -d billing_dev -f migrations/0042_retry_queue.sql" },
        ];
        case "discover": {
          const base = [
            { agent: "claude", agentSessionId: "x1", title: "Add rate limiting to public API", cwd: folders[0].path, lastActivityAt: now - 20 * 60 * min, path: "", folderId: 1, termlabSession: null, runningElsewhere: false },
            { agent: "codex", agentSessionId: "x2", title: "Terraform state split proposal", cwd: folders[2].path, lastActivityAt: now - 3 * 60 * min, path: "", folderId: 3, termlabSession: null, runningElsewhere: false },
          ];
          if (!req.params?.everywhere) return base;
          return [
            ...base,
            { agent: "claude", agentSessionId: "x3", title: "Customer sync integration", cwd: "/Users/demo/code/crm-sync", lastActivityAt: now - 2 * 60 * min, path: "", folderId: null, termlabSession: null, runningElsewhere: true },
            { agent: "vibe", agentSessionId: "x4", title: "Hi", cwd: "/Users/demo/Documents/aitest", lastActivityAt: now - 40 * 24 * 60 * min, path: "", folderId: null, termlabSession: null, runningElsewhere: false },
          ];
        }
        case "external": return [
          { agent: "claude", agentSessionId: "x3", shortId: "a1b2c3d4", name: "crm-sync", cwd: "/Users/demo/code/crm-sync", kind: "background", status: "needs_you", startedAt: now - 60 * min, folderId: null, attachable: true, termlabSession: null },
          { agent: "claude", agentSessionId: "x5", shortId: "e5f6a7b8", name: "server-management", cwd: "/Users/demo/work/server-management", kind: "interactive", status: "running", startedAt: now - 30 * min, folderId: null, attachable: false, termlabSession: null },
        ];
        case "import": return { imported: 2, alreadyPresent: 0, foldersAdded: 2, skipped: 0 };
        case "previewCommand": return "cd /Users/demo/code/billing-api && claude --session-id 3f2c…e1 --permission-mode manual --settings '/Users/demo/Library/Application Support/dev.termlab.Termlab/hooks/claude-settings.json'";
        case "handoffMessage": return "You're taking over a task that Claude Code was working on in this folder.\n\nGoal: Move invoice retries to a queue worker\n\nFiles changed so far:\n- src/billing/retry.ts (+48 -71)";
        default: return null;
      }
    },
    { shouldMockEvents: true },
  );
}
