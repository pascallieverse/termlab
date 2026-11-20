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
    /** The project folder's path on the machine (Termlab 0.2.9+); its notes are keyed by it. */
    directory?: string | null;
    /** The project's place in Termlab's sidebar, and when the session started (0.2.9+). */
    project_order?: number | null;
    created_at?: number | null;
    /** The project folder's id on its machine (0.2.9+). */
    folder_id?: number | null;
    /** Put on hold in Termlab: listed under On hold only (0.2.9+). */
    on_hold?: boolean;
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
    /** Its Termlab syncs project notes. */
    notes_sync?: boolean;
};

/** A project folder on a machine, in Termlab's sidebar order (0.2.9+). */
export type Project = { id: number; name: string; path: string; order: number };

/** An agent installed on a machine, that sessions can be started with. */
export type AgentChoice = { agent: string; name: string; model_flag: boolean; models: string[] };

export type MachineProjects = { machine_id: number; projects: Project[]; agents: AgentChoice[] };

export type PermissionPreset = 'ask_first' | 'auto_edit' | 'full_auto';

export type StartSessionRequest = {
    folder_id: number;
    agent: string;
    permission: PermissionPreset;
    model?: string | null;
    message?: string | null;
};

/** A project's notes, as the machine last reported them. */
export type ProjectNotes = {
    machine_id: number;
    path: string;
    text: string;
    updated_at: number | null;
};

export type UsageWindow = {
    id: string;
    label?: string | null;
    usedPercentage: number;
    resetsAt: number | null;
};

/** An agent's subscription plan usage, as Termlab reports it (camelCase, as in the app). */
export type PlanUsage = {
    agent: string;
    windows: UsageWindow[];
    updatedAt: number;
    connected?: boolean | null;
    plan?: string | null;
    note?: string | null;
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
