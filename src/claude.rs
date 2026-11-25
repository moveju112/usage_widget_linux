pub(crate) mod events;
use crate::{cli, common::*, schedule::Call};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

fn scope(profile: &Path, data: &Value) -> String {
    let mut hash = Sha256::new();
    hash.update(profile.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(
        data["oauthAccount"]["accountUuid"]
            .as_str()
            .unwrap_or_default(),
    );
    format!("{:x}", hash.finalize())
}
pub(crate) fn windows(data: &Value, field: &str) -> Vec<Value> {
    [("five_hour", 300), ("seven_day", 10080)]
        .into_iter()
        .filter_map(|(key, minutes)| {
            let v = &data[key];
            window(&json!(minutes), &v[field], &v["resets_at"])
        })
        .collect()
}
pub fn capture(paths: &Paths, raw: &[u8], at: f64) -> io::Result<()> {
    let Ok(data) = serde_json::from_slice::<Value>(raw) else {
        return Ok(());
    };
    if !data.is_object() {
        return Ok(());
    }
    let values = windows(&data["rate_limits"], "used_percentage");
    let status = if !values.is_empty() {
        "ok"
    } else if data["context_window"]["current_usage"].is_object() {
        "unavailable"
    } else {
        "waiting"
    };
    let mut sample = snapshot("statusLine", values, (status == "ok").then_some(at), status);
    let dir = cache();
    fs::create_dir_all(&dir)?;
    let target = dir.join("claude.json");
    let lock = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join("capture.lock"))?;
    lock.lock()?;
    let profile = paths.profile();
    sample["profile"] = scope(&profile, &json_file(&profile).unwrap_or_default()).into();
    sample["observed"] = json!(at);
    if let Some(previous) = json_file(&target)
        && previous["profile"] == sample["profile"]
    {
        if previous["observed"].as_f64().is_some_and(|t| t > at)
            || (status == "waiting"
                && previous["windows"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()))
        {
            return Ok(());
        }
        if status == "ok"
            && let Some(old) = from_hook(
                &previous,
                sample["profile"].as_str().unwrap_or_default(),
                at,
            )
            && let Some(mut merged) = merge_windows([&sample, &old])
        {
            sample["windows"] = merged["windows"].take();
        }
    }
    atomic(&target, &serde_json::to_vec(&sample)?, 0o600)
}
pub(crate) fn local(data: &Value, at: f64) -> Option<Value> {
    let entry = &data["cachedUsageUtilization"];
    let account = data["oauthAccount"]["accountUuid"].as_str()?;
    if account.is_empty() || entry["accountUuid"].as_str() != Some(account) {
        return None;
    }
    let received = entry["fetchedAtMs"].as_f64()? / 1000.0;
    if !(0.0 < received && received <= at + 5.0) {
        return None;
    }
    let values = windows(&entry["utilization"], "utilization");
    (!values.is_empty()).then(|| snapshot("CLI 캐시", values, Some(received), "ok"))
}
fn from_hook(data: &Value, profile: &str, at: f64) -> Option<Value> {
    if data["profile"].as_str()? != profile {
        return None;
    }
    let values: Vec<_> = data["windows"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            let mut w = window(&item["minutes"], &item["percent"], &item["resets"])?;
            if let Some(t) = timestamp(&item["received"]).filter(|t| *t > 0.0 && *t <= at + 5.0) {
                w["received"] = json!(t);
            }
            matches!(w["minutes"].as_u64(), Some(300 | 10080)).then_some(w)
        })
        .collect();
    if !values.is_empty()
        && let Some(t) = timestamp(&data["received"])
        && t > 0.0
        && t <= at + 5.0
    {
        return Some(snapshot("statusLine", values, Some(t), "ok"));
    }
    let status = data["status"].as_str()?;
    if !matches!(status, "waiting" | "unavailable") {
        return None;
    }
    let mut sample = empty("statusLine", status);
    sample["observed"] = json!(timestamp(&data["observed"]));
    Some(sample)
}
// Inputs first in the list win ties. Every window keeps its own observation time.
fn merge_windows<'a>(sources: impl IntoIterator<Item = &'a Value>) -> Option<Value> {
    let mut selected = std::collections::BTreeMap::new();
    for source in sources {
        for w in source["windows"].as_array().into_iter().flatten() {
            let Some(minutes @ (300 | 10080)) = w["minutes"].as_u64() else {
                continue;
            };
            let Some(at) = w["received"]
                .as_f64()
                .or_else(|| source["received"].as_f64())
            else {
                continue;
            };
            if selected
                .get(&minutes)
                .is_some_and(|(previous, _, _)| *previous >= at)
            {
                continue;
            }
            let mut row = w.clone();
            row["received"] = json!(at);
            selected.insert(
                minutes,
                (at, source["source"].as_str().unwrap_or("Claude Code"), row),
            );
        }
    }
    let &(at, source, _) = selected.values().max_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(snapshot(
        source,
        selected.into_values().map(|(_, _, row)| row).collect(),
        Some(at),
        "ok",
    ))
}

pub(crate) type Signature = (u64, u64, i64, i64, u64);
pub(crate) fn signature(path: &Path) -> Option<Signature> {
    let m = fs::metadata(path).ok()?;
    Some((m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.len()))
}
#[derive(Default)]
pub struct Reader {
    profile_version: Option<(PathBuf, Option<Signature>)>,
    hook_version: Option<(String, Option<Signature>)>,
    identity: String,
    account: Option<String>,
    event_account: Option<String>,
    events: events::FileSource,
    remote_version: Option<Signature>,
    remote: Option<Value>,
    local: Option<Value>,
    hook: Option<Value>,
}
impl Reader {
    pub fn read(&mut self, paths: &Paths) -> Value {
        let at = now();
        let profile = paths.profile();
        let version = (profile.clone(), signature(&profile));
        if self.profile_version.as_ref() != Some(&version) {
            let data = json_file(&profile).unwrap_or_default();
            self.identity = scope(&profile, &data);
            self.account = crate::relay::account_tag(&data);
            self.event_account = events::account_id(&data);
            self.local = local(&data, at);
            self.profile_version = Some(version);
        }
        let remote_path = cache().join("claude-remote.json");
        let version = signature(&remote_path);
        if self.remote_version != version {
            self.remote = json_file(&remote_path);
            self.remote_version = version;
        }
        let has_cli = paths.find_cli("claude").is_some();
        let local = if has_cli {
            self.local_read(at)
        } else {
            empty("Claude Code", "missing")
        };
        let remote = self.remote.as_ref().filter(|v| {
            self.account
                .as_ref()
                .is_none_or(|id| v["account"].as_str() == Some(id))
        });
        let remote = remote.map(|r| {
            snapshot(
                "SSH",
                r["windows"].as_array().cloned().unwrap_or_default(),
                r["received"].as_f64(),
                "ok",
            )
        });
        if self.account.is_none()
            && let Some(remote) = remote
        {
            return merge_windows([&remote]).unwrap_or(remote);
        }
        let file = self
            .events
            .read(self.event_account.as_deref(), !has_cli, at);
        merge_windows(
            [Some(&local), remote.as_ref(), file.as_ref()]
                .into_iter()
                .flatten(),
        )
        .unwrap_or(local)
    }

    fn local_read(&mut self, at: f64) -> Value {
        let hook = cache().join("claude.json");
        let version = (self.identity.clone(), signature(&hook));
        if self.hook_version.as_ref() != Some(&version) {
            self.hook = json_file(&hook).and_then(|d| from_hook(&d, &self.identity, at));
            self.hook_version = Some(version);
        }
        if let Some(h) = &self.hook
            && h["status"] == "unavailable"
            && h["observed"].as_f64() >= self.local.as_ref().and_then(|v| v["received"].as_f64())
        {
            return h.clone();
        }
        merge_windows(self.hook.iter().chain(self.local.iter()))
            .unwrap_or_else(|| empty("Claude Code", "waiting"))
    }
}

fn latest_haiku(models: &[Value]) -> Option<(&str, &str)> {
    models
        .iter()
        .filter_map(|model| {
            let id = model["value"].as_str()?;
            if id != "haiku" && !id.starts_with("claude-") {
                return None;
            }
            if !id.split('-').any(|part| part == "haiku") {
                return None;
            }
            let name = model["displayName"].as_str().unwrap_or(id);
            let mut parts = id
                .split('-')
                .filter(|v| v.len() < 4)
                .filter_map(|v| v.parse::<u32>().ok())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                let version = name
                    .split(|c: char| !c.is_ascii_digit() && c != '.')
                    .find(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                    .unwrap_or("");
                parts = version
                    .split('.')
                    .filter_map(|v| v.parse::<u32>().ok())
                    .collect();
            }
            Some(((parts, id != "haiku", id), (id, name)))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, model)| model)
}

fn haiku_catalog(mut command: std::process::Command, cancel: &AtomicBool) -> Value {
    command.args([
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
    ]);
    cli::run(
        command,
        cancel,
        Duration::from_secs(15),
        "scheduled",
        |mut input, output| {
            let request = br#"{"type":"control_request","request_id":"catalog","request":{"subtype":"initialize"}}"#;
            if input
                .write_all(request)
                .and_then(|()| input.write_all(b"\n"))
                .is_err()
            {
                return empty("scheduled", "error");
            }
            for line in BufReader::new(output.take(4 * 1024 * 1024))
                .lines()
                .map_while(Result::ok)
            {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let response = &msg["response"];
                if msg["type"] != "control_response" || response["request_id"] != "catalog" {
                    continue;
                }
                let Some(models) = response["response"]["models"].as_array() else {
                    break;
                };
                return match latest_haiku(models) {
                    Some((id, label)) => json!({"status":"ok","model":id,"label":label}),
                    None => empty("scheduled", "no_model"),
                };
            }
            empty("scheduled", "error")
        },
    )
}

pub fn scheduled(paths: &Paths, cancel: &AtomicBool, call: Call) -> Value {
    let Some(mut command) = cli::command(paths, "claude") else {
        return empty("scheduled", "missing");
    };
    let Ok(workspace) = tempfile::tempdir() else {
        return empty("scheduled", "error");
    };
    let prepare = |cmd: &mut std::process::Command| {
        cmd.current_dir(workspace.path()).args([
            "--safe-mode",
            "--restricted",
            "--strict-mcp-config",
        ]);
        // The installer records the default settings directory too. Exporting it
        // explicitly would move the CLI's default ~/.claude.json account profile.
        if let Some(dir) = paths.configured("CLAUDE_CONFIG_DIR")
            && (dir != home().join(".claude") || dir.join(".claude.json").is_file())
        {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
    };
    prepare(&mut command);
    command.args(["auth", "status", "--json"]);
    let auth = cli::run(
        command,
        cancel,
        Duration::from_secs(15),
        "scheduled",
        |input, output| {
            drop(input);
            let info: Value = match serde_json::from_reader(output.take(64 * 1024)) {
                Ok(value) => value,
                Err(_) => return empty("scheduled", "error"),
            };
            let signed_in = info["loggedIn"] == true
                && info["authMethod"] == "claude.ai"
                && info["apiProvider"] == "firstParty";
            empty("scheduled", if signed_in { "ok" } else { "login" })
        },
    );
    if auth["status"] != "ok" {
        return auth;
    }
    let usage = Reader::default().read(paths);
    if usage["windows"]
        .as_array()
        .is_some_and(|windows| windows.iter().any(crate::common::exhausted))
    {
        return empty("scheduled", "limited");
    }
    let Some(mut command) = cli::command(paths, "claude") else {
        return empty("scheduled", "missing");
    };
    prepare(&mut command);
    let (model, label) = if call.model.is_empty() {
        let Some(mut catalog) = cli::command(paths, "claude") else {
            return empty("scheduled", "missing");
        };
        prepare(&mut catalog);
        let selected = haiku_catalog(catalog, cancel);
        if selected["status"] != "ok" {
            return selected;
        }
        (
            selected["model"].as_str().unwrap().to_owned(),
            selected["label"].as_str().unwrap().to_owned(),
        )
    } else {
        (call.model.clone(), call.model.clone())
    };
    command.args([
        "--print",
        "--model",
        &model,
        "--max-turns",
        "1",
        "--tools",
        "",
        "--permission-mode",
        "dontAsk",
        "--no-session-persistence",
        "--system-prompt",
        "Answer briefly without using tools.",
        "--output-format",
        "json",
        "--",
        "Reply with OK only.",
    ]);
    if let Some(status) = call.skipped() {
        return empty("scheduled", status);
    }
    let sent_at = now().ceil() as i64;
    cli::run(
        command,
        cancel,
        Duration::from_secs(90),
        "scheduled",
        move |input, output| {
            drop(input);
            let result: Value =
                serde_json::from_reader(output.take(512 * 1024)).unwrap_or_default();
            let status = if result["type"] == "result"
                && result["subtype"] == "success"
                && result["is_error"] == false
            {
                "ok"
            } else if result["api_error_status"] == 429 || result["error"] == "rate_limit" {
                "limited"
            } else {
                "error"
            };
            let mut info = empty("scheduled", status);
            info["selectedModel"] = label.into();
            info["sentAt"] = sent_at.into();
            info
        },
    )
}
