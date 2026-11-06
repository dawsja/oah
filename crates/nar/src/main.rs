//! nar — a full-screen AI coding agent for the terminal, rendered with OpenTUI.

mod agent;
mod api;
mod auth;
mod commands;
mod config;
mod mcp;
mod memory;
mod session;
mod skills;
mod tools;
mod ui;

use std::collections::HashSet;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opentui::{Terminal, TerminalOptions};
use tokio_util::sync::CancellationToken;

use crate::session::SessionData;
use crate::ui::app::{App, AppMsg};

const USAGE: &str = "\
nar — an AI coding agent for your terminal

Usage:
  nar [options] [prompt]
  nar sessions            List saved sessions
  nar login               Sign in with ChatGPT (no TUI)
  nar logout              Sign out

Options:
  -r, --resume [id]       Resume a session (default: most recent in this directory)
  -c, --continue          Continue the most recent session in this directory
  -m, --model <slug>      Model to use
  -e, --effort <level>    minimal | low | medium | high | xhigh
  -p, --permission <lvl>  auto | ask | allow
      --plan              Start in plan mode
  -h, --help              Show help
  -V, --version           Show version
";

struct Args {
    resume: Option<Option<String>>,
    model: Option<String>,
    effort: Option<config::Effort>,
    permission: Option<config::Permission>,
    plan: bool,
    prompt: Option<String>,
    sub: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { resume: None, model: None, effort: None, permission: None, plan: false, prompt: None, sub: None };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let mut rest = Vec::new();
    while i < argv.len() {
        let s = argv[i].as_str();
        let next = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            argv.get(*i).cloned().ok_or_else(|| format!("{s} needs a value"))
        };
        match s {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("nar {}", config::VERSION);
                std::process::exit(0);
            }
            "-r" | "--resume" => {
                let id = argv.get(i + 1).filter(|v| !v.starts_with('-')).cloned();
                if id.is_some() {
                    i += 1;
                }
                a.resume = Some(id);
            }
            "-c" | "--continue" => a.resume = Some(None),
            "-m" | "--model" => a.model = Some(next(&mut i)?),
            "-e" | "--effort" => a.effort = Some(config::Effort::parse(&next(&mut i)?).ok_or("unknown effort")?),
            "-p" | "--permission" => a.permission = Some(config::Permission::parse(&next(&mut i)?).ok_or("unknown permission level")?),
            "--plan" => a.plan = true,
            "sessions" | "login" | "logout" if rest.is_empty() && a.sub.is_none() => a.sub = Some(s.to_string()),
            _ if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => rest.push(argv[i].clone()),
        }
        i += 1;
    }
    if !rest.is_empty() {
        a.prompt = Some(rest.join(" "));
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nar: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let _ = config::ensure_dir(&config::home_dir());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build().expect("tokio runtime");
    let http = api::http_client();
    let auth = auth::Auth::new(http.clone());

    match args.sub.as_deref() {
        Some("sessions") => {
            let list = session::list();
            if list.is_empty() {
                println!("No saved sessions.");
            }
            for s in list.iter().take(50) {
                println!(
                    "{}  {:>8}  {}  {}",
                    s.id,
                    session::relative_time(s.updated),
                    config::short_path(std::path::Path::new(&s.cwd)),
                    if s.title.is_empty() { "(untitled)" } else { &s.title }
                );
            }
            return;
        }
        Some("login") => {
            let r = rt.block_on(async {
                let p = auth.begin_chatgpt_login().await?;
                println!("Opening your browser to sign in with ChatGPT. If it doesn't open, visit:\n\n  {}\n", p.display_url);
                auth::open_browser(&p.url);
                p.task.await?
            });
            match r {
                Ok(s) => println!("Signed in{}. {}", s.email.map(|e| format!(" as {e}")).unwrap_or_default(), s.label),
                Err(e) => {
                    eprintln!("Sign-in failed: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Some("logout") => {
            match rt.block_on(auth.logout()) {
                Ok(n) => println!("{n}"),
                Err(e) => eprintln!("{e}"),
            }
            return;
        }
        _ => {}
    }

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("nar needs an interactive terminal.");
        std::process::exit(1);
    }

    let mut settings = config::Settings::load();
    if let Some(m) = args.model {
        settings.model = m;
    }
    if let Some(e) = args.effort {
        settings.effort = e;
    }
    if let Some(p) = args.permission {
        settings.permission = p;
    }
    if args.plan {
        settings.mode = config::Mode::Plan;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut session = match &args.resume {
        Some(Some(id)) => match SessionData::load(id) {
            Some(s) => s,
            None => {
                eprintln!("nar: no session matches '{id}'. Run `nar sessions` to list them.");
                std::process::exit(1);
            }
        },
        Some(None) => SessionData::latest(&cwd.display().to_string()).unwrap_or_default(),
        None => SessionData::default(),
    };
    if args.resume.is_some() {
        if !session.model.is_empty() && settings.model != session.model && std::env::args().all(|a| a != "-m" && a != "--model") {
            settings.model = session.model.clone();
        }
        settings.effort = session.effort;
        settings.mode = if args.plan { config::Mode::Plan } else { session.mode };
    }
    session.model = settings.model.clone();
    let session_cwd = std::path::PathBuf::from(&session.cwd);
    let cwd = if args.resume.is_some() && session_cwd.is_dir() {
        let _ = std::env::set_current_dir(&session_cwd);
        session_cwd
    } else {
        cwd
    };

    let mut term = match Terminal::new(TerminalOptions { title: Some("nar".into()), ..Default::default() }) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("nar: failed to start the terminal UI: {e}");
            std::process::exit(1);
        }
    };
    let waker = term.waker();
    let (tx, rx) = std::sync::mpsc::channel::<AppMsg>();

    let _guard = rt.enter();
    let skills = Arc::new(skills::Skills::discover(&cwd));
    let memory = Arc::new(memory::Memory::new(&cwd));
    let mcp = {
        let tx = tx.clone();
        let waker = waker.clone();
        mcp::McpManager::start(&cwd, http.clone(), move || {
            let _ = tx.send(AppMsg::McpChanged);
            waker.wake();
        })
    };
    let client = api::Client::new(http.clone(), auth.clone(), settings.api_base.clone());
    let agent = agent::Agent {
        client,
        mcp: mcp.clone(),
        ctx: tools::ToolCtx { cwd: cwd.clone(), cancel: CancellationToken::new(), skills: skills.clone(), memory },
        grants: Arc::new(Mutex::new(HashSet::new())),
    };
    let mut app = App::new(settings, session, cwd, agent, auth, mcp, skills, rt.handle().clone(), tx, waker);
    if args.resume.is_some() && !app.session.transcript.is_empty() {
        let id = app.session.id.clone();
        app.session.transcript.push(session::Entry::Info { text: format!("Resumed session {id}") });
        app.revs.push(u64::MAX / 2);
        app.cache.push(None);
    }
    if let Some(p) = args.prompt {
        app.editor.set(&p);
        app.handle_event(
            opentui::Event::Key(opentui::KeyEvent { code: opentui::key::ENTER, mods: Default::default(), text: String::new(), repeat: false }),
            &mut term,
        );
    }

    let frame_budget = Duration::from_millis(16);
    let mut last_frame = Instant::now() - frame_budget;
    loop {
        app.on_tick();
        if app.dirty || app.animating() {
            if last_frame.elapsed() >= frame_budget || !app.animating() {
                ui::view::render(&mut app, &mut term);
                if let Err(e) = term.present() {
                    app.quit = true;
                    eprintln!("render error: {e}");
                }
                app.dirty = false;
                last_frame = Instant::now();
            }
        }
        let timeout = if app.animating() { Duration::from_millis(50) } else { Duration::from_millis(500) };
        for ev in term.poll(timeout) {
            app.handle_event(ev, &mut term);
        }
        while let Ok(msg) = rx.try_recv() {
            app.handle_msg(msg, &mut term);
        }
        if app.quit {
            break;
        }
    }

    app.shutdown();
    let id = app.session.id.clone();
    let saved = !app.session.is_empty();
    drop(term);
    if saved {
        println!("\x1b[2mSession saved.\x1b[0m Resume with:\n\n  \x1b[1mnar --resume {id}\x1b[0m\n");
    }
    rt.shutdown_timeout(Duration::from_millis(300));
}
