//! The real `termlabd` and `termlab-hook` binaries. The daemon gets a bare PATH and a
//! shell that fails, so it can't find (or run) any real agent.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use termlab_core::ipc::{Client, DaemonInfo, PROTOCOL_VERSION, Request};

fn termlabd() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_termlabd"))
}

/// `termlab-hook` is built into the same target directory by `cargo test --workspace`.
fn termlab_hook() -> Option<PathBuf> {
    let p = termlabd().with_file_name("termlab-hook");
    p.exists().then_some(p)
}

fn wait_for(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn hook_forwarder_never_blocks_or_prints() {
    let Some(hook) = termlab_hook() else {
        eprintln!("termlab-hook not built; run `cargo test --workspace`");
        return;
    };
    for env in [
        vec![],
        vec![
            ("TERMLAB_SESSION_ID", "00000000-0000-0000-0000-000000000000"),
            ("TERMLAB_HOOK_SOCKET", "/nonexistent/hook.sock"),
        ],
    ] {
        let start = Instant::now();
        let child = Command::new(&hook)
            .args(["claude", "Stop"])
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Stdin left open on purpose: the forwarder must not wait forever for it.
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty() && out.stderr.is_empty());
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "took {:?}",
            start.elapsed()
        );
    }
}

#[test]
fn daemon_starts_answers_and_stops() {
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(termlabd())
        .arg("--data-dir")
        .arg(data.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("SHELL", "/usr/bin/false")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let socket = data.path().join("termlabd.sock");
    wait_for(&socket);
    if termlab_hook().is_some() {
        wait_for(&data.path().join("hook.sock"));
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = Client::connect(&socket).await.unwrap();
        let info: DaemonInfo = client
            .call_as(Request::Hello {
                protocol: PROTOCOL_VERSION,
            })
            .await
            .unwrap();
        assert_eq!(info.pid, child.id());
        assert_eq!(info.live_sessions, 0);

        // A second daemon notices the first and exits.
        let second = Command::new(termlabd())
            .arg("--data-dir")
            .arg(data.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));

        // With hooks: a forwarded event for an unknown session is accepted and ignored.
        if let Some(hook) = termlab_hook() {
            let status = Command::new(hook)
                .args(["codex", "notify", r#"{"type":"agent-turn-complete"}"#])
                .env("TERMLAB_SESSION_ID", "00000000-0000-0000-0000-000000000000")
                .env("TERMLAB_HOOK_SOCKET", data.path().join("hook.sock"))
                .status()
                .unwrap();
            assert!(status.success());
        }

        client
            .call(Request::Shutdown { force: false })
            .await
            .unwrap();
    });

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "daemon didn't stop"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!socket.exists(), "socket removed on exit");
    assert!(data.path().join("termlab.sqlite").exists());
}
