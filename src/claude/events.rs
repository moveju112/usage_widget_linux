use super::{Signature, signature};
use crate::common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub(crate) const SETTING: &str = "claude-events-path";

fn small_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

pub(super) fn account_id(profile: &Value) -> Option<String> {
    let id = profile["oauthAccount"]["accountUuid"].as_str()?.trim();
    (!id.is_empty()).then(|| format!("{:x}", Sha256::digest(format!("claude-account-v1/{id}"))))
}

#[derive(Default)]
pub(super) struct FileSource {
    setting_version: Option<Signature>,
    path: Option<PathBuf>,
    version: Option<(PathBuf, Option<Signature>)>,
    data: Option<Value>,
}

impl FileSource {
    pub(super) fn read(
        &mut self,
        account: Option<&str>,
        allow_standalone: bool,
        at: f64,
    ) -> Option<Value> {
        let setting = crate::schedule::config_dir().join(SETTING);
        let revision = signature(&setting);
        if revision != self.setting_version {
            self.setting_version = revision;
            self.path = small_file(&setting, 8192)
                .and_then(|v| String::from_utf8(v).ok())
                .map(|s| PathBuf::from(s.trim()))
                .filter(|p| p.is_absolute());
            self.data = None;
            self.version = None;
        }
        let configured = self.path.as_ref()?;
        let path = if configured.is_dir() {
            configured.join("claude.json")
        } else {
            configured.clone()
        };
        let version = (path.clone(), signature(&path));
        if self.version.as_ref() != Some(&version) {
            self.data = small_file(&path, 16384).and_then(|v| serde_json::from_slice(&v).ok());
            self.version = Some(version);
        }
        let data = self.data.as_ref()?;
        if data["version"].as_u64() != Some(1) {
            return None;
        }
        let file_account = match data.get("account")? {
            Value::Null => None,
            Value::String(id)
                if id.len() == 64
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) =>
            {
                Some(id.as_str())
            }
            _ => return None,
        };
        if account.is_some() && file_account != account || account.is_none() && !allow_standalone {
            return None;
        }
        let mut rows = Vec::new();
        for (key, minutes) in [("five_hour", 300), ("seven_day", 10080)] {
            let v = &data["windows"][key];
            let Some(t) = timestamp(&v["received_at"]).filter(|t| *t > 0.0 && *t <= at + 5.0)
            else {
                continue;
            };
            if !v["resets_at"].is_null() && timestamp(&v["resets_at"]).is_none() {
                continue;
            }
            if let Some(mut row) = window(&json!(minutes), &v["used_percent"], &v["resets_at"]) {
                row["received"] = json!(t);
                rows.push(row);
            }
        }
        let latest = rows
            .iter()
            .filter_map(|w| w["received"].as_f64())
            .reduce(f64::max)?;
        Some(snapshot("이벤트 파일", rows, Some(latest), "ok"))
    }
}
