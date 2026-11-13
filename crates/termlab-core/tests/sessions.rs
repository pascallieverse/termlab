//! Integration tests driven by `tests/fixtures/fake-agent.sh`. No real agent, no network.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use termlab_core::adapter::{AgentAdapter, CommandSpec, Installed, LaunchRequest};
use termlab_core::env::UserEnv;
use termlab_core::model::{
    AgentId, ApprovalChoice, Billing, PermissionPreset, SessionId, SessionStatus, TermSize,
};
use termlab_core::pty::{ExitInfo, PtyEvent, PtyProcess, SpawnSpec};
use termlab_core::service::StartMode;
use termlab_core::{CoreEvent, DataDirs, StartRequest, Termlab, TermlabConfig};
use tokio::sync::broadcast;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

fn fake_agent() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-agent.sh")
}

fn test_env() -> UserEnv {
    UserEnv::from_vars([
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
        ("HOME", "/tmp"),
        ("SECRET_TEST_VAR", "kept"),
    ])
}

fn spawn_fake(size: TermSize, cap: usize) -> PtyProcess {
    let spec = SpawnSpec {
        program: "/bin/sh".into(),
        args: vec![fake_agent().to_string_lossy().into_owned()],
        cwd: std::env::temp_dir(),
        env: test_env().for_spawn(&[]).into_iter().collect(),
        size,
    };
    PtyProcess::spawn(&spec, cap).expect("spawn fake agent")
}

/// Reads output into `buf` until it contains `needle`.
async fn wait_for(rx: &mut broadcast::Receiver<PtyEvent>, buf: &mut String, needle: &str) {
    let found = timeout(WAIT, async {
        loop {
            if buf.contains(needle) {
                return;
            }
            match rx.recv().await {
                Ok(PtyEvent::Output(b)) => buf.push_str(&String::from_utf8_lossy(&b)),
                Ok(PtyEvent::Exited(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
    .await;
    assert!(
        found.is_ok() && buf.contains(needle),
        "timed out waiting for {needle:?}; got:\n{buf}"
    );
}

async fn wait_exit(rx: &mut broadcast::Receiver<PtyEvent>) -> ExitInfo {
    timeout(WAIT, async {
        loop {
            match rx.recv().await {
                Ok(PtyEvent::Exited(info)) => return info,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("closed before exit"),
            }
        }
    })
    .await
    .expect("timed out waiting for exit")
}

// ---- pty level -----------------------------------------------------------------

#[tokio::test]
async fn spawn_echo_and_colours_survive() {
    let pty = spawn_fake(TermSize::default(), 1 << 20);
    let att = pty.attach();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "FAKE AGENT READY").await;
    // ANSI is passed through untouched.
    assert!(buf.contains("\x1b[32mFAKE AGENT READY\x1b[0m"));

    pty.write(b"hello termlab\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "echo: hello termlab").await;
    pty.write(b"exit 0\n".to_vec()).unwrap();
    assert_eq!(
        wait_exit(&mut rx).await,
        ExitInfo {
            code: 0,
            success: true
        }
    );
}

#[tokio::test]
async fn initial_size_and_resize_reach_the_child() {
    let pty = spawn_fake(TermSize { cols: 90, rows: 25 }, 1 << 20);
    let att = pty.attach();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "READY").await;

    pty.write(b"size\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "SIZE 25 90").await;

    pty.resize(TermSize {
        cols: 132,
        rows: 41,
    })
    .unwrap();
    pty.write(b"size\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "SIZE 41 132").await;

    assert!(pty.resize(TermSize { cols: 0, rows: 10 }).is_err());
    pty.write(b"exit 0\n".to_vec()).unwrap();
    wait_exit(&mut rx).await;
}

#[tokio::test]
async fn exit_codes_are_reported() {
    for code in [0, 1, 3, 42] {
        let pty = spawn_fake(TermSize::default(), 1 << 20);
        let att = pty.attach();
        let mut rx = att.events;
        let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
        wait_for(&mut rx, &mut buf, "READY").await;
        pty.write(format!("exit {code}\n").into_bytes()).unwrap();
        let info = wait_exit(&mut rx).await;
        assert_eq!(info.code, code);
        assert_eq!(info.success, code == 0);
        assert_eq!(pty.wait().await, info);
        assert_eq!(pty.exit_info(), Some(info));
        // The last output arrives before the exit is announced.
        assert!(
            String::from_utf8_lossy(&pty.scrollback_snapshot()).contains(&format!("bye {code}"))
        );
    }
}

#[tokio::test]
async fn kill_stops_the_child() {
    let pty = spawn_fake(TermSize::default(), 1 << 20);
    let mut rx = pty.subscribe();
    pty.kill().unwrap();
    let info = wait_exit(&mut rx).await;
    assert!(!info.success);
}

#[tokio::test]
async fn scrollback_captures_output_and_respects_its_cap() {
    let cap = 8 * 1024;
    let pty = spawn_fake(TermSize::default(), cap);
    let att = pty.attach();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "READY").await;

    pty.write(b"flood 3000\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "FLOOD DONE").await;

    let snap = pty.scrollback_snapshot();
    let text = String::from_utf8_lossy(&snap);
    assert!(snap.len() <= cap, "scrollback {} > cap {cap}", snap.len());
    assert!(text.contains("flood line 003000"));
    assert!(text.contains("FLOOD DONE"));
    assert!(
        !text.contains("flood line 000001"),
        "oldest lines should be trimmed"
    );
    // Trimming happens on a line boundary.
    assert!(text.starts_with("flood line "));

    // A late viewer gets the snapshot plus everything after it, with no gap.
    let late = pty.attach();
    let mut late_rx = late.events;
    let mut late_buf = String::from_utf8_lossy(&late.snapshot).into_owned();
    pty.write(b"after attach\n".to_vec()).unwrap();
    wait_for(&mut late_rx, &mut late_buf, "echo: after attach").await;
    assert!(late_buf.contains("FLOOD DONE"));

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("s.log");
    pty.flush_scrollback(&file).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), pty.scrollback_snapshot());

    pty.write(b"exit 0\n".to_vec()).unwrap();
    wait_exit(&mut rx).await;
}

// ---- service level -------------------------------------------------------------

/// An adapter that launches the fake agent, registered under the `claude` id so Claude
/// hook payloads apply to it.
struct FakeAdapter;

impl AgentAdapter for FakeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }
    fn subagent_timeline(
        &self,
        _id: &str,
        subagent_id: &str,
        transcript: Option<&Path>,
        _env: &UserEnv,
    ) -> Vec<termlab_core::transcript::TimelineEntry> {
        // Like Claude's adapter, including refusing ids that aren't plain names.
        let safe = subagent_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
        match transcript {
            Some(t) if safe => termlab_core::transcript::claude_subagent_timeline(
                &t.with_extension("")
                    .join("subagents")
                    .join(format!("agent-{subagent_id}.jsonl")),
            ),
            _ => Vec::new(),
        }
    }

    // Reads sub-agents like Claude does.
    fn subagents(
        &self,
        _id: &str,
        transcript: Option<&Path>,
        _since: termlab_core::model::Millis,
        now: termlab_core::model::Millis,
        _env: &UserEnv,
    ) -> Vec<termlab_core::model::SubAgent> {
        transcript
            .map(|t| termlab_core::adapter::claude_subagents(t, now))
            .unwrap_or_default()
    }
    // Follows /rename and /color like Claude does, so that logic is tested on Claude's format.
    fn transcript_marks(
        &self,
        transcript: &Path,
        from: u64,
    ) -> (u64, termlab_core::adapter::TranscriptMarks) {
        termlab_core::adapter::claude_transcript_marks(transcript, from)
    }
    fn binary_name(&self) -> &'static str {
        "sh"
    }
    fn api_key_vars(&self) -> &'static [&'static str] {
        &["ANTHROPIC_API_KEY"]
    }
    fn detect(&self, _env: &UserEnv) -> Option<Installed> {
        Some(Installed {
            agent: AgentId::Claude,
            path: "/bin/sh".into(),
            version: Some("0.0.0-fake".into()),
            login_billing: Billing::Subscription,
        })
    }
    fn can_resume(&self) -> bool {
        true
    }
    fn can_fork(&self) -> bool {
        true
    }
    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        let mut args = vec![fake_agent().to_string_lossy().into_owned()];
        if let Some(m) = &req.first_message {
            args.push(m.clone());
        }
        CommandSpec {
            program: req.program.clone(),
            args,
            env: vec![("TERMLAB_SESSION_ID".into(), req.session_id.to_string())],
            cwd: req.cwd.clone(),
            agent_session_id: Some(format!("fake-{}", req.session_id)),
        }
    }
    fn resume(&self, id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut spec = self.launch(req);
        spec.args.push(format!("resume {id}"));
        spec.agent_session_id = Some(id.into());
        Some(spec)
    }
    fn fork(&self, id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        let mut spec = self.launch(req);
        spec.args.push(format!("fork {id}"));
        spec.agent_session_id = None;
        Some(spec)
    }
}

fn open_termlab(root: &Path, env: UserEnv) -> Arc<Termlab> {
    let mut config = TermlabConfig::new(DataDirs::new(root), env);
    config.adapters = vec![Box::new(FakeAdapter)];
    Termlab::open(config).expect("open termlab")
}

/// With hooks enabled: the status monitor runs and hook.sock accepts events.
fn open_termlab_with_hooks(root: &Path) -> Arc<Termlab> {
    let mut config = TermlabConfig::new(DataDirs::new(root), test_env());
    config.adapters = vec![Box::new(FakeAdapter)];
    config.hook_binary = Some("/nonexistent/termlab-hook".into());
    let tl = Termlab::open(config).expect("open termlab");
    tl.start_background().expect("background tasks");
    tl
}

fn start_req(folder_id: i64, msg: Option<&str>) -> StartRequest {
    StartRequest {
        folder_id,
        agent: AgentId::Claude,
        model: None,
        permission: PermissionPreset::AskFirst,
        first_message: msg.map(Into::into),
        size: Some(TermSize {
            cols: 100,
            rows: 30,
        }),
        notify: true,
        title: None,
        mode: StartMode::New,
    }
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    timeout(WAIT, async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

async fn wait_for_status(tl: &Termlab, id: SessionId, status: SessionStatus) {
    wait_until(&format!("{status:?}"), || {
        tl.session(id).unwrap().record.status == status
    })
    .await;
}

async fn wait_for_exit_status(tl: &Termlab, id: SessionId, status: SessionStatus) {
    wait_until(&format!("exit as {status:?}"), || {
        tl.session(id).unwrap().record.status == status && !tl.is_live(id)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_runs_a_session_end_to_end() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    let mut events = tl.subscribe();

    let folder = tl.add_folder(project.path()).unwrap();
    assert_eq!(tl.list_folders().unwrap(), vec![folder.clone()]);

    let view = tl
        .start_session(&start_req(folder.id, Some("fix the tests")), None)
        .unwrap();
    let rec = view.record.clone();
    assert!(view.live);
    assert_eq!(rec.status, SessionStatus::Running);
    assert_eq!(rec.title, "fix the tests");
    assert_eq!(rec.billing, Billing::Subscription);
    assert_eq!(rec.agent_session_id, Some(format!("fake-{}", rec.id)));
    // Not a git repository: no start commit.
    assert_eq!(rec.start_commit, None);

    let att = tl.attach(rec.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "ARGS: fix the tests").await;

    // The user's environment passes through; Termlab adds its session id; no keys added.
    tl.write(rec.id, b"env SECRET_TEST_VAR\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "ENV SECRET_TEST_VAR=kept").await;
    tl.write(rec.id, b"env TERMLAB_SESSION_ID\n".to_vec())
        .unwrap();
    wait_for(
        &mut rx,
        &mut buf,
        &format!("ENV TERMLAB_SESSION_ID={}", rec.id),
    )
    .await;
    tl.write(rec.id, b"env ANTHROPIC_API_KEY\n".to_vec())
        .unwrap();
    wait_for(&mut rx, &mut buf, "ENV ANTHROPIC_API_KEY unset").await;

    // The cwd is the project folder.
    tl.write(rec.id, b"pwd\n".to_vec()).unwrap();
    let cwd = folder.path.to_string_lossy().into_owned();
    wait_for(&mut rx, &mut buf, &format!("PWD {cwd}")).await;
    tl.write(rec.id, b"size\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "SIZE 30 100").await;
    tl.resize(rec.id, TermSize { cols: 80, rows: 24 }).unwrap();
    tl.write(rec.id, b"size\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "SIZE 24 80").await;

    // send_message types the text and presses Enter.
    tl.send_message(rec.id, "hello from the composer").unwrap();
    wait_for(&mut rx, &mut buf, "echo: hello from the composer").await;

    tl.write(rec.id, b"exit 7\n".to_vec()).unwrap();
    wait_for_exit_status(&tl, rec.id, SessionStatus::Error).await;
    let done = tl.session(rec.id).unwrap();
    assert_eq!(done.record.exit_code, Some(7));
    assert!(!done.live);
    assert!(matches!(
        tl.write(rec.id, b"x".to_vec()),
        Err(termlab_core::CoreError::SessionNotLive(_))
    ));

    // A finished session still shows its output, from the saved scrollback file.
    let after = tl.attach(rec.id).unwrap();
    let text = String::from_utf8_lossy(&after.snapshot);
    assert!(text.contains("FAKE AGENT READY") && text.contains("bye 7"));
    assert_eq!(after.exited.map(|e| e.code), Some(7));

    // The UI was told about the start and the exit.
    let mut statuses = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let CoreEvent::SessionUpdated { session } = e {
            statuses.push(session.record.status);
        }
    }
    assert_eq!(statuses.first(), Some(&SessionStatus::Running));
    assert_eq!(statuses.last(), Some(&SessionStatus::Error));
}

#[tokio::test(flavor = "multi_thread")]
async fn clean_exit_is_done_and_stop_is_done() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    let folder = tl.add_folder(project.path()).unwrap();

    let a = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    assert_eq!(a.title, "New Claude Code session");
    tl.write(a.id, b"exit 0\n".to_vec()).unwrap();
    wait_for_exit_status(&tl, a.id, SessionStatus::Done).await;
    assert_eq!(tl.session(a.id).unwrap().record.exit_code, Some(0));

    // A user-requested stop counts as Done, not Error.
    let b = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    tl.stop(b.id).unwrap();
    wait_for_exit_status(&tl, b.id, SessionStatus::Done).await;

    // Finished sessions are removed at once.
    tl.delete_session(b.id).unwrap();
    assert!(tl.session(b.id).is_err());
    // Removing a running session stops it, then removes it once it has exited.
    let c = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    tl.delete_session(c.id).unwrap();
    wait_until("running session removed", || tl.session(c.id).is_err()).await;
    assert!(!tl.is_live(c.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_resumes_in_place_keeping_earlier_output() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    let folder = tl.add_folder(project.path()).unwrap();
    let rec = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    tl.write(rec.id, b"first run\n".to_vec()).unwrap();
    tl.write(rec.id, b"exit 0\n".to_vec()).unwrap();
    wait_for_exit_status(&tl, rec.id, SessionStatus::Done).await;

    let again = tl.restart(rec.id, None).unwrap();
    assert!(again.live);
    assert_eq!(again.record.status, SessionStatus::Running);
    assert_eq!(again.record.exit_code, None);
    let att = tl.attach(rec.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    assert!(buf.contains("echo: first run"), "earlier output kept");
    wait_for(&mut rx, &mut buf, &format!("ARGS: resume fake-{}", rec.id)).await;
    tl.stop(rec.id).unwrap();
    wait_for_exit_status(&tl, rec.id, SessionStatus::Done).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_and_hand_off_start_new_sessions() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    let folder = tl.add_folder(project.path()).unwrap();
    let src = tl
        .start_session(
            &start_req(folder.id, Some("Make webhooks idempotent")),
            None,
        )
        .unwrap()
        .record;

    let fork = tl.fork(src.id, None).unwrap().record;
    assert_ne!(fork.id, src.id);
    assert_eq!(fork.title, "Fork: Make webhooks idempotent");
    let att = tl.attach(fork.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, &format!("fork fake-{}", src.id)).await;

    let msg = tl.handoff_message(src.id).unwrap();
    assert!(msg.contains("Claude Code was working on"), "{msg}");
    assert!(msg.contains("Goal: Make webhooks idempotent"), "{msg}");
    let handed = tl
        .hand_off(
            src.id,
            AgentId::Claude,
            PermissionPreset::AutoEdit,
            None,
            None,
        )
        .unwrap()
        .record;
    assert_eq!(handed.title, "Hand-off: Make webhooks idempotent");
    assert_eq!(handed.permission_mode, Some(PermissionPreset::AutoEdit));

    for id in [src.id, fork.id, handed.id] {
        tl.stop(id).unwrap();
        wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn broadcast_reaches_every_running_session_in_a_folder() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    let folder = tl.add_folder(project.path()).unwrap();
    let elsewhere = tl.add_folder(other.path()).unwrap();
    let a = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    let b = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    let c = tl
        .start_session(&start_req(elsewhere.id, None), None)
        .unwrap()
        .record;

    assert_eq!(tl.broadcast(folder.id, "run the linter").unwrap(), 2);
    for id in [a.id, b.id] {
        let att = tl.attach(id).unwrap();
        let mut rx = att.events;
        let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
        wait_for(&mut rx, &mut buf, "echo: run the linter").await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let c_out = String::from_utf8_lossy(&tl.attach(c.id).unwrap().snapshot).into_owned();
    assert!(!c_out.contains("run the linter"));

    for id in [a.id, b.id, c.id] {
        tl.stop(id).unwrap();
        wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn billing_reflects_inherited_api_keys() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let mut env = vec![("PATH", "/usr/bin:/bin"), ("HOME", "/tmp")];
    env.push(("ANTHROPIC_API_KEY", "sk-test"));
    let tl = open_termlab(data.path(), UserEnv::from_vars(env));
    let folder = tl.add_folder(project.path()).unwrap();
    let view = tl.start_session(&start_req(folder.id, None), None).unwrap();
    assert_eq!(view.record.billing, Billing::ApiKey);
    assert_eq!(view.api_key_vars, vec!["ANTHROPIC_API_KEY".to_string()]);
    let rec = view.record;

    // The key is passed through unchanged, not stripped or replaced.
    let att = tl.attach(rec.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    tl.write(rec.id, b"env ANTHROPIC_API_KEY\n".to_vec())
        .unwrap();
    wait_for(&mut rx, &mut buf, "ENV ANTHROPIC_API_KEY=sk-test").await;
    tl.stop(rec.id).unwrap();
    wait_for_exit_status(&tl, rec.id, SessionStatus::Done).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_from_a_previous_run_are_marked_ended_on_open() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let id = {
        let tl = open_termlab(data.path(), test_env());
        let folder = tl.add_folder(project.path()).unwrap();
        let rec = tl
            .start_session(&start_req(folder.id, None), None)
            .unwrap()
            .record;
        let att = tl.attach(rec.id).unwrap();
        let mut rx = att.events;
        let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
        wait_for(&mut rx, &mut buf, "READY").await;
        // Simulate the daemon quitting: scrollback saved, process stopped.
        tl.shutdown();
        rec.id
    };
    let tl = open_termlab(data.path(), test_env());
    let rec = tl.session(id).unwrap().record;
    // Ended because the service stopped: done, not an error.
    assert_eq!(rec.status, SessionStatus::Done);
    assert!(!tl.is_live(id));
    let after = tl.attach(id).unwrap();
    assert!(String::from_utf8_lossy(&after.snapshot).contains("FAKE AGENT READY"));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_folder_rejects_non_directories() {
    let data = tempfile::tempdir().unwrap();
    let tl = open_termlab(data.path(), test_env());
    assert!(tl.add_folder(Path::new("/definitely/not/here")).is_err());
    let file = data.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(tl.add_folder(&file).is_err());
}

// ---- status: heuristics, hooks, approvals ---------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn heuristics_go_running_then_idle_then_needs_you_and_answer_with_keys() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let rec = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;

    // Quiet after the banner: Idle once the 2 s output window has passed.
    wait_for_status(&tl, rec.id, SessionStatus::Idle).await;
    let view = tl.session(rec.id).unwrap();
    assert!(view.preview.iter().any(|l| l.contains("FAKE AGENT READY")));

    // A permission prompt on screen: Needs you, with the options parsed.
    tl.write(rec.id, b"prompt\n".to_vec()).unwrap();
    wait_for_status(&tl, rec.id, SessionStatus::NeedsYou).await;
    let approval = tl.session(rec.id).unwrap().approval.expect("approval");
    assert_eq!(approval.question, "Do you want to proceed?");
    assert_eq!(approval.options.len(), 3);
    assert!(approval.has(ApprovalChoice::AllowAlways));

    // "Always allow" moves down one option and presses Enter.
    let att = tl.attach(rec.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    tl.answer(rec.id, ApprovalChoice::AllowAlways).unwrap();
    wait_for(&mut rx, &mut buf, "ANSWER len=3").await;
    // The prompt is gone: the answer can't be sent twice.
    wait_until("prompt gone", || {
        tl.session(rec.id).unwrap().approval.is_none()
    })
    .await;
    assert!(tl.answer(rec.id, ApprovalChoice::AllowOnce).is_err());

    tl.stop(rec.id).unwrap();
    wait_for_exit_status(&tl, rec.id, SessionStatus::Done).await;
}

/// Sends a recorded hook payload the way `termlab-hook` does.
async fn send_hook(tl: &Termlab, session: SessionId, event: &str, payload: &str) {
    use tokio::io::AsyncWriteExt;
    let mut s = tokio::net::UnixStream::connect(tl.dirs().hook_socket())
        .await
        .expect("hook socket");
    let msg = format!(
        "{{\"v\":1,\"session\":\"{session}\",\"agent\":\"claude\",\"event\":\"{event}\"}}\n{payload}"
    );
    s.write_all(msg.as_bytes()).await.unwrap();
    s.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn recorded_claude_hook_payloads_drive_status() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let rec = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record;
    let id = rec.id;

    // Payloads shaped like Claude Code 2.1.322's hook input.
    send_hook(&tl, id, "SessionStart", r#"{"session_id":"claude-abc","transcript_path":"/tmp/claude-abc.jsonl","cwd":"/w","hook_event_name":"SessionStart","source":"startup"}"#).await;
    wait_for_status(&tl, id, SessionStatus::Idle).await;
    let r = tl.session(id).unwrap().record;
    assert_eq!(r.agent_session_id.as_deref(), Some("claude-abc"));
    assert_eq!(r.transcript_path.as_deref(), Some("/tmp/claude-abc.jsonl"));

    send_hook(&tl, id, "UserPromptSubmit", r#"{"session_id":"claude-abc","hook_event_name":"UserPromptSubmit","prompt":"run the migration"}"#).await;
    wait_for_status(&tl, id, SessionStatus::Running).await;

    send_hook(&tl, id, "PreToolUse", r#"{"session_id":"claude-abc","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"psql -f 0042.sql"},"tool_use_id":"t1"}"#).await;
    send_hook(&tl, id, "PermissionRequest", r#"{"session_id":"claude-abc","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"psql -f 0042.sql"}}"#).await;
    wait_for_status(&tl, id, SessionStatus::NeedsYou).await;
    let approval = tl.session(id).unwrap().approval.expect("approval");
    assert_eq!(approval.detail.as_deref(), Some("$ psql -f 0042.sql"));

    send_hook(&tl, id, "PostToolUse", r#"{"session_id":"claude-abc","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"psql -f 0042.sql"},"tool_response":{},"tool_use_id":"t1"}"#).await;
    wait_for_status(&tl, id, SessionStatus::Running).await;
    assert!(tl.session(id).unwrap().approval.is_none());

    send_hook(&tl, id, "Notification", r#"{"session_id":"claude-abc","hook_event_name":"Notification","message":"Claude needs your permission to use Edit","notification_type":"permission_prompt"}"#).await;
    wait_for_status(&tl, id, SessionStatus::NeedsYou).await;

    send_hook(&tl, id, "Stop", r#"{"session_id":"claude-abc","hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"Migration applied."}"#).await;
    wait_for_status(&tl, id, SessionStatus::Idle).await;

    // Events for unknown sessions or with garbage are ignored without harm.
    send_hook(&tl, SessionId::new(), "Stop", "{}").await;
    send_hook(&tl, id, "Stop", "not json").await;

    // The Timeline shows what happened.
    let tl_entries = tl.timeline(id).unwrap();
    let texts: Vec<_> = tl_entries.iter().map(|e| e.text.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Asked permission: $ psql -f 0042.sql")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("Finished: Migration applied.")),
        "{texts:?}"
    );

    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
}

// ---- loading sessions from the agents' own history --------------------------------

/// A fake agent with history on disk and one background session running "elsewhere".
struct HistoryAdapter {
    other_folder: PathBuf,
}

impl AgentAdapter for HistoryAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }
    fn binary_name(&self) -> &'static str {
        "sh"
    }
    fn api_key_vars(&self) -> &'static [&'static str] {
        &[]
    }
    fn detect(&self, env: &UserEnv) -> Option<Installed> {
        FakeAdapter.detect(env)
    }
    fn can_resume(&self) -> bool {
        true
    }
    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        FakeAdapter.launch(req)
    }
    fn resume(&self, id: &str, req: &LaunchRequest) -> Option<CommandSpec> {
        FakeAdapter.resume(id, req)
    }
    fn discover_all(&self, _env: &UserEnv) -> Vec<termlab_core::adapter::PastSession> {
        let past = |id: &str, title: &str, cwd: &Path, at| termlab_core::adapter::PastSession {
            agent: AgentId::Claude,
            agent_session_id: id.into(),
            title: title.into(),
            cwd: cwd.to_path_buf(),
            last_activity_at: at,
            path: "/nonexistent.jsonl".into(),
        };
        vec![
            past("past-1", "Add rate limiting", &self.other_folder, 3_000),
            past("bg-1", "Background refactor", &self.other_folder, 2_000),
            past(
                "gone-1",
                "Folder was deleted",
                Path::new("/definitely/gone"),
                1_000,
            ),
        ]
    }
    fn running(&self, _p: &Path, _env: &UserEnv) -> Vec<termlab_core::adapter::RunningSession> {
        vec![termlab_core::adapter::RunningSession {
            agent: AgentId::Claude,
            agent_session_id: "bg-1".into(),
            short_id: Some("bg1".into()),
            name: Some("Background refactor".into()),
            cwd: self.other_folder.clone(),
            kind: termlab_core::adapter::RunningKind::Background,
            status: Some(SessionStatus::Running),
            started_at: 1,
        }]
    }
    fn attach(
        &self,
        r: &termlab_core::adapter::RunningSession,
        req: &LaunchRequest,
    ) -> Option<CommandSpec> {
        let mut spec = FakeAdapter.launch(req);
        spec.args
            .push(format!("attach {}", r.short_id.clone().unwrap_or_default()));
        spec.agent_session_id = Some(r.agent_session_id.clone());
        Some(spec)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn loads_sessions_from_everywhere_imports_and_attaches() {
    let data = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let other_path = other.path().canonicalize().unwrap();
    let mut config = TermlabConfig::new(DataDirs::new(data.path()), test_env());
    config.adapters = vec![Box::new(HistoryAdapter {
        other_folder: other_path.clone(),
    })];
    let tl = Termlab::open(config).unwrap();

    // Nothing added yet: "everywhere" still finds them, flagged with what's running.
    let found = tl.discover(None, true).unwrap();
    assert_eq!(found.len(), 3);
    assert!(
        found
            .iter()
            .all(|d| d.folder_id.is_none() && d.termlab_session.is_none())
    );
    let bg = found
        .iter()
        .find(|d| d.past.agent_session_id == "bg-1")
        .unwrap();
    assert!(bg.running_elsewhere);
    let ext = tl.external_sessions();
    assert_eq!(ext.len(), 1);
    assert!(ext[0].attachable);

    // Import all: the folder is added; the deleted one is skipped; repeats are no-ops.
    let items: Vec<_> = found.iter().map(|d| d.past.clone()).collect();
    let summary = tl.import(&items).unwrap();
    assert_eq!(
        (
            summary.imported,
            summary.folders_added,
            summary.skipped,
            summary.already_present
        ),
        (2, 1, 1, 0)
    );
    assert_eq!(tl.import(&items).unwrap().already_present, 2);
    let folders = tl.list_folders().unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path, other_path);
    let sessions = tl.list_sessions().unwrap();
    let imported = sessions
        .iter()
        .find(|s| s.record.agent_session_id.as_deref() == Some("past-1"))
        .unwrap();
    assert_eq!(
        imported.record.origin,
        termlab_core::model::SessionOrigin::Imported
    );
    assert_eq!(imported.record.status, SessionStatus::Done);
    assert_eq!(imported.record.title, "Add rate limiting");
    assert!(!imported.live);

    // An imported session resumes in place with the agent's resume command.
    let resumed = tl.restart(imported.record.id, None).unwrap();
    assert!(resumed.live);
    let att = tl.attach(imported.record.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "ARGS: resume past-1").await;
    tl.stop(imported.record.id).unwrap();
    wait_for_exit_status(&tl, imported.record.id, SessionStatus::Done).await;

    // The one running in the background can't be resumed a second time…
    let bg_rec = sessions
        .iter()
        .find(|s| s.record.agent_session_id.as_deref() == Some("bg-1"))
        .unwrap();
    let err = tl.restart(bg_rec.record.id, None).unwrap_err().to_string();
    assert!(err.contains("attach to it instead"), "{err}");

    // …but can be attached to.
    let attached = tl.attach_external(AgentId::Claude, "bg-1", None).unwrap();
    assert_eq!(
        attached.record.origin,
        termlab_core::model::SessionOrigin::Attached
    );
    assert_eq!(attached.record.title, "Background refactor");
    let att = tl.attach(attached.record.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    wait_for(&mut rx, &mut buf, "ARGS: attach bg1").await;
    // Attaching again returns the same Termlab session.
    let again = tl.attach_external(AgentId::Claude, "bg-1", None).unwrap();
    assert_eq!(again.record.id, attached.record.id);
    tl.stop(attached.record.id).unwrap();
    wait_for_exit_status(&tl, attached.record.id, SessionStatus::Done).await;

    // Resuming an attached session re-attaches (the agent kept it running) rather than
    // starting a second copy of the conversation.
    tl.restart(attached.record.id, None).unwrap();
    let att = tl.attach(attached.record.id).unwrap();
    let mut rx = att.events;
    let mut buf = String::new();
    wait_for(&mut rx, &mut buf, "ARGS: attach bg1").await;
    tl.stop(attached.record.id).unwrap();
    wait_for_exit_status(&tl, attached.record.id, SessionStatus::Done).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_interrupted_by_a_service_stop_resume_automatically() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let (id, size) = {
        let tl = open_termlab(data.path(), test_env());
        let folder = tl.add_folder(project.path()).unwrap();
        let rec = tl
            .start_session(&start_req(folder.id, None), None)
            .unwrap()
            .record;
        tl.resize(rec.id, TermSize { cols: 91, rows: 27 }).unwrap();
        let att = tl.attach(rec.id).unwrap();
        let mut rx = att.events;
        let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
        wait_for(&mut rx, &mut buf, "READY").await;
        // The service stops (quit, update, crash recovery all end up here).
        tl.shutdown();
        (rec.id, TermSize { cols: 91, rows: 27 })
    };

    // Next start: the session comes back by itself, with the agent's resume command, at the
    // size it had, earlier output kept.
    let tl = open_termlab(data.path(), test_env());
    tl.start_background().unwrap();
    wait_until("auto-resume", || tl.is_live(id)).await;
    let view = tl.session(id).unwrap();
    assert_eq!(view.record.status, SessionStatus::Running);
    assert_eq!(view.record.term_size, Some(size));
    let att = tl.attach(id).unwrap();
    assert_eq!(att.size, Some(size));
    let mut rx = att.events;
    let mut buf = String::from_utf8_lossy(&att.snapshot).into_owned();
    assert!(buf.contains("FAKE AGENT READY"), "earlier output kept");
    wait_for(&mut rx, &mut buf, &format!("ARGS: resume fake-{id}")).await;
    tl.write(id, b"size\n".to_vec()).unwrap();
    wait_for(&mut rx, &mut buf, "SIZE 27 91").await;

    // Only once: a later start doesn't resume it again after it's stopped on purpose.
    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    drop(tl);
    let tl = open_termlab(data.path(), test_env());
    tl.start_background().unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!tl.is_live(id));
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_does_not_count_as_the_agent_working() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let id = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record
        .id;
    wait_for_status(&tl, id, SessionStatus::Idle).await;

    // Typing a message character by character: the echo changes the screen, but the
    // session stays Idle.
    for c in "hello there".chars() {
        tl.write(id, c.to_string().into_bytes()).unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(
            tl.session(id).unwrap().record.status,
            SessionStatus::Idle,
            "typing {c:?} made it look busy"
        );
    }
    tl.write(id, b"\n".to_vec()).unwrap();

    // Real work (output well after the keystroke) shows as Running, then Idle again.
    tl.write(id, b"work 12\n".to_vec()).unwrap();
    wait_for_status(&tl, id, SessionStatus::Running).await;
    wait_for_status(&tl, id, SessionStatus::Idle).await;

    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_inside_claude_renames_the_session() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let id = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record
        .id;
    let transcript = data.path().join("claude-ren.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
    )
    .unwrap();
    let start = format!(
        r#"{{"session_id":"claude-ren","transcript_path":"{}","cwd":"/w","hook_event_name":"SessionStart","source":"startup"}}"#,
        transcript.display()
    );
    send_hook(&tl, id, "SessionStart", &start).await;

    let append = |t: &str| {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        std::io::Write::write_all(
            &mut f,
            format!("{{\"type\":\"custom-title\",\"customTitle\":\"{t}\",\"sessionId\":\"claude-ren\"}}\n").as_bytes(),
        )
        .unwrap();
    };
    let wait_title = |want: &'static str| {
        let tl = tl.clone();
        async move {
            for _ in 0..150 {
                if tl.session(id).unwrap().record.title == want {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!(
                "title never became {want:?}: {:?}",
                tl.session(id).unwrap().record.title
            );
        }
    };

    // /rename in Claude renames the session.
    append("Billing retries");
    wait_title("Billing retries").await;

    // A rename in Termlab afterwards sticks: the old /rename isn't applied again.
    tl.set_title(id, "My own name").unwrap();
    tokio::time::sleep(Duration::from_millis(6_000)).await;
    assert_eq!(tl.session(id).unwrap().record.title, "My own name");

    // A new /rename wins again.
    append("Queue worker");
    wait_title("Queue worker").await;

    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_session_lists_its_subagents() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let id = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record
        .id;
    let transcript = data.path().join("claude-sub.jsonl");
    std::fs::write(&transcript, "{}\n").unwrap();
    let start = format!(
        r#"{{"session_id":"claude-sub","transcript_path":"{}","cwd":"/w","hook_event_name":"SessionStart","source":"startup"}}"#,
        transcript.display()
    );
    send_hook(&tl, id, "SessionStart", &start).await;
    assert!(tl.session(id).unwrap().subagents.is_empty());

    // Claude starts a sub-agent: its transcript and meta appear next to the session's.
    let sub = data.path().join("claude-sub").join("subagents");
    std::fs::create_dir_all(&sub).unwrap();
    let working = "{\"type\":\"assistant\",\"message\":{\"stop_reason\":\"tool_use\"}}\n";
    std::fs::write(sub.join("agent-atests-1.jsonl"), working).unwrap();
    std::fs::write(
        sub.join("agent-atests-1.meta.json"),
        r#"{"name":"test-runner","description":"Run the billing tests","model":"sonnet"}"#,
    )
    .unwrap();
    wait_until("the sub-agent to be listed", || {
        tl.session(id).unwrap().subagents.len() == 1
    })
    .await;
    let agents = tl.session(id).unwrap().subagents;
    assert_eq!(agents[0].name, "test-runner");
    assert_eq!(
        agents[0].description.as_deref(),
        Some("Run the billing tests")
    );
    assert!(agents[0].running);

    // It reports back: listed as finished.
    let done = concat!(
        r#"{"type":"user","isSidechain":true,"timestamp":"2026-10-13T13:47:01.000Z","message":{"role":"user","content":"Run the billing tests"}}"#,
        "\n",
        r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-10-13T13:47:05.000Z","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#,
        "\n",
        r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-10-13T13:48:00.000Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"41 passed"}]}}"#,
        "\n",
    );
    std::fs::write(sub.join("agent-atests-1.jsonl"), done).unwrap();
    wait_until("the sub-agent to finish", || {
        tl.session(id)
            .unwrap()
            .subagents
            .first()
            .is_some_and(|a| !a.running)
    })
    .await;

    // Opening the sub-agent shows what it did: its task, its tool calls, its answer.
    let steps: Vec<String> = tl
        .subagent_timeline(id, "atests-1")
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(steps, ["Run the billing tests", "$ npm test", "41 passed"]);
    assert!(tl.subagent_timeline(id, "../../etc").unwrap().is_empty());

    // An ended session lists none.
    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    assert!(tl.session(id).unwrap().subagents.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_colour_set_inside_claude_colours_the_session() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let id = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record
        .id;
    let transcript = data.path().join("claude-col.jsonl");
    std::fs::write(&transcript, "{}\n").unwrap();
    let start = format!(
        r#"{{"session_id":"claude-col","transcript_path":"{}","cwd":"/w","hook_event_name":"SessionStart","source":"startup"}}"#,
        transcript.display()
    );
    send_hook(&tl, id, "SessionStart", &start).await;
    assert_eq!(tl.session(id).unwrap().record.color, None);

    let set = |color: &str| {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        std::io::Write::write_all(
            &mut f,
            format!("{{\"type\":\"agent-color\",\"agentColor\":\"{color}\",\"sessionId\":\"claude-col\"}}\n").as_bytes(),
        )
        .unwrap();
    };
    let color = || tl.session(id).unwrap().record.color;

    // /color purple
    set("purple");
    wait_until("the colour to apply", || {
        color().as_deref() == Some("purple")
    })
    .await;
    // A colour Termlab doesn't know is ignored; the next valid one applies.
    set("chartreuse");
    set("cyan");
    wait_until("the colour to change", || {
        color().as_deref() == Some("cyan")
    })
    .await;

    // /color default clears it; then a colour again.
    set("default");
    wait_until("the colour to clear", || color().is_none()).await;
    set("cyan");
    wait_until("the colour to come back", || {
        color().as_deref() == Some("cyan")
    })
    .await;

    // The colour stays with the session after it ends.
    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    assert_eq!(color().as_deref(), Some("cyan"));
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_status_line_reports_plan_limits() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tl = open_termlab_with_hooks(data.path());
    let folder = tl.add_folder(project.path()).unwrap();
    let id = tl
        .start_session(&start_req(folder.id, None), None)
        .unwrap()
        .record
        .id;
    assert!(tl.plan_usage().is_empty());
    let id_before = tl.session(id).unwrap().record.agent_session_id;

    // Before a session's first response the status line has no limits: nothing is recorded.
    send_hook(
        &tl,
        id,
        "StatusLine",
        r#"{"context_window":{"used_percentage":2}}"#,
    )
    .await;
    // Then it does. One window resets in the future, one already has.
    let soon = (termlab_core::model::now_ms() / 1000) + 3600;
    let report = format!(
        r#"{{"session_id":"s","rate_limits":{{"five_hour":{{"used_percentage":23.5,"resets_at":{soon}}},"seven_day":{{"used_percentage":63,"resets_at":{soon}}},"seven_day_opus":{{"used_percentage":90,"resets_at":1000}}}}}}"#
    );
    send_hook(&tl, id, "StatusLine", &report).await;
    wait_until("the plan limits", || !tl.plan_usage().is_empty()).await;
    let usage = tl.plan_usage().remove(0);
    assert_eq!(usage.agent, AgentId::Claude);
    let got: Vec<_> = usage
        .windows
        .iter()
        .map(|w| (w.id.as_str(), w.used_percentage))
        .collect();
    // The window that already reset is left out.
    assert_eq!(got, [("five_hour", 23.5), ("seven_day", 63.0)]);
    assert_eq!(usage.windows[0].resets_at, Some(soon * 1000));

    // The status line isn't session activity: nothing in the Timeline, the session untouched.
    assert!(
        tl.timeline(id)
            .unwrap()
            .iter()
            .all(|e| !e.text.contains("StatusLine"))
    );
    assert_eq!(tl.session(id).unwrap().record.agent_session_id, id_before);

    tl.stop(id).unwrap();
    wait_for_exit_status(&tl, id, SessionStatus::Done).await;
    tl.shutdown();
    drop(tl);

    // The last report survives a restart of the service.
    let tl = open_termlab_with_hooks(data.path());
    let again = tl.plan_usage();
    assert_eq!(again.len(), 1, "plan usage after reopening");
    assert_eq!(again[0].windows.len(), 2);
    tl.shutdown();
}
