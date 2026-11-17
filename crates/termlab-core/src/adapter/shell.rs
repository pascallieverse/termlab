//! A plain terminal: the user's own shell in the project folder, no agent. It runs like any
//! other session (status from the screen, stop, restart), so it sits next to the agents in
//! the New session dialog.

use std::path::PathBuf;

use super::{AgentAdapter, CommandSpec, Installed, LaunchRequest};
use crate::env::UserEnv;
use crate::model::{AgentId, Billing};

pub struct ShellAdapter;

impl ShellAdapter {
    /// The user's login shell (`$SHELL`), else the platform's default.
    fn shell(env: &UserEnv) -> Option<PathBuf> {
        if cfg!(windows) {
            return env
                .which("pwsh")
                .or_else(|| env.which("powershell"))
                .or_else(|| env.get("COMSPEC").map(PathBuf::from))
                .or_else(|| env.which("cmd"));
        }
        let fallback = if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else {
            "/bin/sh"
        };
        env.get("SHELL")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| Some(PathBuf::from(fallback)).filter(|p| p.is_file()))
    }
}

impl AgentAdapter for ShellAdapter {
    fn id(&self) -> AgentId {
        AgentId::Shell
    }

    /// Shown in the dialog; the real program comes from `detect`.
    fn binary_name(&self) -> &'static str {
        if cfg!(windows) {
            "PowerShell"
        } else {
            "$SHELL"
        }
    }

    fn api_key_vars(&self) -> &'static [&'static str] {
        &[]
    }

    fn detect(&self, env: &UserEnv) -> Option<Installed> {
        Some(Installed {
            agent: self.id(),
            path: Self::shell(env)?,
            version: None,
            login_billing: Billing::Unknown,
        })
    }

    fn model_flag(&self) -> bool {
        false
    }

    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        // A login shell, as a terminal app opens one (PowerShell and cmd take no such flag).
        let args = if cfg!(windows) {
            Vec::new()
        } else {
            vec!["-l".to_string()]
        };
        CommandSpec::new(req, args, None)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::model::{PermissionPreset, SessionId};

    #[test]
    fn opens_the_users_login_shell_in_the_folder() {
        let env = UserEnv::from_vars([("SHELL", "/bin/sh"), ("PATH", "/usr/bin:/bin")]);
        let installed = ShellAdapter.detect(&env).expect("a shell");
        assert_eq!(installed.path, PathBuf::from("/bin/sh"));
        let spec = ShellAdapter.launch(&LaunchRequest {
            session_id: SessionId::new(),
            program: installed.path,
            cwd: PathBuf::from("/tmp/project"),
            model: Some("ignored".into()),
            permission: PermissionPreset::FullAuto,
            first_message: Some("ignored".into()),
            agent_permission_mode: None,
            hooks: None,
        });
        assert_eq!(spec.args, ["-l"]);
        assert_eq!(spec.display(), "cd /tmp/project && sh -l");
    }
}
