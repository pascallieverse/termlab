//! Manual check against a REAL agent (not run by `cargo test`; it uses the agent's login
//! and a little of its quota). Starts the agent in a scratch git folder (under
//! ~/.cache/termlab-probe) with "Ask first", asks it to run a command that writes a file, and
//! prints every status change, the parsed permission prompt and the screen, then answers
//! "Allow once" through Termlab and waits for the turn to finish.
//!
//!   cargo build -p termlab-hook
//!   cargo run -p termlab-core --example live_probe -- claude haiku
//!   cargo run -p termlab-core --example live_probe -- codex

use std::time::{Duration, Instant};

use termlab_core::env::UserEnv;
use termlab_core::model::{AgentId, ApprovalChoice, PermissionPreset, SessionStatus, TermSize};
use termlab_core::service::StartMode;
use termlab_core::{DataDirs, StartRequest, Termlab, TermlabConfig};

#[tokio::main]
async fn main() -> anyhow_free::Result {
    let mut args = std::env::args().skip(1);
    let agent: AgentId = args.next().unwrap_or_else(|| "claude".into()).parse()?;
    let model = args.next();

    // Not under the system temp dir: Claude Code doesn't keep transcripts for folders there.
    let home = std::env::var("HOME")?;
    let data = std::path::Path::new(&home)
        .join(".cache/termlab-probe")
        .join(std::process::id().to_string());
    let project = data.join("project");
    std::fs::create_dir_all(&project)?;
    std::fs::write(project.join("README.md"), "probe\n")?;
    let git = |a: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(a)
            .output()
    };
    git(&["init", "-q"])?;
    git(&["add", "."])?;
    git(&[
        "-c",
        "user.name=probe",
        "-c",
        "user.email=p@p",
        "commit",
        "-qm",
        "init",
    ])?;

    let hook = std::env::current_exe()?
        .parent()
        .and_then(|d| d.parent())
        .map(|d| d.join("termlab-hook"))
        .filter(|p| p.exists());
    println!("data dir: {}\nhook binary: {hook:?}", data.display());

    let mut config = TermlabConfig::new(DataDirs::new(&data), UserEnv::capture_login_shell());
    config.hook_binary = hook;
    let tl = Termlab::open(config)?;
    tl.start_background()?;
    let folder = tl.add_folder(&project)?;
    let view = tl.start_session(
        &StartRequest {
            folder_id: folder.id,
            agent,
            model,
            permission: PermissionPreset::AskFirst,
            first_message: Some("Use the Bash tool to run exactly this command: touch probe.txt — then reply with the single word done.".into()),
            size: Some(TermSize { cols: 110, rows: 34 }),
            notify: false,
            title: None,
            mode: StartMode::New,
        },
        None,
    )?;
    let id = view.record.id;
    println!("started: {}", view.record.title);

    let start = Instant::now();
    let mut last = None;
    let mut answered = false;
    let mut trust_answered = false;
    while start.elapsed() < Duration::from_secs(90) {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let v = tl.session(id)?;
        if last != Some(v.record.status) {
            println!(
                "\n[{:>5.1}s] status → {:?}",
                start.elapsed().as_secs_f32(),
                v.record.status
            );
            last = Some(v.record.status);
        }
        if v.record.status == SessionStatus::NeedsYou && !answered {
            println!("approval: {:#?}", v.approval);
            println!(
                "--- screen tail ---\n{}\n-------------------",
                v.preview.join("\n")
            );
            let Some(a) = &v.approval else { continue };
            if a.options.is_empty() {
                continue;
            }
            // The folder-trust dialog comes first on a new folder; it isn't the tool prompt.
            let is_trust = a.question.to_lowercase().contains("trust");
            println!(
                "answering Allow once{}",
                if is_trust { " (folder trust)" } else { "" }
            );
            tl.answer(id, ApprovalChoice::AllowOnce)?;
            if is_trust {
                trust_answered = true;
            } else {
                answered = true;
            }
        }
        if answered && v.record.status == SessionStatus::Idle {
            println!("\nturn finished; final screen:\n{}", v.preview.join("\n"));
            break;
        }
        if !v.live {
            println!("session exited: {:?}", v.record.exit_code);
            break;
        }
        if start.elapsed() > Duration::from_secs(40)
            && !answered
            && v.record.status == SessionStatus::Idle
        {
            println!("--- idle with no prompt found; full screen ---");
            for (i, row) in tl.screen_text(id)?.iter().enumerate() {
                println!("{i:>2}|{row}");
            }
            break;
        }
    }
    let _ = trust_answered;
    if let Ok(rows) = tl.screen_text(id) {
        println!("--- final screen ---");
        for (i, row) in rows.iter().enumerate() {
            println!("{i:>2}|{row}");
        }
    }
    let r = tl.session(id)?.record;
    println!(
        "\nagent session id: {:?}\ntranscript: {:?}",
        r.agent_session_id, r.transcript_path
    );
    println!(
        "details: {:#?}",
        tl.details(id)
            .map(|d| (d.branch, d.usage, d.resume_command))
    );
    // Claude Code reports the plan's limits through the status line Termlab registers.
    println!("plan usage: {:#?}", tl.plan_usage());
    println!("timeline:");
    for e in tl.timeline(id)? {
        println!("  {:?} {}", e.kind, e.text);
    }
    let _ = tl.stop(id);
    tokio::time::sleep(Duration::from_secs(1)).await;
    tl.shutdown();
    Ok(())
}

/// Keeps the example free of extra dependencies.
mod anyhow_free {
    pub type Result = std::result::Result<(), Box<dyn std::error::Error>>;
}
