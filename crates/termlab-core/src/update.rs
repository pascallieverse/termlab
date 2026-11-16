//! Notices when an agent was updated while a session still runs the older program.
//!
//! Agents update themselves (Claude Code swaps the link `~/.local/bin/claude` to a new file
//! under `versions/`) or are upgraded by the user (Homebrew, npm). A session that was
//! already running keeps the program it started with until it is restarted. Nothing here
//! looks online: it only compares what is installed with what the session runs.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
}
