use crate::common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
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
fn windows(data: &Value, field: &str) -> Vec<Value> {
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
        && (previous["observed"].as_f64().is_some_and(|t| t > at)
            || (status == "waiting"
                && previous["windows"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())))
    {
        return Ok(());
    }
    atomic(&target, &serde_json::to_vec(&sample)?, 0o600)
}
fn local(data: &Value, at: f64) -> Option<Value> {
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
            let w = window(&item["minutes"], &item["percent"], &item["resets"])?;
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
type Signature = (u64, i64, i64, u64);
fn signature(path: &Path) -> Option<Signature> {
    let m = fs::metadata(path).ok()?;
    Some((m.ino(), m.mtime(), m.mtime_nsec(), m.len()))
}
#[derive(Default)]
pub struct Reader {
    profile_version: Option<(PathBuf, Option<Signature>)>,
    hook_version: Option<(String, Option<Signature>)>,
    identity: String,
    local: Option<Value>,
    hook: Option<Value>,
}
impl Reader {
    pub fn read(&mut self, paths: &Paths) -> Value {
        if paths.find_cli("claude").is_none() {
            return empty("Claude Code", "missing");
        }
        let at = now();
        let profile = paths.profile();
        let version = (profile.clone(), signature(&profile));
        if self.profile_version.as_ref() != Some(&version) {
            let data = json_file(&profile).unwrap_or_default();
            self.identity = scope(&profile, &data);
            self.local = local(&data, at);
            self.profile_version = Some(version);
        }
        let hook = cache().join("claude.json");
        let version = (self.identity.clone(), signature(&hook));
        if self.hook_version.as_ref() != Some(&version) {
            self.hook = json_file(&hook).and_then(|d| from_hook(&d, &self.identity, at));
            self.hook_version = Some(version);
        }
        let local_time = self
            .local
            .as_ref()
            .and_then(|v| v["received"].as_f64())
            .unwrap_or_default();
        if let Some(h) = &self.hook {
            let t = if h["status"] == "unavailable" {
                &h["observed"]
            } else {
                &h["received"]
            };
            if t.as_f64().unwrap_or_default() >= local_time {
                return h.clone();
            }
        }
        self.local
            .clone()
            .unwrap_or_else(|| empty("Claude Code", "waiting"))
    }
}
