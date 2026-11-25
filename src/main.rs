mod claude;
mod cli;
mod codex;
mod common;
mod install;
mod relay;
mod schedule;
mod update;

use common::*;
use schedule::Provider;
use serde_json::{Value, json};
use std::{
    env,
    io::{self, BufRead, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(120);
enum Event {
    Refresh,
    Closed,
    Schedule(Provider, Value),
    Codex(Value),
}

fn publish(state: &Value) -> io::Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, state)?;
    out.write_all(b"\n")?;
    out.flush()
}
fn serve(paths: Arc<Paths>) -> io::Result<()> {
    let mut reader = claude::Reader::default();
    let mut state = json!({"version":env!("CARGO_PKG_VERSION"),"claude":reader.read(&paths),"codex":empty("app-server","loading"),"notice":"","schedule":{"status":"off"},"claudeSchedule":{"status":"off"}});
    publish(&state)?;
    let (tx, rx) = mpsc::channel();
    let input = tx.clone();
    thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if line.trim() == "refresh" && input.send(Event::Refresh).is_err() {
                return;
            }
        }
        let _ = input.send(Event::Closed);
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let schedulers = Provider::ALL.map(|provider| {
        let (stop, stopped) = mpsc::channel();
        let (paths, cancel, output) = (paths.clone(), cancel.clone(), tx.clone());
        let worker = thread::spawn(move || {
            schedule::watch(provider, paths, cancel, stopped, |value| {
                let _ = output.send(Event::Schedule(provider, value));
            })
        });
        (stop, worker)
    });
    let mut job = None;
    let mut next_query = Instant::now();
    let mut next_local = next_query + Duration::from_secs(2);
    let result = loop {
        if job.is_none() && Instant::now() >= next_query {
            next_query = Instant::now() + INTERVAL;
            let (path, stop, output) = (paths.clone(), cancel.clone(), tx.clone());
            job = Some(thread::spawn(move || {
                let _ = output.send(Event::Codex(codex::read(&path, &stop, TIMEOUT)));
            }));
        }
        let next = if job.is_some() {
            next_local
        } else {
            next_query.min(next_local)
        };
        let event = rx.recv_timeout(next.saturating_duration_since(Instant::now()));
        let manual = matches!(event, Ok(Event::Refresh));
        let mut changed = false;
        match event {
            Ok(Event::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            Ok(Event::Schedule(provider, value)) => {
                changed = state[provider.state_key()] != value;
                state[provider.state_key()] = value;
            }
            Ok(Event::Codex(mut value)) => {
                if let Some(handle) = job.take() {
                    let _ = handle.join();
                }
                if value["received"].is_null() {
                    value["received"] = state["codex"]["received"].clone();
                }
                state["codex"] = value;
                changed = true;
            }
            Ok(Event::Refresh) if job.is_none() => next_query = Instant::now(),
            _ => {}
        }
        if manual || Instant::now() >= next_local {
            let value = reader.read(&paths);
            if value != state["claude"] {
                state["claude"] = value;
                state["notice"] = "".into();
                changed = true;
            } else if manual {
                state["notice"] = "다음 Claude Code 응답 대기".into();
            }
            next_local = Instant::now() + Duration::from_secs(2);
        }
        if (changed || manual)
            && let Err(err) = publish(&state)
        {
            break Err(err);
        }
    };
    cancel.store(true, Ordering::Relaxed);
    for (stop, worker) in schedulers {
        drop(stop);
        let _ = worker.join();
    }
    if let Some(handle) = job {
        let _ = handle.join();
    }
    result
}
fn run() -> io::Result<()> {
    let paths = Paths::load();
    let action = env::args().nth(1).unwrap_or_default();
    match action.as_str() {
        "sender-install" => relay::install(&paths, &env::args_os().skip(2).collect::<Vec<_>>()),
        "sender-uninstall" => relay::uninstall(),
        "sender-flush" => relay::flush(),
        "receiver-key" => relay::receiver_key(),
        "receive" => relay::receive(&paths, &env::args().nth(2).unwrap_or_default()),
        "sender" | "capture" | "forward" => {
            let sending = action == "sender";
            let mode = if sending {
                env::args().nth(2).unwrap_or_default()
            } else {
                action
            };
            if !matches!(mode.as_str(), "capture" | "forward") {
                return Err(io::Error::other(
                    "sender capture 또는 sender forward가 필요해.",
                ));
            }
            let at = now();
            let mut raw = Vec::new();
            io::stdin().read_to_end(&mut raw)?;
            if sending {
                let _ = relay::enqueue(&paths, &raw, at);
            } else {
                let _ = claude::capture(&paths, &raw, at);
            }
            if mode == "forward" {
                io::stdout().write_all(&raw)?;
            }
            Ok(())
        }
        "once" => publish(
            &json!({"claude":claude::Reader::default().read(&paths),"codex":codex::read(&paths,&AtomicBool::new(false),TIMEOUT)}),
        ),
        "serve" => serve(Arc::new(paths)),
        "--version" => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "check-update" | "update" => {
            update::run(&action, env::args().nth(2).as_deref() == Some("--json"))
        }
        "apply-update" => install::apply_update(&env::args().nth(2).unwrap_or_default()),
        "install" | "start" | "uninstall" => install::run(&action),
        _ => Err(io::Error::other(
            "사용법: ai-usage-tray install | start | uninstall | check-update | update | once | serve | capture | forward | sender-install SSH_HOST KEY | sender-uninstall | receiver-key",
        )),
    }
}
fn main() {
    unsafe {
        libc::umask(0o077);
    }
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1)
    }
}
