//! The environment agents are launched with.
//!
//! Termlab passes the user's environment through unchanged (hard requirement: never inject
//! API keys). A macOS app started from Finder or the Dock does not inherit the user's shell
//! environment, though — its PATH is just `/usr/bin:/bin:/usr/sbin:/sbin`, so `claude` in
//! `~/.local/bin` would not be found. So we ask the user's login shell for its environment
//! once at startup and use that.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::model::Billing;

/// Variables that, when present, make an agent bill to an API key instead of the user's
/// subscription login. Termlab never sets these; it only warns when they are inherited.
pub const API_KEY_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "MISTRAL_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
];

/// Variables that identify the *process that launched Termlab* when that was itself an
/// agent session (e.g. Termlab started from a terminal inside Claude Code). Passing them on
/// would make every agent think it is that session's child — Claude Code then turns off
/// transcript saving — and would hand out the parent's messaging token. They are removed;
/// configuration variables (models, providers, feature flags) are kept.
const INHERITED_SESSION_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_JOB_DIR",
    "CLAUDE_PID",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "CODEX_THREAD_ID",
    "TERMLAB_SESSION_ID",
    "TERMLAB_HOOK_SOCKET",
];

const MARKER: &str = "__TERMLAB_ENV_BEGIN__";
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEnv {
    vars: BTreeMap<String, String>,
}

impl UserEnv {
    /// This process's own environment.
    pub fn from_current() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    pub fn from_vars<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// The login shell's environment, falling back to this process's environment if the
    /// shell can't be run or takes too long.
    pub fn capture_login_shell() -> Self {
        let current = Self::from_current();
        let shell = current
            .get("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
        match capture_from_shell(&shell, CAPTURE_TIMEOUT) {
            Ok(vars) if vars.contains_key("PATH") => {
                tracing::info!(shell = %shell.display(), count = vars.len(), "captured login shell environment");
                Self { vars }
            }
            Ok(_) => {
                tracing::warn!("login shell environment had no PATH; using process environment");
                current
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not capture login shell environment; using process environment");
                current
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn path_var(&self) -> Option<OsString> {
        self.get("PATH").map(OsString::from)
    }

    /// Finds an executable on this environment's PATH.
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        let cwd = self
            .get("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        which::which_in(program, self.path_var(), cwd).ok()
    }

    /// API-key variables present (non-empty) in this environment.
    pub fn api_key_vars(&self) -> Vec<&'static str> {
        API_KEY_VARS
            .iter()
            .copied()
            .filter(|k| self.get(k).is_some_and(|v| !v.trim().is_empty()))
            .collect()
    }

    /// The final environment for a spawned agent: the user's environment unchanged, plus
    /// terminal variables a GUI-launched process lacks, plus the adapter's additions.
    pub fn for_spawn(&self, additions: &[(String, String)]) -> BTreeMap<String, String> {
        let mut env = self.vars.clone();
        // Our pty is an xterm.js terminal; GUI apps have no TERM at all.
        env.insert("TERM".into(), "xterm-256color".into());
        env.entry("COLORTERM".into())
            .or_insert_with(|| "truecolor".into());
        if !["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .any(|k| env.get(*k).is_some_and(|v| !v.is_empty()))
        {
            env.insert("LANG".into(), "en_US.UTF-8".into());
        }
        // These describe the terminal Termlab itself was started from, not ours.
        for k in [
            "TERM_PROGRAM",
            "TERM_PROGRAM_VERSION",
            "TERM_SESSION_ID",
            "ITERM_SESSION_ID",
        ] {
            env.remove(k);
        }
        for k in INHERITED_SESSION_MARKERS {
            env.remove(*k);
        }
        for (k, v) in additions {
            env.insert(k.clone(), v.clone());
        }
        env
    }
}

/// Billing implied by the environment for a given agent's key variables.
pub fn billing_for(env: &UserEnv, key_vars: &[&str]) -> Billing {
    if key_vars
        .iter()
        .any(|k| env.get(k).is_some_and(|v| !v.trim().is_empty()))
    {
        Billing::ApiKey
    } else {
        Billing::Unknown
    }
}

fn capture_from_shell(
    shell: &Path,
    timeout: Duration,
) -> std::io::Result<BTreeMap<String, String>> {
    // -l loads the login profile, -i the interactive rc files (where most people set PATH).
    // The marker separates the env dump from anything the rc files print.
    let mut child = Command::new(shell)
        .args([
            "-l",
            "-i",
            "-c",
            &format!("printf '{MARKER}'; /usr/bin/env -0"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "login shell did not finish in time",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = reader.join().unwrap_or_default();
    Ok(parse_env_dump(&out))
}

fn parse_env_dump(out: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(out);
    let Some(start) = text.find(MARKER) else {
        return BTreeMap::new();
    };
    text[start + MARKER.len()..]
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_dump_after_marker_only() {
        let dump = format!("motd junk\n{MARKER}PATH=/a:/b\0HOME=/h\0MULTI=x\ny\0");
        let vars = parse_env_dump(dump.as_bytes());
        assert_eq!(vars.get("PATH").unwrap(), "/a:/b");
        assert_eq!(vars.get("MULTI").unwrap(), "x\ny");
        assert_eq!(vars.len(), 3);
        assert!(parse_env_dump(b"no marker").is_empty());
    }

    #[test]
    fn spawn_env_keeps_user_vars_and_never_adds_keys() {
        let env = UserEnv::from_vars([
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("TERM_PROGRAM", "iTerm"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_MESSAGING_TOKEN", "secret"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
        ]);
        let out = env.for_spawn(&[("TERMLAB_SESSION_ID".into(), "s1".into())]);
        assert_eq!(out["PATH"], "/bin");
        assert_eq!(out["TERM"], "xterm-256color");
        assert_eq!(out["TERMLAB_SESSION_ID"], "s1");
        assert!(!out.contains_key("TERM_PROGRAM"));
        // The launching agent session's markers are dropped; user configuration is kept.
        assert!(!out.contains_key("CLAUDE_CODE_CHILD_SESSION"));
        assert!(!out.contains_key("CLAUDE_CODE_MESSAGING_TOKEN"));
        assert_eq!(out["CLAUDE_CODE_USE_BEDROCK"], "1");
        for k in API_KEY_VARS {
            assert!(!out.contains_key(*k));
        }
    }

    #[test]
    fn detects_inherited_api_keys() {
        let env = UserEnv::from_vars([("ANTHROPIC_API_KEY", "sk-x"), ("OPENAI_API_KEY", " ")]);
        assert_eq!(env.api_key_vars(), vec!["ANTHROPIC_API_KEY"]);
        assert_eq!(billing_for(&env, &["ANTHROPIC_API_KEY"]), Billing::ApiKey);
        assert_eq!(billing_for(&env, &["OPENAI_API_KEY"]), Billing::Unknown);
    }
}
