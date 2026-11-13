//! App ⇄ daemon protocol, end to end over a Unix socket, with the fake agent.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use termlab_core::adapter::{AgentAdapter, CommandSpec, Installed, LaunchRequest};
use termlab_core::env::UserEnv;
use termlab_core::ipc::{
    Client, ClientEvent, DaemonInfo, PROTOCOL_VERSION, RESET, Request, Server,
};
use termlab_core::model::{
    AgentId, Billing, Folder, PermissionPreset, SessionId, SessionStatus, SessionView, TermSize,
};
use termlab_core::service::StartMode;
use termlab_core::{CoreEvent, DataDirs, StartRequest, Termlab, TermlabConfig};
use tokio::sync::broadcast;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

struct FakeAdapter;

impl AgentAdapter for FakeAdapter {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }
    fn binary_name(&self) -> &'static str {
        "sh"
    }
    fn api_key_vars(&self) -> &'static [&'static str] {
        &[]
    }
    fn detect(&self, _env: &UserEnv) -> Option<Installed> {
        Some(Installed {
            agent: AgentId::Claude,
            path: "/bin/sh".into(),
            version: Some("0.0.0-fake".into()),
            login_billing: Billing::Subscription,
        })
    }
    fn launch(&self, req: &LaunchRequest) -> CommandSpec {
        CommandSpec {
            program: req.program.clone(),
            args: vec![
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/fake-agent.sh")
                    .to_string_lossy()
                    .into_owned(),
            ],
            env: vec![],
            cwd: req.cwd.clone(),
            agent_session_id: None,
        }
    }
}

async fn start_server(root: &Path) -> (Arc<Termlab>, PathBuf) {
    let mut config = TermlabConfig::new(
        DataDirs::new(root),
        UserEnv::from_vars([("PATH", "/usr/bin:/bin"), ("HOME", "/tmp")]),
    );
    config.adapters = vec![Box::new(FakeAdapter)];
    let tl = Termlab::open(config).unwrap();
    tl.start_background().unwrap();
    let socket = tl.dirs().daemon_socket();
    let server = Server::new(tl.clone());
    let path = socket.clone();
    tokio::spawn(async move { server.serve(&path).await });
    // Wait for the socket to accept.
    timeout(WAIT, async {
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (tl, socket)
}

async fn output_until(
    rx: &mut broadcast::Receiver<ClientEvent>,
    id: SessionId,
    buf: &mut Vec<u8>,
    needle: &str,
) {
    let ok = timeout(WAIT, async {
        loop {
            if String::from_utf8_lossy(buf).contains(needle) {
                return;
            }
            match rx.recv().await {
                Ok(ClientEvent::Output { id: sid, data }) if sid == id => {
                    buf.extend_from_slice(&data)
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    })
    .await;
    assert!(
        ok.is_ok(),
        "timed out waiting for {needle:?}; got {:?}",
        String::from_utf8_lossy(buf)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn app_and_daemon_round_trip_and_reconnect() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let (tl, socket) = start_server(data.path()).await;

    let client = Client::connect(&socket).await.unwrap();
    let mut events = client.subscribe();
    let hello: DaemonInfo = client
        .call_as(Request::Hello {
            protocol: PROTOCOL_VERSION,
        })
        .await
        .unwrap();
    assert_eq!(hello.protocol, PROTOCOL_VERSION);
    assert_eq!(hello.live_sessions, 0);

    let folder: Folder = client
        .call_as(Request::AddFolder {
            path: project.path().to_string_lossy().into_owned(),
        })
        .await
        .unwrap();
    let request = StartRequest {
        folder_id: folder.id,
        agent: AgentId::Claude,
        model: None,
        permission: PermissionPreset::AskFirst,
        first_message: None,
        size: Some(TermSize { cols: 90, rows: 20 }),
        notify: true,
        title: None,
        mode: StartMode::New,
    };
    let preview: String = client
        .call_as(Request::PreviewCommand {
            request: request.clone(),
            id: SessionId::new(),
        })
        .await
        .unwrap();
    assert!(preview.starts_with("cd "), "{preview}");
    let view: SessionView = client
        .call_as(Request::StartSession { request, id: None })
        .await
        .unwrap();
    let id = view.record.id;

    // Errors come back as messages, not disconnects.
    let err = client
        .call(Request::Stop {
            id: SessionId::new(),
        })
        .await
        .unwrap_err();
    assert!(err.contains("not running"), "{err}");

    // Attach: reset + snapshot first, then live output.
    client.call(Request::Attach { id }).await.unwrap();
    let mut out = Vec::new();
    output_until(&mut events, id, &mut out, "FAKE AGENT READY").await;
    // Reset, then the size the output was drawn at, then the output.
    assert!(out.starts_with(RESET));
    assert!(out[RESET.len()..].starts_with(b"\x1b[8;20;90t"));

    client
        .call(Request::Write {
            id,
            data: "via the socket\n".into(),
        })
        .await
        .unwrap();
    output_until(&mut events, id, &mut out, "echo: via the socket").await;
    client
        .call(Request::Resize {
            id,
            cols: 120,
            rows: 40,
        })
        .await
        .unwrap();
    client
        .call(Request::Write {
            id,
            data: "size\n".into(),
        })
        .await
        .unwrap();
    output_until(&mut events, id, &mut out, "SIZE 40 120").await;

    // The app goes away; the session keeps running in the daemon.
    drop(events);
    drop(client);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(tl.is_live(id));
    tl.write(id, b"while the app was closed\n".to_vec())
        .unwrap();

    // A new connection re-attaches and gets everything replayed, including what was
    // printed while no app was connected.
    let client = Client::connect(&socket).await.unwrap();
    let mut events = client.subscribe();
    let sessions: Vec<SessionView> = client.call_as(Request::ListSessions).await.unwrap();
    assert!(sessions.iter().any(|s| s.record.id == id && s.live));
    client.call(Request::Attach { id }).await.unwrap();
    let mut replay = Vec::new();
    output_until(
        &mut events,
        id,
        &mut replay,
        "echo: while the app was closed",
    )
    .await;
    // Replayed at the size it was last drawn at (the resize above).
    assert!(replay[RESET.len()..].starts_with(b"\x1b[8;40;120t"));
    let text = String::from_utf8_lossy(&replay);
    assert!(
        text.contains("echo: via the socket"),
        "earlier output replayed"
    );

    // Exit is reported as an event.
    client
        .call(Request::Write {
            id,
            data: "exit 4\n".into(),
        })
        .await
        .unwrap();
    let got = timeout(WAIT, async {
        loop {
            if let Ok(ClientEvent::Core(CoreEvent::SessionUpdated { session })) =
                events.recv().await
                && session.record.id == id
                && session.record.status == SessionStatus::Error
            {
                return session;
            }
        }
    })
    .await
    .expect("exit event");
    assert_eq!(got.record.exit_code, Some(4));
    assert!(!got.live);

    // Shutdown is refused while sessions run, allowed when none do.
    client
        .call(Request::Shutdown { force: false })
        .await
        .unwrap();
}
