// The shapes in docs/server-protocol.md.

export type SessionStatus = 'needs_you' | 'running' | 'idle' | 'done' | 'error';

export type ApprovalChoice = 'allow_once' | 'allow_always' | 'deny';

export type Approval = {
    kind: 'permission' | 'question';
    question: string;
    detail: string | null;
    options: { choice: ApprovalChoice | null; label: string }[];
};

export type ChangedFile = { path: string; added: number; removed: number };

export type RemoteSession = {
    id: string;
    machine_id: number;
    machine: string;
    project: string;
    title: string;
    agent: string;
    model: string | null;
    branch: string | null;
    status: SessionStatus;
    approval: Approval | null;
    preview: string[];
    tokens: number | null;
    context_pct: number | null;
    changed_files: ChangedFile[];
    status_changed_at: string | null;
    updated_at: string;
};

export type Machine = {
    id: number;
    name: string;
    os: string | null;
    version: string | null;
    online: boolean;
    last_seen_at: string | null;
    sessions: number;
    running: number;
};

export type WebhookEndpoint = {
    id: number;
    url: string;
    description: string | null;
    events: string[];
    enabled: boolean;
    health: 'healthy' | 'failing' | 'disabled';
    last_delivery_at: string | null;
};

export type WebhookDelivery = {
    id: number;
    event: string;
    response_code: string | null;
    ok: boolean;
    pending: boolean;
    attempt: number;
    duration_ms: number | null;
    created_at: string;
    delivered_at: string | null;
};

export type ServerState = {
    setup_required: boolean;
    signed_in: boolean;
    version: string;
    server_name: string;
    vapid_public_key: string | null;
};

export type TotpEnrolment = {
    totp_secret: string;
    otpauth_url: string;
    qr_svg: string;
};

export type DeviceSession = {
    id: number;
    user_agent: string | null;
    ip: string | null;
    created_at: string;
    last_seen_at: string | null;
    current: boolean;
};

export type AuditEntry = {
    id: number;
    action: string;
    detail: string | null;
    machine: string | null;
    session_title: string | null;
    ip: string | null;
    created_at: string;
};
