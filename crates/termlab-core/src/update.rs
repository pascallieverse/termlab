//! Notices when an agent was updated while a session still runs the older program, and
//! updates agents that don't update themselves.
//!
//! Agents update themselves (Claude Code swaps the link `~/.local/bin/claude` to a new file
//! under `versions/`) or are upgraded by the user (Homebrew, npm). A session that was
//! already running keeps the program it started with until it is restarted. Nothing here
//! looks online: it compares what is installed with what the session runs, and with the
//! newest version the agent itself last saw (Codex keeps that in `~/.codex/version.json`).
//!
//! An agent installed with npm or Homebrew is updated by running that package manager on
//! the copy the session runs. A machine can have several copies (two npm prefixes, say),
//! and a plain `npm install -g` updates whichever one the current `npm` belongs to, which
//! need not be the one Termlab starts.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use crate::env::UserEnv;

/// What a session's agent program was when the session started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramWatch {
    /// The program as launched (often a link to the real file).
    program: PathBuf,
    /// The real file it pointed to at the start, and when that file was last changed.
    target: Option<PathBuf>,
    modified: Option<SystemTime>,
    /// For a session Termlab only attaches to: the process that really runs the agent.
    worker_pid: Option<u32>,
}

impl ProgramWatch {
    pub fn start(program: &Path, worker_pid: Option<u32>) -> Self {
        let target = std::fs::canonicalize(program).ok();
        Self {
            program: program.to_path_buf(),
            modified: target.as_deref().and_then(modified),
            target,
            worker_pid,
        }
    }

    /// The program as launched.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Whether the installed program is no longer the one this session runs.
    pub fn outdated(&self) -> bool {
        let Ok(installed) = std::fs::canonicalize(&self.program) else {
            return false;
        };
        if let Some(pid) = self.worker_pid {
            // The worker started before Termlab attached: ask what it runs.
            return process_executable(pid).is_some_and(|exe| {
                // An older version's file may be gone by now; its path still tells it apart.
                std::fs::canonicalize(&exe).unwrap_or(exe) != installed
            });
        }
        match &self.target {
            Some(target) if *target != installed => true,
            // The same file, replaced in place.
            Some(_) => match (self.modified, modified(&installed)) {
                (Some(then), Some(now)) => then != now,
                _ => false,
            },
            None => false,
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// The executable file a running process was started from.
pub fn process_executable(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }
    #[cfg(windows)]
    {
        // Not looked up on Windows: sessions Termlab attaches to are compared by nothing,
        // so they never ask for a restart there.
        let _ = pid;
        None
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // `lsof -a -p <pid> -d txt -Fn`: the process's program text files, one `n<path>`
        // line each; the first is the executable itself (the rest are libraries).
        let out = std::process::Command::new("/usr/sbin/lsof")
            .args(["-a", "-p", &pid.to_string(), "-d", "txt", "-Fn"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        lsof_executable(&String::from_utf8_lossy(&out.stdout))
    }
}

#[cfg_attr(any(target_os = "linux", windows), allow(dead_code))]
fn lsof_executable(output: &str) -> Option<PathBuf> {
    output
        .lines()
        .find_map(|l| l.strip_prefix('n'))
        .filter(|p| p.starts_with('/'))
        .map(PathBuf::from)
}

/// How long a package manager may take to update an agent: under the app's 120 s call limit.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(110);

/// A copy of an agent installed by a package manager that Termlab can run to update it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// A global npm package: `<prefix>/lib/node_modules/<package>` (Windows:
    /// `<prefix>\node_modules\<package>`).
    Npm {
        package: String,
        prefix: PathBuf,
        version: Option<String>,
    },
    /// A Homebrew formula (`Cellar/<name>/<version>`) or cask (`Caskroom/<name>/<version>`).
    Homebrew {
        name: String,
        cask: bool,
        version: String,
    },
}

impl Install {
    /// How `program` (as found on PATH, often a link) was installed, when Termlab can tell.
    pub fn find(program: &Path) -> Option<Self> {
        let real = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
        if let Some(found) = homebrew(&real).or_else(|| npm_package(&real)) {
            return Some(found);
        }
        // npm's shims on Windows (`codex.cmd`) are small scripts naming the package's file.
        let script = std::fs::read(program)
            .ok()
            .filter(|b| b.len() < 16 * 1024)?;
        let text = String::from_utf8_lossy(&script);
        // The path is relative to the shim's folder: `"%dp0%\node_modules\…\codex.js"`
        // (cmd) or `"$basedir/node_modules/…/codex.js"` (sh, PowerShell).
        let at = text.find("node_modules")?;
        let quote = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`');
        let start = text[..at].rfind(quote).map_or(0, |i| i + 1);
        let end = text[at..].find(quote).map_or(text.len(), |i| at + i);
        let mut path = program.parent()?.to_path_buf();
        path.extend(
            text[start..end]
                .split(['/', '\\'])
                .filter(|p| !p.is_empty() && !p.starts_with(['%', '$'])),
        );
        npm_package(&path)
    }

    /// The installed version.
    pub fn version(&self) -> Option<&str> {
        match self {
            Install::Npm { version, .. } => version.as_deref(),
            Install::Homebrew { version, .. } => Some(version),
        }
    }

    /// The command that updates this copy: npm at its own prefix, or Homebrew.
    fn command(&self, env: &UserEnv) -> Result<(PathBuf, Vec<String>), String> {
        match self {
            Install::Npm {
                package, prefix, ..
            } => {
                let npm = env
                    .which("npm")
                    .ok_or("npm isn't on your PATH, so Termlab can't update this agent")?;
                Ok((
                    npm,
                    vec![
                        "install".into(),
                        "--global".into(),
                        "--prefix".into(),
                        prefix.to_string_lossy().into_owned(),
                        format!("{package}@latest"),
                    ],
                ))
            }
            Install::Homebrew { name, cask, .. } => {
                let brew = env
                    .which("brew")
                    .ok_or("Homebrew isn't on your PATH, so Termlab can't update this agent")?;
                let mut args = vec!["upgrade".to_string()];
                if *cask {
                    args.push("--cask".into());
                }
                args.push(name.clone());
                Ok((brew, args))
            }
        }
    }

    /// A one-line description of the update command, for logs and errors.
    pub fn describe(&self) -> String {
        match self {
            Install::Npm {
                package, prefix, ..
            } => format!(
                "npm install --global --prefix \"{}\" {package}@latest",
                prefix.display()
            ),
            Install::Homebrew { name, cask, .. } => {
                format!("brew upgrade{} {name}", if *cask { " --cask" } else { "" })
            }
        }
    }

    /// Runs the package manager and waits for it. The error carries the end of its output.
    pub fn update(&self, env: &UserEnv) -> Result<(), String> {
        let (program, args) = self.command(env)?;
        let what = self.describe();
        tracing::info!(command = %what, "updating agent");
        let mut child = Command::new(&program)
            .args(&args)
            .env_clear()
            .envs(env.vars())
            // Homebrew would otherwise refresh all its formulae first, which can take minutes.
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't run `{what}`: {e}"))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let out = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stdout.read_to_string(&mut s);
            s
        });
        let err = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() > UPDATE_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("`{what}` took too long and was stopped"));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(format!("`{what}` failed: {e}")),
            }
        };
        let mut output = out.join().unwrap_or_default();
        output.push_str(&err.join().unwrap_or_default());
        if status.success() {
            return Ok(());
        }
        tracing::warn!(command = %what, %output, "agent update failed");
        let tail: Vec<&str> = output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let tail = tail[tail.len().saturating_sub(3)..].join("\n");
        Err(if tail.is_empty() {
            format!("`{what}` failed ({status})")
        } else {
            format!("`{what}` failed:\n{tail}")
        })
    }
}

/// `…/Cellar/<name>/<version>/…` or `…/Caskroom/<name>/<version>/…`.
fn homebrew(real: &Path) -> Option<Install> {
    let parts: Vec<String> = real
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let at = parts
        .iter()
        .position(|p| p == "Cellar" || p == "Caskroom")?;
    let name = parts.get(at + 1)?.clone();
    let version = parts.get(at + 2)?.clone();
    Some(Install::Homebrew {
        cask: parts[at] == "Caskroom",
        name,
        version,
    })
}

/// The global npm package a file belongs to: the folder right under the last `node_modules`
/// (two folders for a scoped `@scope/name`), with its `package.json`.
fn npm_package(file: &Path) -> Option<Install> {
    let parts: Vec<_> = file.components().collect();
    let at = parts
        .iter()
        .rposition(|c| c.as_os_str() == "node_modules")?;
    let first = parts.get(at + 1)?.as_os_str().to_string_lossy();
    let depth = if first.starts_with('@') { 2 } else { 1 };
    if parts.len() <= at + depth {
        return None;
    }
    let modules: PathBuf = parts[..=at].iter().collect();
    let dir: PathBuf = parts[..=at + depth].iter().collect();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).ok()?).ok()?;
    let package = manifest.get("name")?.as_str()?.to_string();
    let version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .map(String::from);
    // Unix keeps global packages in `<prefix>/lib/node_modules`, Windows in
    // `<prefix>\node_modules`. Anything else isn't a global install (a project's own copy).
    let parent = modules.parent()?;
    let prefix = if parent.file_name().is_some_and(|n| n == "lib") {
        parent.parent()?.to_path_buf()
    } else if cfg!(windows) {
        parent.to_path_buf()
    } else {
        return None;
    };
    Some(Install::Npm {
        package,
        prefix,
        version,
    })
}

/// Whether version `a` is newer than `b` ("0.172.0" > "0.169.0"). Pre-release and build
/// suffixes are ignored; a version that doesn't start with a number is never newer.
pub fn is_newer(a: &str, b: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        let core = v.trim().trim_start_matches('v');
        let core = core.split(['-', '+', ' ']).next()?;
        core.split('.').map(|p| p.parse().ok()).collect()
    }
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn link(target: &Path, link: &Path) {
        let _ = std::fs::remove_file(link);
        std::os::unix::fs::symlink(target, link).expect("symlink");
    }

    #[cfg(unix)]
    #[test]
    fn a_swapped_link_means_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("1.0.0"), dir.path().join("1.0.1"));
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let program = dir.path().join("agent");
        link(&old, &program);

        let watch = ProgramWatch::start(&program, None);
        assert!(!watch.outdated(), "nothing changed yet");
        link(&new, &program);
        assert!(watch.outdated(), "the link now points to a newer file");
        // A session started now runs the new one.
        assert!(!ProgramWatch::start(&program, None).outdated());
    }

    #[test]
    fn a_file_replaced_in_place_means_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agent");
        std::fs::write(&program, "old").unwrap();
        let watch = ProgramWatch::start(&program, None);
        assert!(!watch.outdated());

        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&program)
            .unwrap();
        file.set_modified(later).unwrap();
        assert!(watch.outdated());
    }

    #[test]
    fn a_missing_program_is_not_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agent");
        std::fs::write(&program, "x").unwrap();
        let watch = ProgramWatch::start(&program, None);
        std::fs::remove_file(&program).unwrap();
        assert!(
            !watch.outdated(),
            "mid-upgrade, or uninstalled: nothing to restart into"
        );
        assert!(!ProgramWatch::start(&dir.path().join("nope"), None).outdated());
    }

    #[cfg(unix)]
    #[test]
    fn an_attached_worker_is_compared_by_what_it_runs() {
        // This test process stands in for the worker: it runs the test binary.
        let me = std::process::id();
        let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(
            process_executable(me).map(|p| std::fs::canonicalize(&p).unwrap_or(p)),
            Some(exe.clone())
        );
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agent");
        link(&exe, &program);
        assert!(!ProgramWatch::start(&program, Some(me)).outdated());

        let newer = dir.path().join("newer");
        std::fs::write(&newer, "new").unwrap();
        link(&newer, &program);
        assert!(ProgramWatch::start(&program, Some(me)).outdated());
    }

    #[test]
    fn lsof_output() {
        assert_eq!(
            lsof_executable(
                "p2261\nftxt\nn/Users/x/.local/share/claude/versions/2.1.323\nftxt\nn/usr/lib/dyld\n"
            ),
            Some(PathBuf::from(
                "/Users/x/.local/share/claude/versions/2.1.323"
            ))
        );
        assert_eq!(lsof_executable(""), None);
    }

    #[test]
    fn version_order() {
        assert!(is_newer("0.172.0", "0.169.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.99.1"));
        assert!(!is_newer("0.169.0", "0.169.0"));
        assert!(!is_newer("0.168.0", "0.169.0"));
        assert!(!is_newer("0.170.0-alpha.1", "0.170.0"));
        assert!(!is_newer("latest", "0.1.0"));
    }

    fn npm_install(prefix: &Path, version: &str) -> PathBuf {
        let modules = if cfg!(windows) {
            prefix.join("node_modules")
        } else {
            prefix.join("lib").join("node_modules")
        };
        let dir = modules.join("@openai").join("codex");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(
            dir.join("package.json"),
            format!(r#"{{"name":"@openai/codex","version":"{version}"}}"#),
        )
        .unwrap();
        let script = dir.join("bin").join("codex.js");
        std::fs::write(&script, "#!/usr/bin/env node\n").unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn an_npm_link_is_traced_to_its_own_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = std::fs::canonicalize(dir.path()).unwrap();
        npm_install(&prefix, "0.169.0");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        let program = prefix.join("bin").join("codex");
        link(
            Path::new("../lib/node_modules/@openai/codex/bin/codex.js"),
            &program,
        );
        let install = Install::find(&program).expect("an npm install");
        assert_eq!(
            install,
            Install::Npm {
                package: "@openai/codex".into(),
                prefix: prefix.clone(),
                version: Some("0.169.0".into()),
            }
        );
        assert!(install.describe().contains(&*prefix.to_string_lossy()));
    }

    #[test]
    fn an_npm_shim_script_is_traced_to_its_package() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path();
        npm_install(prefix, "0.170.0");
        let shim = prefix.join("codex.cmd");
        let modules = if cfg!(windows) {
            "node_modules"
        } else {
            "lib\\node_modules"
        };
        std::fs::write(
            &shim,
            format!(
                "@ECHO off\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\{modules}\\@openai\\codex\\bin\\codex.js\" %*\r\n"
            ),
        )
        .unwrap();
        let found = Install::find(&shim).expect("the shim's package");
        assert_eq!(found.version(), Some("0.170.0"));
    }

    #[test]
    fn homebrew_paths() {
        assert_eq!(
            homebrew(Path::new(
                "/opt/homebrew/Caskroom/codex/0.169.0/codex-aarch64-apple-darwin"
            )),
            Some(Install::Homebrew {
                name: "codex".into(),
                cask: true,
                version: "0.169.0".into(),
            })
        );
        assert_eq!(
            homebrew(Path::new("/usr/local/Cellar/gemini-cli/0.9.0/bin/gemini"))
                .map(|i| i.describe()),
            Some("brew upgrade gemini-cli".into())
        );
        assert_eq!(homebrew(Path::new("/usr/local/bin/codex")), None);
    }

    #[test]
    fn a_plain_binary_is_not_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("claude");
        std::fs::write(&program, [0u8, 1, 2, 3]).unwrap();
        assert_eq!(Install::find(&program), None);
    }
}
