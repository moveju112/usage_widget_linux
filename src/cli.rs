use crate::common::{Paths, empty};
use serde_json::Value;
use std::{
    env,
    os::unix::process::CommandExt,
    process::{ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub fn command(paths: &Paths, name: &str) -> Option<Command> {
    let cli = paths.find_cli(name)?;
    let mut dirs = vec![cli.parent().unwrap().to_owned()];
    if let Some(node) = paths.find_cli("node") {
        dirs.push(node.parent().unwrap().to_owned())
    }
    dirs.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut cmd = Command::new(cli);
    if let Ok(path) = env::join_paths(dirs) {
        cmd.env("PATH", path);
    }
    Some(cmd)
}

pub fn run(
    mut cmd: Command,
    cancel: &AtomicBool,
    timeout: Duration,
    source: &'static str,
    exchange: impl FnOnce(ChildStdin, ChildStdout) -> Value + Send + 'static,
) -> Value {
    if cancel.load(Ordering::Relaxed) {
        return empty(source, "timeout");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let Ok(mut child) = cmd.spawn() else {
        return empty(source, "error");
    };
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (send, recv) = mpsc::channel();
    let io_job = thread::spawn(move || {
        let _ = send.send(exchange(input, output));
    });
    let deadline = Instant::now() + timeout;
    let result = loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            break empty(source, "timeout");
        }
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        match recv.recv_timeout(wait) {
            Ok(value) => break value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            _ => break empty(source, "error"),
        }
    };
    // Successful exchanges close stdin. Give the CLI a moment to finish its own cleanup.
    if result["status"] != "timeout" {
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                let _ = io_job.join();
                return result;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    // The dedicated group includes CLI launcher children. Its leader is still unreaped.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let until = Instant::now() + Duration::from_millis(200);
    while !io_job.is_finished() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
    let _ = io_job.join();
    result
}
