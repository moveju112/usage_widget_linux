// SSH is used only by the systemd one-shot worker. The status line only touches local files.
use crate::{claude, common::*, install::remove_file};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

mod pull;
mod setup;
pub use pull::{vm, watch};
pub use setup::{install, receiver_key, uninstall};

const ACK: &[u8] = b"ok\n";
const MAX_PACKET: u64 = 4096;
const UNCHANGED_INTERVAL: f64 = 300.0;
const COALESCE: std::time::Duration = std::time::Duration::from_secs(15);

pub fn sender_dir() -> PathBuf {
    env_path("XDG_DATA_HOME")
        .unwrap_or_else(|| home().join(".local/share"))
        .join("ai-usage-tray-sender")
}

// GNOME, the login shell and the systemd user manager can have different XDG variables.
fn runtime_dir() -> io::Result<PathBuf> {
    Ok(env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("송신기 설치 경로 오류"))?
        .to_owned())
}

pub fn account_tag(profile: &Value) -> Option<String> {
    let uuid = profile["oauthAccount"]["accountUuid"].as_str()?.trim();
    (!uuid.is_empty()).then(|| {
        format!(
            "{:x}",
            Sha256::digest(format!("ai-usage-tray/claude/v1/{uuid}"))
        )
    })
}
fn valid_host(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-._@:[]".contains(&c))
}
fn valid_tag(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn locked(path: &Path) -> io::Result<fs::File> {
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}
fn invalid() -> io::Error {
    io::Error::other("사용량 패킷 또는 계정 식별자가 올바르지 않아.")
}
fn read_input() -> io::Result<Value> {
    let mut bytes = Vec::new();
    io::stdin().take(MAX_PACKET + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PACKET {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub fn enqueue(paths: &Paths, raw: &[u8], at: f64) -> io::Result<()> {
    let data: Value = serde_json::from_slice(raw)?;
    let windows = claude::windows(&data["rate_limits"], "used_percentage");
    if windows.is_empty() {
        return Ok(());
    }
    let dir = runtime_dir()?;
    let _lock = locked(&dir.join("queue.lock"))?;
    let config = json_file(&dir.join("sender.json")).ok_or_else(invalid)?;
    // The CLI profile can be large. Re-read it only when its inode, size or mtime changes.
    let profile = paths.profile();
    let fingerprint = json!(claude::signature(&profile).ok_or_else(invalid)?);
    let identity_file = dir.join("identity.json");
    let identity = match json_file(&identity_file) {
        Some(cached) if cached["signature"] == fingerprint => cached,
        _ => {
            let account = json_file(&profile)
                .and_then(|v| account_tag(&v))
                .ok_or_else(invalid)?;
            let identity = json!({"signature":fingerprint, "account":account});
            atomic(&identity_file, &serde_json::to_vec(&identity)?, 0o600)?;
            identity
        }
    };
    let account = &identity["account"];
    if &config["account"] != account {
        return Ok(());
    }
    let pending = dir.join("pending.json");
    let queued = json_file(&pending);
    let interval = if queued.is_some() {
        COALESCE.as_secs_f64()
    } else {
        UNCHANGED_INTERVAL
    };
    let previous = queued.or_else(|| json_file(&dir.join("sent.json")));
    let packet = json!({"v":1, "account":account, "received":at, "windows":windows});
    if let Some(prev) = previous {
        let last = prev["received"].as_f64().unwrap_or_default();
        if last >= at || (prev["windows"] == packet["windows"] && at - last < interval) {
            return Ok(());
        }
    }
    atomic(&pending, &serde_json::to_vec(&packet)?, 0o600)
}

pub fn flush() -> io::Result<()> {
    let dir = runtime_dir()?;
    let config = json_file(&dir.join("sender.json")).ok_or_else(invalid)?;
    let _single = locked(&dir.join("send.lock"))?;
    let pending = dir.join("pending.json");
    // Coalesce fast-changing values before starting another SSH handshake.
    if pending.exists()
        && let Ok(age) = fs::metadata(dir.join("sent.json")).and_then(|m| m.modified())
        && let Ok(age) = age.elapsed()
        && let Some(wait) = COALESCE.checked_sub(age)
    {
        std::thread::sleep(wait);
    }
    let packet = match fs::read(&pending) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let host = config["host"].as_str().ok_or_else(invalid)?;
    let key = config["key"].as_str().ok_or_else(invalid)?;
    // A wall clock deadline also bounds a peer that authenticates but never acknowledges.
    let mut child = Command::new("timeout")
        .args([
            "-k",
            "2s",
            "20s",
            "ssh",
            "-T",
            "-oBatchMode=yes",
            "-oStrictHostKeyChecking=yes",
            "-oIdentitiesOnly=yes",
            "-oClearAllForwardings=yes",
            "-oControlPath=none",
            "-oConnectTimeout=5",
            "-i",
            key,
            "--",
            host,
            "ai-usage-tray receive",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let wrote = child.stdin.take().unwrap().write_all(&packet);
    let mut reply = Vec::new();
    let read = child.stdout.take().unwrap().take(4).read_to_end(&mut reply);
    if !child.wait()?.success() || wrote.is_err() || read.is_err() || reply != ACK {
        return Err(io::Error::other(
            "SSH 수신 확인 실패. 최신 한 건을 보관하고 재시도해.",
        ));
    }
    let _lock = locked(&dir.join("queue.lock"))?;
    atomic(&dir.join("sent.json"), &packet, 0o600)?;
    if fs::read(&pending)? == packet {
        remove_file(&pending)?;
    }
    Ok(())
}

fn validate(packet: &Value, expected: &str) -> io::Result<()> {
    let at = packet["received"].as_f64().ok_or_else(invalid)?;
    let windows = packet["windows"].as_array().ok_or_else(invalid)?;
    if packet.as_object().map(|o| o.len()) != Some(4)
        || packet["v"].as_u64() != Some(1)
        || packet["account"] != expected
        || !valid_tag(expected)
        || !(at > 0.0 && at <= now() + 5.0)
        || !(1..=2).contains(&windows.len())
    {
        return Err(invalid());
    }
    let mut seen = Vec::new();
    for w in windows {
        let period = w["minutes"].as_u64().ok_or_else(invalid)?;
        if !matches!(period, 300 | 10080)
            || seen.contains(&period)
            || w.as_object().map(|o| o.len()) != Some(3)
            || w.get("resets").is_none()
            || !(0.0..=100.0).contains(&w["percent"].as_f64().ok_or_else(invalid)?)
            || !(w["resets"].is_null()
                || w["resets"].is_number() && timestamp(&w["resets"]).is_some())
        {
            return Err(invalid());
        }
        seen.push(period);
    }
    Ok(())
}
// Keeps the newest packet for the account this PC shows, whether a VM pushed it or this PC pulled it.
fn store(paths: &Paths, packet: &Value, expected: &str) -> io::Result<()> {
    validate(packet, expected)?;
    let dir = cache();
    fs::create_dir_all(&dir)?;
    let _lock = locked(&dir.join("receive.lock"))?;
    let target = dir.join("claude-remote.json");
    let previous = json_file(&target);
    let local = json_file(&paths.profile()).and_then(|v| account_tag(&v));
    let selected = local
        .as_deref()
        .or_else(|| previous.as_ref()?.get("account")?.as_str());
    if selected.is_some_and(|selected| selected != expected) {
        return Err(io::Error::other("메인 PC가 표시하는 Claude 계정과 달라."));
    }
    if !previous.is_some_and(|v| {
        v["account"] == expected && v["received"].as_f64() >= packet["received"].as_f64()
    }) {
        atomic(&target, &serde_json::to_vec(packet)?, 0o600)?;
    }
    Ok(())
}
pub fn receive(paths: &Paths, expected: &str) -> io::Result<()> {
    store(paths, &read_input()?, expected)?;
    io::stdout().write_all(ACK)
}
