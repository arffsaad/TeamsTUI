//! TeamsTUI — Microsoft Teams in your terminal.
mod app;
mod backend;
mod html;
mod input;
mod model;
mod ui;

use anyhow::Result;
use app::{App, Ev};
use backend::auth::{self, Auth};
use backend::{demo::Demo, graph::Graph, Backend};
use crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

const USAGE: &str = "TeamsTUI — Microsoft Teams in your terminal

USAGE:
    teamstui [OPTIONS]

OPTIONS:
    --demo               Run with built-in sample data (no account, no network)
    --client-id <ID>     Entra ID application (client) ID to sign in with
                         [env: TEAMSTUI_CLIENT_ID]
    --tenant <TENANT>    Tenant ID or domain (default: organizations)
                         [env: TEAMSTUI_TENANT]
    --scopes <SCOPES>    Space-separated Graph scopes to request
                         [env: TEAMSTUI_SCOPES]
    --image-protocol <P> Inline image rendering: auto, kitty, or halfblocks
                         [env: TEAMSTUI_IMAGE_PROTOCOL]
    --logout             Forget the saved sign-in and exit
    -h, --help           Show this help
    -V, --version        Show the version
";

struct Opts {
    demo: bool,
    client_id: String,
    tenant: String,
    scopes: String,
    image_protocol: String,
}

fn parse_args() -> Result<Option<Opts>> {
    let env = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
    let mut o = Opts {
        demo: false,
        client_id: env("TEAMSTUI_CLIENT_ID", auth::DEFAULT_CLIENT_ID),
        tenant: env("TEAMSTUI_TENANT", auth::DEFAULT_TENANT),
        scopes: env("TEAMSTUI_SCOPES", auth::DEFAULT_SCOPES),
        image_protocol: env("TEAMSTUI_IMAGE_PROTOCOL", "auto"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| anyhow::anyhow!("{name} needs a value"));
        match a.as_str() {
            "--demo" => o.demo = true,
            "--client-id" => o.client_id = value("--client-id")?,
            "--tenant" => o.tenant = value("--tenant")?,
            "--scopes" => o.scopes = value("--scopes")?,
            "--image-protocol" => o.image_protocol = value("--image-protocol")?,
            "--logout" => {
                auth::logout()?;
                println!("Signed out. Saved sign-in removed from {}", auth::cache_path().display());
                return Ok(None);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("teamstui {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => anyhow::bail!("unknown option '{other}' (try --help)"),
        }
    }
    o.image_protocol = o.image_protocol.to_ascii_lowercase();
    if !o.scopes.split_whitespace().any(|s| s == "offline_access") {
        o.scopes = format!("offline_access {}", o.scopes);
    }
    Ok(Some(o))
}

/// Decide whether to use the Kitty graphics protocol for crisp inline images.
fn use_kitty(pref: &str) -> bool {
    match pref {
        "kitty" => return true,
        "halfblocks" | "half-blocks" | "none" | "off" => return false,
        _ => {}
    }
    // tmux/screen don't pass graphics through reliably — fall back to half-blocks.
    if std::env::var_os("TMUX").is_some() || std::env::var("TERM").map(|t| t.starts_with("screen")).unwrap_or(false) {
        return false;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default();
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
        || std::env::var_os("KONSOLE_VERSION").is_some()
        || term.contains("kitty")
        || term.contains("ghostty")
        || prog.eq_ignore_ascii_case("ghostty")
        || prog == "WezTerm"
}

type Term = Terminal<CrosstermBackend<std::io::Stdout>>;

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste, crossterm::cursor::Show);
}

/// Device-code sign-in screen. Returns false when the user quits.
async fn login(term: &mut Term, rx: &mut UnboundedReceiver<Ev>, tx: &UnboundedSender<Ev>, auth: &Arc<Auth>) -> Result<bool> {
    let start = |auth: Arc<Auth>, tx: UnboundedSender<Ev>| {
        tokio::spawn(async move {
            match auth.start_device_flow().await {
                Ok(dc) => {
                    let _ = tx.send(Ev::DeviceCode(Ok(dc.clone())));
                    let _ = tx.send(Ev::LoginDone(auth.poll_device_flow(&dc).await.map_err(|e| e.to_string())));
                }
                Err(e) => {
                    let _ = tx.send(Ev::DeviceCode(Err(e.to_string())));
                }
            }
        })
    };
    let mut task = start(auth.clone(), tx.clone());
    let mut st = ui::LoginState { code: None, error: None, tick: 0 };
    loop {
        term.draw(|f| ui::draw_login(f, &st))?;
        match rx.recv().await {
            None => return Ok(false),
            Some(Ev::Tick) => st.tick += 1,
            Some(Ev::DeviceCode(Ok(dc))) => st.code = Some(dc),
            Some(Ev::DeviceCode(Err(e))) | Some(Ev::LoginDone(Err(e))) => st.error = Some(e),
            Some(Ev::LoginDone(Ok(()))) => return Ok(true),
            Some(Ev::Input(Event::Key(k))) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(false),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
                KeyCode::Char('r') if st.error.is_some() => {
                    task.abort();
                    st = ui::LoginState { code: None, error: None, tick: 0 };
                    task = start(auth.clone(), tx.clone());
                }
                KeyCode::Enter | KeyCode::Char('o') => {
                    if let Some(c) = &st.code {
                        let _ = open::that_detached(&c.verification_uri);
                    }
                }
                _ => {}
            },
            Some(_) => {}
        }
    }
}

async fn run(term: &mut Term, opts: Opts) -> Result<Option<String>> {
    let (tx, mut rx) = unbounded_channel::<Ev>();
    // Terminal input on a dedicated thread; ticks drive polling and animations.
    let input_tx = tx.clone();
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(ev) => {
                if input_tx.send(Ev::Input(ev)).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    });
    let tick_tx = tx.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(250));
        loop {
            iv.tick().await;
            if tick_tx.send(Ev::Tick).is_err() {
                break;
            }
        }
    });

    let backend: Arc<dyn Backend> = if opts.demo {
        Arc::new(Demo::new())
    } else {
        let auth = Arc::new(Auth::new(opts.client_id, opts.tenant, opts.scopes));
        if !auth.signed_in().await && !login(term, &mut rx, &tx, &auth).await? {
            return Ok(None);
        }
        Arc::new(Graph::new(auth))
    };

    let mut app = App::new(backend, tx, opts.demo, use_kitty(&opts.image_protocol));
    app.start();
    let mut out = stdout();
    loop {
        term.draw(|f| ui::draw(f, &mut app))?;
        let _ = app.graphics.flush(&mut out);
        let Some(ev) = rx.recv().await else { break };
        app.on_event(ev);
        // Coalesce bursts (pastes, fast typing, several responses) into one redraw.
        while let Ok(ev) = rx.try_recv() {
            app.on_event(ev);
        }
        if app.quit {
            break;
        }
    }
    app.graphics.teardown(&mut out);
    Ok(app.exit_msg)
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(opts) = parse_args()? else { return Ok(()) };
    enable_raw_mode()?;
    crossterm::execute!(stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));
    let mut term = Terminal::new(CrosstermBackend::new(stdout()))?;
    let result = run(&mut term, opts).await;
    restore_terminal();
    match result {
        Ok(Some(msg)) => println!("{msg}"),
        Ok(None) => {}
        Err(e) => {
            eprintln!("teamstui: {e}");
            std::process::exit(1);
        }
    }
    // The input thread may be blocked in read(); don't wait for it.
    std::process::exit(0);
}
