# Termlab server: API and protocol

`termlab-server` is a self-hosted server that gives one person remote access to their own
Termlab computers (browser and phone) and sends webhooks when sessions change. Everything
here is served by one binary (`crates/termlab-server`). The web UI (`apps/server-web`) is
embedded in it and uses only the API below.

All JSON uses snake_case. Times are ISO 8601 strings in UTC. Errors are
`{ "message": "…" }` with a 4xx/5xx status.

## Shapes

```ts
type SessionStatus = "needs_you" | "running" | "idle" | "done" | "error";
type ApprovalChoice = "allow_once" | "allow_always" | "deny";

type Approval = {
  kind: "permission" | "question";
  question: string;
  detail: string | null;
  options: { choice: ApprovalChoice | null; label: string }[];
};

type SessionView = {
  id: string;               // Termlab's session uuid
  machine_id: number;
  machine: string;          // machine name
  project: string;
  title: string;
  agent: string;            // claude | codex | vibe | antigravity | shell
  model: string | null;
  branch: string | null;
  status: SessionStatus;
  approval: Approval | null;
  preview: string[];        // last ≤12 screen lines
  tokens: number | null;
  context_pct: number | null;
  changed_files: { path: string; added: number; removed: number }[];
  status_changed_at: string | null;
  updated_at: string;
};

type Machine = {
  id: number;
  name: string;
  os: string | null;
  version: string | null;   // Termlab version
  online: boolean;
  last_seen_at: string | null;
  sessions: number;
  running: number;          // running + needs_you
};

type WebhookEndpoint = {
  id: number; url: string; description: string | null; events: string[];
  enabled: boolean; health: "healthy" | "failing" | "disabled"; last_delivery_at: string | null;
};

type WebhookDelivery = {
  id: number; event: string; response_code: string | null; ok: boolean; pending: boolean;
  attempt: number; duration_ms: number | null; created_at: string; delivered_at: string | null;
};
```

## Browser API

Signed-in requests carry the `termlab_session` cookie (HttpOnly, SameSite=Strict, Secure
unless the public URL is plain http). Every request other than GET must send the header
`X-Termlab-Request: 1` (CSRF guard); without it the server answers 403.

### Setup and sign-in

| Request | Answer |
|---|---|
| `GET /api/state` | `{ setup_required, signed_in, version, server_name, vapid_public_key }` |
| `POST /api/setup/start { setup_token }` | `{ totp_secret, otpauth_url, qr_svg }`. Only while no owner exists; the setup token is printed to the server's console/log at start. |
| `POST /api/setup/finish { setup_token, password, totp_code }` | 204 + session cookie. Password ≥ 12 characters. |
| `POST /api/login { password, totp_code }` | 204 + cookie, 401 `{message}`, 429 after 5 failures per IP per 15 min |
| `POST /api/logout` | 204 |

### Remote

| Request | Answer |
|---|---|
| `GET /api/overview` | `{ machines: Machine[], sessions: SessionView[] }` |
| `POST /api/sessions/{id}/answer { choice }` | 202 `{ command_id }`; 409 `{message}` when not waiting or machine offline |
| `POST /api/sessions/{id}/message { text }` | 202 / 409 |
| `POST /api/sessions/{id}/stop` | 202 / 409 |
| `POST /api/sessions/{id}/hold` | 202 / 409 |

Each of these is written to the audit log before the command goes to the machine.

### Machines

| Request | Answer |
|---|---|
| `GET /api/machines` | `Machine[]` |
| `POST /api/pairing-codes` | `{ code: "K7QX-2MPD", expires_at }` (10 minutes, single use; a new code replaces the previous one) |
| `PATCH /api/machines/{id} { name }` | `Machine` |
| `DELETE /api/machines/{id}` | 204. Revokes its token and disconnects it. |

### Webhooks

| Request | Answer |
|---|---|
| `GET /api/webhooks` | `{ endpoints: WebhookEndpoint[], events: { name, description }[] }` |
| `POST /api/webhooks { url, description?, events[] }` | `WebhookEndpoint` (422 `{message}` on a bad URL) |
| `PATCH /api/webhooks/{id} { url?, description?, events?, enabled? }` | `WebhookEndpoint` |
| `DELETE /api/webhooks/{id}` | 204 |
| `POST /api/webhooks/{id}/secret` | `{ secret }` (shown on request only) |
| `POST /api/webhooks/{id}/roll` | `{ secret }` |
| `POST /api/webhooks/{id}/test` | 202 |
| `GET /api/webhooks/{id}/deliveries` | `WebhookDelivery[]` (newest 50) |
| `POST /api/deliveries/{id}/redeliver` | 202 |

Events: `session.needs_you`, `approval.requested`, `session.done`, `session.error`,
`session.started`, `machine.offline`, plus `webhook.test`.

Deliveries are `POST` with JSON
`{ id, event, created_at, program: "termlab", machine, project?, agent?, reason?, session?, approval? }`
and headers `X-Termlab-Event`, `X-Termlab-Delivery` (the event id) and
`X-Termlab-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, "<t>.<body>")>`.
Private and loopback addresses are refused unless the server runs with
`--allow-private-webhooks`.

### Account

| Request | Answer |
|---|---|
| `POST /api/account/password { current_password, new_password, totp_code }` | 204; signs out every other session |
| `POST /api/account/totp/start` | `{ totp_secret, otpauth_url, qr_svg }` (a new secret, pending) |
| `POST /api/account/totp/finish { password, totp_code }` | 204 (switches to the pending secret) |
| `GET /api/account/sessions` | `{ id, user_agent, ip, created_at, last_seen_at, current }[]` |
| `DELETE /api/account/sessions/{id}` | 204 |
| `GET /api/audit?before={id}` | `{ entries: { id, action, detail, machine, session_title, ip, created_at }[], next: number \| null }` (50 per page) |

### Push notifications

| Request | Answer |
|---|---|
| `GET /api/push` | `{ public_key, subscriptions: { id, label, created_at, endpoint }[] }` (compare `endpoint` with this browser's subscription to find the current device) |
| `POST /api/push { endpoint, keys: { p256dh, auth }, label }` | 201 `{ id }` |
| `DELETE /api/push/{id}` | 204 |
| `POST /api/push/test` | 202 |

The server sends a push for `session.needs_you` and `session.error`, at most one per session
per minute, as JSON `{ title, body, url }` (url: `/remote?session=<id>`).

### Live updates: `GET /api/ws` (websocket, cookie auth)

Server → browser:
- `{ "type": "session.updated", "session": SessionView }`
- `{ "type": "session.removed", "id": "…" }`
- `{ "type": "machine.updated", "machine": Machine }`
- `{ "type": "output", "id": "…", "data": "<base64>", "snapshot": true|false, "cols": 120|null, "rows": 32|null }`.
  A snapshot resets the terminal: resize to `cols`×`rows` (when given), clear, then write.
- `{ "type": "watch", "id": "…", "live": true|false }`. Whether the machine is streaming
  that terminal now.

Browser → server:
- `{ "type": "watch", "id": "…", "on": true|false }`: open or close a terminal view.
- `{ "type": "input", "id": "…", "data": "<base64>" }`: keystrokes for a watched terminal.

## Machine API (termlabd)

### Pairing

`POST /api/pair { code, name, os, version }` → 201 `{ machine_id, token, server_name }`.
422 `{message}` for a wrong or expired code. The token is sent as
`Authorization: Bearer <token>` from then on. The server stores only its SHA-256.

`DELETE /api/machine` (bearer token) → 204: the computer unpairs itself.

### `GET /api/machine/ws` (websocket, bearer token)

Machine → server:
- `{ "type": "hello", "protocol": 2, "version": "0.2.7", "os": "macOS 15 · Apple Silicon" }` (first message)
- `{ "type": "snapshot", "sessions": SessionView[] }`: replaces the machine's sessions.
  The fields `machine_id`, `machine`, `status_changed_at` and `updated_at` are filled in by
  the server and may be omitted.
- `{ "type": "session", "session": SessionView }`: upsert one session
- `{ "type": "removed", "id": "…" }`
- `{ "type": "output", "id": "…", "data": "<base64>", "snapshot": bool, "cols": n|null, "rows": n|null }`

Server → machine:
- `{ "type": "command", "id": "<uuid>", "command": "answer" | "message" | "stop" | "hold", "session_id": "…", "data": { "choice"?: ApprovalChoice, "text"?: string } }`
- `{ "type": "watch", "id": "…", "on": bool }`: start streaming (snapshot first) or stop.
- `{ "type": "input", "id": "…", "data": "<base64>" }`: keystrokes from a viewer.
- `{ "type": "bye", "reason": "…" }`: the token was revoked; forget it.

A machine is online while its websocket is open (pings every 25 s). If it stays
disconnected for 90 s, `machine.offline` fires. A closed connection answered with 401 means
the token was revoked: the machine unpairs itself.
