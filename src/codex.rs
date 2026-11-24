use crate::{common::*, schedule::Call};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    process::{ChildStdin, ChildStdout},
    sync::atomic::AtomicBool,
    time::Duration,
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
    fn message(&mut self) -> Result<Value, Failure> {
        let mut line = Vec::new();
        let size = self
            .output
            .by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if size == 0 || size > 4 * 1024 * 1024 {
            return Err(Failure::Protocol);
        }
        Ok(serde_json::from_slice(&line)?)
    }
    fn response(&mut self, id: i64) -> Result<Value, Failure> {
        loop {
            let mut msg = self.message()?;
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
    fn initialize(&mut self) -> Result<(), Failure> {
        self.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"ai_usage_tray","title":"AI Usage Tray","version":"1.0"},"capabilities":{}}}))?;
        self.response(1)?;
        self.send(json!({"method":"initialized","params":{}}))
    }
    fn latest_luna(&mut self) -> Result<Option<String>, Failure> {
        let mut cursor = Value::Null;
        let mut seen = std::collections::HashSet::new();
        let mut best = None;
        loop {
            self.send(
                json!({"id":6,"method":"model/list","params":{"cursor":cursor,"limit":100}}),
            )?;
            let page = self.response(6)?;
            for entry in page["data"].as_array().ok_or(Failure::Protocol)? {
                let Some(id) = entry["model"].as_str() else {
                    continue;
                };
                if entry["hidden"] == true {
                    continue;
                }
                let Some(version) = id
                    .strip_prefix("gpt-")
                    .and_then(|s| s.strip_suffix("-luna"))
                else {
                    continue;
                };
                let Ok(parts) = version
                    .split('.')
                    .map(str::parse::<u64>)
                    .collect::<Result<Vec<_>, _>>()
                else {
                    continue;
                };
                let candidate = (parts, id.to_owned());
                if best.as_ref().is_none_or(|current| &candidate > current) {
                    best = Some(candidate);
                }
            }
            let Some(next) = page["nextCursor"].as_str().filter(|s| !s.is_empty()) else {
                break;
            };
            if !seen.insert(next.to_owned()) {
                return Err(Failure::Protocol);
            }
            cursor = next.into();
        }
        Ok(best.map(|(_, id)| id))
    }
    fn scheduled(&mut self, call: &Call, cwd: &std::path::Path) -> Result<Value, Failure> {
        self.send(json!({"id":2,"method":"account/read","params":{"refreshToken":false}}))?;
        if self.response(2)?["account"]["type"] != "chatgpt" {
            return Err(Failure::Login);
        }
        self.send(json!({"id":5,"method":"account/rateLimits/read","params":{}}))?;
        if windows(&self.response(5)?).iter().any(exhausted) {
            return Ok(empty("scheduled", "limited"));
        }
        let model = if call.model.is_empty() {
            let Some(id) = self.latest_luna()? else {
                return Ok(empty("scheduled", "no_model"));
            };
            id
        } else {
            call.model.clone()
        };
        self.send(json!({"id":3,"method":"thread/start","params":{
            "model":model, "modelProvider":"openai", "cwd":cwd, "ephemeral":true,
            "sandbox":"read-only", "approvalPolicy":"never",
            "baseInstructions":"Answer the user's message directly without using tools.",
            "config":{"project_doc_max_bytes":0, "web_search":"disabled",
                "features.shell_tool":false, "features.unified_exec":false,
                "features.hooks":false, "features.multi_agent_v2":false, "features.apps":false,
                "mcp_servers":{}}
        }}))?;
        let started = self.response(3)?;
        if started["model"] != model || started["modelProvider"] != "openai" {
            return Err(Failure::Remote);
        }
        let thread = started["thread"]["id"].as_str().ok_or(Failure::Protocol)?;
        if let Some(status) = call.skipped() {
            return Ok(empty("scheduled", status));
        }
        let sent_at = now().ceil() as i64;
        self.send(json!({"id":4,"method":"turn/start","params":{
            "threadId":thread, "input":[{"type":"text","text":"Reply with OK only."}]
        }}))?;
        let started = self.response(4)?;
        let turn = started["turn"]["id"].as_str().ok_or(Failure::Protocol)?;
        loop {
            let message = self.message()?;
            let params = &message["params"];
            if message["method"] == "turn/completed"
                && params["threadId"] == thread
                && params["turn"]["id"] == turn
            {
                return match params["turn"]["status"].as_str() {
                    Some("completed") => {
                        let mut result = empty("scheduled", "ok");
                        result["selectedModel"] = model.into();
                        result["sentAt"] = sent_at.into();
                        Ok(result)
                    }
                    _ => Err(Failure::Remote),
                };
            }
            // An unattended turn cannot approve tool calls or answer follow-up requests.
            if message.get("id").is_some() && message.get("method").is_some() {
                return Err(Failure::Remote);
            }
        }
    }
    fn usage(&mut self) -> Result<Value, Failure> {
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
    run(paths, cancel, timeout, None)
}
pub fn scheduled(paths: &Paths, cancel: &AtomicBool, call: Call) -> Value {
    let Ok(workspace) = tempfile::tempdir() else {
        return empty("scheduled", "error");
    };
    run(
        paths,
        cancel,
        Duration::from_secs(90),
        Some((call, workspace)),
    )
}
fn run(
    paths: &Paths,
    cancel: &AtomicBool,
    timeout: Duration,
    scheduled: Option<(Call, tempfile::TempDir)>,
) -> Value {
    let Some(mut cmd) = crate::cli::command(paths, "codex") else {
        return empty("app-server", "missing");
    };
    if let Some(profile) = paths.configured("CODEX_HOME") {
        cmd.env("CODEX_HOME", profile);
    }
    if let Some((_, workspace)) = &scheduled {
        cmd.current_dir(workspace.path())
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY");
    }
    cmd.arg("app-server");
    crate::cli::run(cmd, cancel, timeout, "app-server", move |input, output| {
        let mut rpc = Rpc {
            input,
            output: BufReader::new(output),
        };
        match rpc.initialize().and_then(|()| match scheduled {
            Some((call, workspace)) => rpc.scheduled(&call, workspace.path()),
            None => rpc.usage(),
        }) {
            Ok(value) => value,
            Err(Failure::Login) => empty("app-server", "login"),
            Err(_) => empty("app-server", "error"),
        }
    })
}
