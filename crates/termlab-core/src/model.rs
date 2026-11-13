//! Plain data types shared by the core, the app and (later) the daemon.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;

/// Milliseconds since the Unix epoch. All timestamps in Termlab use this.
pub type Millis = i64;

pub fn now_ms() -> Millis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as Millis)
        .unwrap_or(0)
}

/// Termlab's own session id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SessionId {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| CoreError::InvalidInput(format!("not a session id: {s}")))
    }
}

pub type FolderId = i64;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = CoreError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok($name::$variant),)+
                    other => Err(CoreError::InvalidInput(format!(
                        concat!("unknown ", stringify!($name), ": {}"), other
                    ))),
                }
            }
        }
    };
}

string_enum!(
    /// The agent CLIs Termlab knows how to run.
    AgentId {
        Claude => "claude",
        Codex => "codex",
        Vibe => "vibe",
        Antigravity => "antigravity",
    }
);

impl AgentId {
    pub fn display_name(self) -> &'static str {
        match self {
            AgentId::Claude => "Claude Code",
            AgentId::Codex => "Codex",
            AgentId::Vibe => "Mistral Vibe",
            AgentId::Antigravity => "Antigravity",
        }
    }
}

string_enum!(
    /// What the user sees next to every session.
    SessionStatus {
        NeedsYou => "needs_you",
        Running => "running",
        Idle => "idle",
        Done => "done",
        Error => "error",
    }
);

impl SessionStatus {
    /// Done and Error are final: the process is gone.
    pub fn is_terminal(self) -> bool {
        matches!(self, SessionStatus::Done | SessionStatus::Error)
    }
}

string_enum!(
    /// Who pays for the session's model usage.
    Billing {
        Subscription => "subscription",
        ApiKey => "api_key",
        Unknown => "unknown",
    }
);

string_enum!(
    /// Where the agent runs. Worktrees and containers are out of scope for now.
    Isolation {
        Checkout => "checkout",
    }
);

string_enum!(
    /// How a session came into Termlab: started by it; imported from an agent's own history
    /// (no Termlab output until resumed); or attached to a background session the agent
    /// runs outside Termlab.
    SessionOrigin {
        Termlab => "termlab",
        Imported => "imported",
        Attached => "attached",
    }
);

string_enum!(
    /// Termlab's agent-neutral permission presets. Each adapter maps them to its own flags.
    PermissionPreset {
        AskFirst => "ask_first",
        AutoEdit => "auto_edit",
        FullAuto => "full_auto",
    }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    pub path: PathBuf,
    pub added_at: Millis,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: SessionId,
    pub folder_id: FolderId,
    pub agent: AgentId,
    pub title: String,
    pub model: Option<String>,
    pub permission_mode: Option<PermissionPreset>,
    pub isolation: Isolation,
    /// The agent's own session id (what its `--resume` takes), when known.
    pub agent_session_id: Option<String>,
    pub status: SessionStatus,
    pub exit_code: Option<i32>,
    pub created_at: Millis,
    pub last_activity_at: Millis,
    pub billing: Billing,
    /// `HEAD` when the session started, for "changed files" and the diff.
    pub start_commit: Option<String>,
    /// The agent's transcript file, when known (Claude reports it through hooks).
    pub transcript_path: Option<String>,
    /// Send a native notification when this session needs input.
    pub notify: bool,
    pub origin: SessionOrigin,
    /// The terminal size its output was last drawn at.
    pub term_size: Option<TermSize>,
    /// A colour the user gave the session inside the agent (Claude's `/color`): one of
    /// `SESSION_COLORS`.
    #[serde(default)]
    pub color: Option<String>,
}

/// The colour names a session can carry (Claude Code's `/color` choices).
pub const SESSION_COLORS: &[&str] = &[
    "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan",
];

/// What an approval button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalChoice {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOption {
    /// 1-based number shown by the agent.
    pub number: u8,
    pub label: String,
    pub choice: Option<ApprovalChoice>,
}

/// A pending permission request, read from hooks and/or the screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// The agent's question, e.g. "Do you want to proceed?".
    pub question: String,
    /// What it wants to do, e.g. "$ npm test" (from a hook, when available).
    pub detail: Option<String>,
    pub options: Vec<ApprovalOption>,
}

impl Approval {
    pub fn has(&self, choice: ApprovalChoice) -> bool {
        choice == ApprovalChoice::Deny || self.options.iter().any(|o| o.choice == Some(choice))
    }
}

/// Token use, from the agent's transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Tokens in the context window after the last turn.
    pub context_tokens: u64,
    /// Size of the context window, when known or estimated.
    pub context_window: Option<u64>,
    /// Input (uncached + cache writes) plus output tokens over the session.
    pub total_tokens: u64,
    /// Dollar cost — only computed for API-key billing with known prices.
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
}

impl Usage {
    pub fn context_percent(&self) -> Option<u8> {
        let w = self.context_window.filter(|w| *w > 0)?;
        Some(((self.context_tokens.saturating_mul(100)) / w).min(100) as u8)
    }
}

/// One limit of the user's plan: a rolling window and how much of it is used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// `five_hour`, `seven_day`, `seven_day:Fable` (a per-model week), `spend_limit`, …
    pub id: String,
    /// A name to show when the id isn't one the UI knows, e.g. "Current week (Fable)".
    #[serde(default)]
    pub label: Option<String>,
    /// 0 to 100 (above 100 once exceeded).
    pub used_percentage: f64,
    /// When the window resets.
    pub resets_at: Option<Millis>,
}

/// How much of the subscription plan's limits is used, as the agent last reported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUsage {
    pub agent: AgentId,
    pub windows: Vec<UsageWindow>,
    pub updated_at: Millis,
    /// Whether the agent is signed in to its service, when Termlab can tell.
    #[serde(default)]
    pub connected: Option<bool>,
    /// The plan's name as the service gives it ("max", "prolite", "Individual").
    #[serde(default)]
    pub plan: Option<String>,
    /// Something to say instead of percentages, e.g. why there are none.
    #[serde(default)]
    pub note: Option<String>,
}

/// A sub-agent an agent started inside a session: Claude Code's Agent/Task tool and
/// teammates, Codex's spawned agents, Vibe's child sessions. Read from the agent's own files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgent {
    pub id: String,
    pub name: String,
    /// What it was asked to do, when the agent records that.
    pub description: Option<String>,
    pub model: Option<String>,
    /// Still working (its transcript hasn't ended and was written to recently).
    pub running: bool,
    pub started_at: Option<Millis>,
    pub last_activity_at: Millis,
}

/// A session as the UI sees it: the stored record plus live state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    #[serde(flatten)]
    pub record: SessionRecord,
    pub live: bool,
    /// The last few non-empty screen lines (mission-control preview).
    pub preview: Vec<String>,
    pub approval: Option<Approval>,
    pub usage: Option<Usage>,
    pub context_percent: Option<u8>,
    /// API-key variables inherited from the user's environment (billing warning).
    pub api_key_vars: Vec<String>,
    /// Sub-agents of a running session: those working now and those that just finished.
    #[serde(default)]
    pub subagents: Vec<SubAgent>,
}

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl Default for TermSize {
    fn default() -> Self {
        Self {
            cols: 120,
            rows: 32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_strings() {
        for a in AgentId::ALL {
            assert_eq!(a.as_str().parse::<AgentId>().unwrap(), *a);
        }
        for s in SessionStatus::ALL {
            assert_eq!(s.as_str().parse::<SessionStatus>().unwrap(), *s);
        }
        assert!("nope".parse::<AgentId>().is_err());
        assert_eq!(
            serde_json::to_string(&SessionStatus::NeedsYou).unwrap(),
            "\"needs_you\""
        );
    }
}
