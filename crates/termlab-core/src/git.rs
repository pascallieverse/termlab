//! Read-only git queries for the inspector and the Diff tab. Uses the `git` CLI from the
//! user's environment; every function returns `None`/empty outside a repository.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::env::UserEnv;

/// Untracked files bigger than this aren't read for line counts or diffs.
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;
const MAX_UNTRACKED_FILES: usize = 200;
/// Cap on the diff text sent to the UI.
pub const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    /// `None` for binary files.
    pub added: Option<u64>,
    pub deleted: Option<u64>,
    pub untracked: bool,
}

fn git(env: &UserEnv, dir: &Path, args: &[&str]) -> Option<(bool, String)> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_clear()
        .envs(env.vars())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

fn git_ok(env: &UserEnv, dir: &Path, args: &[&str]) -> Option<String> {
    match git(env, dir, args)? {
        (true, out) => Some(out),
        _ => None,
    }
}

pub fn branch(env: &UserEnv, dir: &Path) -> Option<String> {
    let b = git_ok(env, dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let b = b.trim();
    if b == "HEAD" {
        // Detached: show the short commit.
        return git_ok(env, dir, &["rev-parse", "--short", "HEAD"]).map(|s| s.trim().to_string());
    }
    (!b.is_empty()).then(|| b.to_string())
}

pub fn head_commit(env: &UserEnv, dir: &Path) -> Option<String> {
    git_ok(env, dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

fn untracked(env: &UserEnv, dir: &Path) -> Vec<String> {
    git_ok(
        env,
        dir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .map(|s| {
        s.split('\0')
            .filter(|p| !p.is_empty())
            .take(MAX_UNTRACKED_FILES)
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}

/// Files changed since `base` (committed, staged, unstaged and untracked).
pub fn changed_files(env: &UserEnv, dir: &Path, base: &str) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> =
        git_ok(env, dir, &["diff", "--numstat", "--no-renames", base])
            .map(|out| {
                out.lines()
                    .filter_map(|l| {
                        let mut parts = l.splitn(3, '\t');
                        let a = parts.next()?;
                        let d = parts.next()?;
                        let path = parts.next()?;
                        Some(ChangedFile {
                            path: path.to_string(),
                            added: a.parse().ok(),
                            deleted: d.parse().ok(),
                            untracked: false,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    for path in untracked(env, dir) {
        let full = dir.join(&path);
        let lines = std::fs::metadata(&full)
            .ok()
            .filter(|m| m.len() <= MAX_UNTRACKED_BYTES)
            .and_then(|_| std::fs::read(&full).ok())
            .filter(|b| !b.contains(&0))
            .map(|b| bytecount_lines(&b));
        files.push(ChangedFile {
            path,
            added: lines,
            deleted: lines.map(|_| 0),
            untracked: true,
        });
    }
    files
}

fn bytecount_lines(b: &[u8]) -> u64 {
    let n = b.iter().filter(|&&c| c == b'\n').count() as u64;
    if b.last().is_some_and(|&c| c != b'\n') {
        n + 1
    } else {
        n
    }
}

/// Unified diff since `base`, including untracked files as new-file diffs.
pub fn diff(env: &UserEnv, dir: &Path, base: &str) -> String {
    let mut out = git_ok(
        env,
        dir,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
            "-U3",
            base,
        ],
    )
    .unwrap_or_default();
    for path in untracked(env, dir) {
        if out.len() > MAX_DIFF_BYTES {
            break;
        }
        let small =
            std::fs::metadata(dir.join(&path)).is_ok_and(|m| m.len() <= MAX_UNTRACKED_BYTES);
        if !small {
            continue;
        }
        // `--no-index` exits 1 when files differ, which is the normal case here.
        if let Some((_, d)) = git(
            env,
            dir,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-index",
                "--",
                "/dev/null",
                &path,
            ],
        ) {
            out.push_str(&d);
        }
    }
    if out.len() > MAX_DIFF_BYTES {
        let mut cut = MAX_DIFF_BYTES;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n… diff truncated\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn branch_changes_and_diff() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        let env = UserEnv::from_vars([
            ("PATH", "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"),
            ("HOME", dir.to_str().unwrap()),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]);
        assert_eq!(branch(&env, dir), None);
        run(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        run(dir, &["add", "."]);
        run(
            dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        );
        let base = head_commit(&env, dir).unwrap();
        assert_eq!(branch(&env, dir).as_deref(), Some("main"));

        std::fs::write(dir.join("a.txt"), "one\n2\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x\ny").unwrap();
        let files = changed_files(&env, dir, &base);
        assert_eq!(
            files,
            vec![
                ChangedFile {
                    path: "a.txt".into(),
                    added: Some(2),
                    deleted: Some(1),
                    untracked: false
                },
                ChangedFile {
                    path: "new.txt".into(),
                    added: Some(2),
                    deleted: Some(0),
                    untracked: true
                },
            ]
        );
        let d = diff(&env, dir, &base);
        assert!(d.contains("+three"));
        assert!(d.contains("+++ b/new.txt"));
    }
}
