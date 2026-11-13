//! Antigravity CLI (`agy`), Google's successor to Gemini CLI. Verified against agy 1.5.6.
//!
//! Hooks are read only from `~/.gemini/config/hooks.json` (shared with the Antigravity
//! desktop app) or `.agents/hooks.json` in the project, and `--settings` doesn't carry them.
//! Termlab won't edit either, so status comes from the screen heuristics.

use std::path::Path;

use super::{
    AgentAdapter, CommandSpec, LaunchRequest, PlanPoll, PlanReport, StatusHints, run_capture,
};
use crate::env::UserEnv;
use crate::model::{AgentId, PermissionPreset};

pub struct AntigravityAdapter;

impl AntigravityAdapter {
    fn options(req: &LaunchRequest) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = req.model() {
            args.extend(["--model".into(), model.into()]);
        }
        match req.permission {
            // The default asks for every tool permission.
            PermissionPreset::AskFirst => {}
            PermissionPreset::AutoEdit => args.extend(["--mode".into(), "accept-edits".into()]),
            PermissionPreset::FullAuto => args.push("--dangerously-skip-permissions".into()),
        }
        if let Some(msg) = req.first_message() {
            // `-i` runs an initial prompt and stays interactive.
            args.extend(["--prompt-interactive".into(), msg.into()]);
        }
        args
    }
}

/// What `agy models` says about the account. Signed out, it answers "Please sign in to view
/// available models"; signed in, it lists them. (agy passes per-model quota to a status
/// line as `quota`, but that hasn't been checked against a signed-in account, so no
/// percentages are read yet.)
fn account_report(models_output: &str) -> PlanReport {
    let signed_out = models_output.to_lowercase().contains("sign in");
    PlanReport {
        windows: Vec::new(),
        at: crate::model::now_ms(),
        connected: Some(!signed_out),
        plan: None,
        note: Some(if signed_out {
            "Not signed in. Start Antigravity once and sign in to connect.".to_string()
        } else {
            "Signed in. Reading Antigravity's quota isn't supported yet.".to_string()
        }),
    }
}

impl AgentAdapter for AntigravityAdapter {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    fn plan_usage_poll(&self) -> Option<PlanPoll> {
        // `agy models` contacts Google: only while a session runs, or when the user asks.
        Some(PlanPoll {
            every_ms: 10 * 60 * 1000,
            only_while_live: true,
        })
    }

    fn plan_usage(&self, program: &Path, env: &UserEnv, _scratch: &Path) -> Option<PlanReport> {
        run_capture(program, &["models"], env).map(|out| account_report(&out))
    }

    fn binary_name(&self) -> &'static str {
        "agy"
    }

    fn api_key_vars(&self) -> &'static [&'static str] {
        &["GEMINI_API_KEY", "GOOGLE_API_KEY"]
    }

    fn can_resume(&self) -> bool {
        true
    }

    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        CommandSpec::new(req, Self::options(req), None)
    }

    /// `--conversation <ID>` resumes a previous conversation.
    fn resume(&self, agent_session_id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut args = vec!["--conversation".into(), agent_session_id.into()];
        args.extend(Self::options(req));
        Some(CommandSpec::new(req, args, Some(agent_session_id.into())))
    }

    fn status_hints(&self) -> StatusHints {
        StatusHints {
            // Unverified against a live prompt; only a full question, not bare words that
            // could appear in mode labels or ordinary output.
            needs_input: vec!["do you want to proceed?"],
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tells_signed_in_from_signed_out() {
        // What agy 1.5.6 prints when signed out.
        let out = "Fetching available models...\nError: Please sign in to view available models. Launch the CLI without arguments to sign in.\n";
        let report = super::account_report(out);
        assert_eq!(report.connected, Some(false));
        assert!(report.note.unwrap().starts_with("Not signed in"));
        let report =
            super::account_report("Fetching available models...\ngemini-3-pro\ngemini-3-flash\n");
        assert_eq!(report.connected, Some(true));
    }

    use super::*;
    use crate::model::SessionId;

    #[test]
    fn launch_args() {
        let mut req = LaunchRequest {
            session_id: SessionId::new(),
            program: "/bin/agy".into(),
            cwd: "/src".into(),
            model: Some("gemini-3-pro".into()),
            permission: PermissionPreset::AskFirst,
            first_message: Some("upgrade react".into()),
            hooks: None,
        };
        assert_eq!(
            AntigravityAdapter.launch(&req).args,
            [
                "--model",
                "gemini-3-pro",
                "--prompt-interactive",
                "upgrade react"
            ]
        );
        req.permission = PermissionPreset::FullAuto;
        req.first_message = None;
        req.model = None;
        assert_eq!(
            AntigravityAdapter.launch(&req).args,
            ["--dangerously-skip-permissions"]
        );
        req.permission = PermissionPreset::AutoEdit;
        assert_eq!(
            AntigravityAdapter.resume("c-1", &req).unwrap().args,
            ["--conversation", "c-1", "--mode", "accept-edits"]
        );
    }
}
