use chrono::DateTime;
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME 설정이 필요해")
}
pub fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
pub fn cache() -> PathBuf {
    env_path("XDG_CACHE_HOME")
        .unwrap_or_else(|| home().join(".cache"))
        .join("ai-usage-tray")
}
pub fn json_file(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
pub fn atomic(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut tmp = NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| io::Error::other("파일 경로 오류"))?,
    )?;
    tmp.write_all(bytes)?;
    tmp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    tmp.persist(path)?;
    Ok(())
}
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub fn timestamp(value: &Value) -> Option<f64> {
    let number = value.as_f64().or_else(|| {
        DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()
            .map(|date| date.timestamp() as f64 + f64::from(date.timestamp_subsec_nanos()) / 1e9)
    })?;
    (0.0..253402300800.0).contains(&number).then_some(number)
}
pub fn window(minutes: &Value, percent: &Value, reset: &Value) -> Option<Value> {
    let duration = minutes.as_f64()?;
    if !(1.0..=1e16).contains(&duration)
        || duration.fract() != 0.0
        || !(0.0..=100.0).contains(&percent.as_f64()?)
    {
        return None;
    }
    Some(json!({"minutes":duration as u64,"percent":percent,"resets":timestamp(reset)}))
}
pub fn snapshot(source: &str, windows: Vec<Value>, received: Option<f64>, status: &str) -> Value {
    json!({"source":source,"windows":windows,"received":received,"status":status})
}
pub fn empty(source: &str, status: &str) -> Value {
    snapshot(source, vec![], None, status)
}

pub struct Paths {
    pub hints: Value,
}
impl Paths {
    pub fn load() -> Self {
        let hints = env::current_exe()
            .ok()
            .and_then(|p| json_file(&p.with_file_name("cli-paths.json")))
            .unwrap_or_default();
        Self { hints }
    }
    pub fn configured(&self, key: &str) -> Option<PathBuf> {
        env_path(key).or_else(|| self.hints[key].as_str().map(PathBuf::from))
    }
    pub fn find_cli(&self, name: &str) -> Option<PathBuf> {
        let h = home();
        let mut candidates: Vec<_> = self.hints[name]
            .as_str()
            .map(PathBuf::from)
            .into_iter()
            .collect();
        candidates.extend(
            env::split_paths(&env::var_os("PATH").unwrap_or_default()).map(|p| p.join(name)),
        );
        for dir in [
            ".local/bin",
            "bin",
            ".npm-global/bin",
            ".volta/bin",
            ".local/share/pnpm",
            "/snap/bin",
            "/usr/local/bin",
        ] {
            candidates.push(h.join(dir).join(name));
        }
        let nvm = h.join(".nvm/versions/node");
        let mut paths: Vec<_> = fs::read_dir(nvm)
            .into_iter()
            .flatten()
            .flatten()
            .map(|v| v.path().join("bin").join(name))
            .collect();
        paths.sort_unstable_by(|a, b| b.cmp(a));
        candidates.extend(paths);
        candidates.into_iter().find(|p| {
            p.is_absolute() && fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0)
        })
    }
    pub fn profile(&self) -> PathBuf {
        let default = home().join(".claude");
        let dir = self
            .configured("CLAUDE_CONFIG_DIR")
            .unwrap_or_else(|| default.clone());
        let inner = dir.join(".claude.json");
        if dir == default && !inner.is_file() {
            home().join(".claude.json")
        } else {
            inner
        }
    }
}
