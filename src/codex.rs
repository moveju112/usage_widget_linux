use crate::common::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::process::CommandExt,
    process::{ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

fn windows(result: &Value) -> Vec<Value> {
    let specific = &result["rateLimitsByLimitId"]["codex"];
    let limits = if specific.is_object() {
        specific
    } else {
        &result["rateLimits"]
    };
    let mut values = BTreeMap::new();
    for key in ["primary", "secondary"] {
        let item = &limits[key];
        if let Some(w) = window(
            &item["windowDurationMins"],
            &item["usedPercent"],
            &item["resetsAt"],
        ) {
            values.entry(w["minutes"].as_u64().unwrap()).or_insert(w);
        }
    }
    values.into_values().collect()
}

struct Rpc {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
enum Failure {
    Login,
    Remote,
    Protocol,
}
impl From<io::Error> for Failure {
    fn from(_: io::Error) -> Self {
        Self::Protocol
    }
}
impl From<serde_json::Error> for Failure {
    fn from(_: serde_json::Error) -> Self {
        Self::Protocol
    }
}
impl Rpc {
    fn send(&mut self, msg: Value) -> Result<(), Failure> {
        serde_json::to_writer(&mut self.input, &msg)?;
        self.input.write_all(b"\n")?;
        Ok(())
    }
    fn response(&mut self, id: i64) -> Result<Value, Failure> {
        let mut line = Vec::new();
        loop {
            line.clear();
            let size = self
                .output
                .by_ref()
                .take(4 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line)?;
            if size == 0 || size > 4 * 1024 * 1024 {
                return Err(Failure::Protocol);
            }
            let mut msg: Value = serde_json::from_slice(&line)?;
            if msg["id"].as_i64() != Some(id) {
                continue;
            }
            if let Some(err) = msg.get("error") {
                let text = err.to_string().to_lowercase();
                return Err(
                    if [
                        "not logged",
                        "not authenticated",
                        "unauthorized",
                        "sign in",
                        "401",
                    ]
                    .iter()
                    .any(|s| text.contains(s))
                    {
                        Failure::Login
                    } else {
                        Failure::Remote
                    },
                );
            }
            if !msg["result"].is_object() {
                return Err(Failure::Protocol);
            }
            return Ok(msg["result"].take());
        }
    }
    fn exchange(mut self) -> Result<Value, Failure> {
        self.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"ai_usage_tray","title":"AI Usage Tray","version":"1.0"},"capabilities":{}}}))?;
        self.response(1)?;
        self.send(json!({"method":"initialized","params":{}}))?;
        self.send(json!({"id":2,"method":"account/rateLimits/read","params":{}}))?;
        let result = match self.response(2) {
            Err(Failure::Remote) => {
                self.send(json!({"id":3,"method":"account/read","params":{"refreshToken":false}}))?;
                let account = self.response(3)?;
                if account["account"].is_null() && account["requiresOpenaiAuth"] != false {
                    return Err(Failure::Login);
                }
                if account["account"]["type"] == "apiKey" {
                    return Ok(empty("app-server", "unavailable"));
                }
                return Err(Failure::Remote);
            }
            other => other?,
        };
        let values = windows(&result);
        let status = if values.is_empty() {
            "unavailable"
        } else {
            "ok"
        };
        Ok(snapshot("app-server", values, Some(now()), status))
    }
}

pub fn read(paths: &Paths, cancel: &AtomicBool, timeout: Duration) -> Value {
    let Some(cli) = paths.find_cli("codex") else {
        return empty("app-server", "missing");
    };
    let mut dirs = vec![cli.parent().unwrap().to_owned()];
    if let Some(node) = paths.find_cli("node") {
        dirs.push(node.parent().unwrap().to_owned())
    }
    dirs.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut cmd = Command::new(cli);
    if let Ok(path) = env::join_paths(dirs) {
        cmd.env("PATH", path);
    }
    if let Some(profile) = paths.configured("CODEX_HOME") {
        cmd.env("CODEX_HOME", profile);
    }
    cmd.arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let Ok(mut child) = cmd.spawn() else {
        return empty("app-server", "error");
    };
    let rpc = Rpc {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
    };
    let (send, recv) = mpsc::channel();
    let io_job = thread::spawn(move || {
        let _ = send.send(rpc.exchange());
    });
    let deadline = Instant::now() + timeout;
    let result = loop {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            break empty("app-server", "timeout");
        }
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        match recv.recv_timeout(wait) {
            Ok(Ok(v)) => break v,
            Ok(Err(Failure::Login)) => break empty("app-server", "login"),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            _ => break empty("app-server", "error"),
        }
    };
    // Successful exchanges close stdin. Give app-server a moment to finish its own cleanup.
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
