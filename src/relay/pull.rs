// The main PC reads each VM's Claude CLI profile over SSH. Nothing is installed on the VM.
use super::*;
use crate::{cli, schedule::config_dir};
use std::{
    collections::HashMap,
    ffi::OsString,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const HOSTS: &str = "claude-vm-hosts";
const STATUS: &str = "claude-vm-status.json";
const INTERVAL: Duration = Duration::from_secs(60);
const LIMIT: u64 = 16 * 1024 * 1024;
// Run by the VM's login shell. Prints "none", "same", or the profile's mtime.size.inode and then its bytes.
const REMOTE: &str = r#"sh -c 'f=$HOME/.claude/.claude.json; [ -f "$f" ] || f=$HOME/.claude.json; s=$(stat -c %Y.%s.%i "$f" 2>/dev/null) || { echo none; exit 0; }; [ "$s" = "$1" ] && { echo same; exit 0; }; echo "$s"; head -c 16777216 "$f"' sh"#;

fn hosts_file() -> PathBuf {
    config_dir().join(HOSTS)
}
fn hosts() -> Vec<String> {
    fs::read_to_string(hosts_file())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| valid_host(s))
        .map(str::to_owned)
        .collect()
}

// Only the account tag and the usage numbers leave the exchange. The profile itself is dropped.
fn pull(paths: &Paths, host: &str, seen: Option<&str>, cancel: &AtomicBool) -> Value {
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-T",
        "-oBatchMode=yes",
        "-oConnectTimeout=5",
        "-oClearAllForwardings=yes",
        "--",
        host,
    ])
    .arg(format!("{REMOTE} {}", seen.unwrap_or("0")));
    let at = now();
    let mut result = cli::run(
        cmd,
        cancel,
        Duration::from_secs(20),
        "SSH",
        move |input, output| {
            drop(input);
            let mut bytes = Vec::new();
            if output.take(LIMIT + 64).read_to_end(&mut bytes).is_err() {
                return empty("SSH", "unreachable");
            }
            let (head, body) = match bytes.iter().position(|b| *b == b'\n') {
                Some(i) => (&bytes[..i], &bytes[i + 1..]),
                None => (&bytes[..], &[][..]),
            };
            let Ok(head) = std::str::from_utf8(head) else {
                return empty("SSH", "invalid");
            };
            match head {
                "" => return empty("SSH", "unreachable"),
                "none" | "same" => return empty("SSH", head),
                _ if !head.bytes().all(|c| c.is_ascii_digit() || c == b'.') => {
                    return empty("SSH", "invalid");
                }
                _ => {}
            }
            let Ok(data) = serde_json::from_slice::<Value>(body) else {
                return empty("SSH", "invalid");
            };
            let Some(account) = account_tag(&data) else {
                return empty("SSH", "no_account");
            };
            let Some(usage) = claude::local(&data, at) else {
                return json!({"status":"empty","sig":head});
            };
            let packet = json!({"v":1,"account":account,"received":usage["received"],"windows":usage["windows"]});
            json!({"status":"ok","sig":head,"packet":packet})
        },
    );
    if result["status"] == "ok"
        && store(
            paths,
            &result["packet"],
            result["packet"]["account"].as_str().unwrap_or_default(),
        )
        .is_err()
    {
        result["status"] = "rejected".into();
    }
    if !matches!(
        result["status"].as_str(),
        Some("ok" | "same" | "empty" | "none" | "no_account" | "invalid" | "rejected")
    ) {
        result["status"] = "unreachable".into();
    }
    // A read cut short because the tray is stopping says nothing about the VM.
    if !cancel.load(Ordering::Relaxed) {
        let _ = record(Some((host, &result)));
    }
    result
}

// One entry per registered VM for the settings window: what the last read found and when.
fn record(read: Option<(&str, &Value)>) -> io::Result<()> {
    let dir = cache();
    fs::create_dir_all(&dir)?;
    let _lock = locked(&dir.join("vm-status.lock"))?;
    let target = dir.join(STATUS);
    let list = hosts();
    let mut all = json_file(&target)
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    all.retain(|host, _| list.contains(host));
    if let Some((host, result)) = read
        && list.iter().any(|known| known == host)
    {
        let status = result["status"].as_str().unwrap_or("unreachable");
        let previous = all.get(host);
        let entry = if status == "same" {
            // The profile has not changed since a read that was already recorded.
            previous.cloned().map(|mut entry| {
                entry["checked"] = json!(now());
                entry
            })
        } else {
            let received = match status {
                "ok" => result["packet"]["received"].clone(),
                // Keep showing when the last value was read while the VM cannot be reached.
                "unreachable" => previous.map_or(Value::Null, |v| v["received"].clone()),
                _ => Value::Null,
            };
            Some(
                json!({"status":status,"text":describe(host, result).unwrap_or_else(|e| e.to_string()),"received":received,"checked":now()}),
            )
        };
        if let Some(entry) = entry {
            all.insert(host.to_owned(), entry);
        }
    }
    atomic(&target, &serde_json::to_vec(&all)?, 0o600)
}

pub fn watch(paths: Arc<Paths>, cancel: Arc<AtomicBool>, wake: mpsc::Receiver<()>) {
    let mut seen: HashMap<String, String> = HashMap::new();
    loop {
        while wake.try_recv().is_ok() {}
        let version = claude::signature(&hosts_file());
        let list = hosts();
        seen.retain(|host, _| list.contains(host));
        for host in &list {
            let result = pull(&paths, host, seen.get(host).map(String::as_str), &cancel);
            // A rejected or unreadable profile is read again, so a later fix shows up by itself.
            if matches!(result["status"].as_str(), Some("ok" | "empty"))
                && let Some(sig) = result["sig"].as_str()
            {
                seen.insert(host.clone(), sig.to_owned());
            }
        }
        // A manual refresh or a changed host list starts the next round early.
        let until = Instant::now() + INTERVAL;
        loop {
            match wake.recv_timeout(Duration::from_secs(2)) {
                Ok(()) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= until || claude::signature(&hosts_file()) != version {
                        break;
                    }
                }
            }
        }
    }
}

// What the latest read of a VM means to the user. An unreachable host is an error.
fn describe(host: &str, result: &Value) -> io::Result<String> {
    Ok(match result["status"].as_str() {
        Some("ok") => {
            let packet = &result["packet"];
            let values: Vec<_> = packet["windows"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|w| {
                    let period = if w["minutes"] == 300 {
                        "5시간"
                    } else {
                        "7일"
                    };
                    format!("{period} {}%", w["percent"])
                })
                .collect();
            values.join(" · ")
        }
        Some("rejected") => {
            "지금 값은 이 PC의 Claude 계정과 다르거나 VM 시계가 맞지 않아 표시하지 않아.".into()
        }
        Some("empty") => "VM의 Claude Code가 아직 사용량을 기록하지 않았어.".into(),
        Some("no_account") => "VM의 Claude Code가 구독 계정으로 로그인돼 있지 않아.".into(),
        Some("none") => "VM에서 Claude Code 프로필을 아직 찾지 못했어.".into(),
        Some("invalid") => "VM의 Claude Code 프로필을 읽을 수 없어. 16MB를 넘거나 로그인 셸이 다른 글자를 출력하는지 확인해줘.".into(),
        _ => {
            return Err(io::Error::other(format!(
                "SSH로 접속할 수 없어. ssh -oBatchMode=yes {host} true 가 성공하는지 확인해줘."
            )));
        }
    })
}

fn save(list: &[String]) -> io::Result<()> {
    let mut text = list.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    atomic(&hosts_file(), text.as_bytes(), 0o600)
}

pub fn vm(paths: &Paths, action: &str, args: &[OsString]) -> io::Result<()> {
    if action == "vm-list" {
        let list = hosts();
        if list.is_empty() {
            println!("등록된 VM이 없어.");
        }
        for host in list {
            println!("{host}");
        }
        return Ok(());
    }
    let [host] = args else {
        return Err(io::Error::other(
            "사용법: vm-add SSH_HOST | vm-remove SSH_HOST | vm-test SSH_HOST | vm-list",
        ));
    };
    let host = host
        .to_str()
        .filter(|s| valid_host(s))
        .ok_or_else(|| io::Error::other("SSH 호스트 또는 user@host를 입력해줘."))?;
    let read = || {
        let result = pull(paths, host, None, &AtomicBool::new(false));
        let text = describe(host, &result)?;
        Ok::<_, io::Error>(match result["packet"]["received"].as_f64() {
            Some(at) => format!("{text} ({:.0}분 전 기록)", ((now() - at) / 60.0).max(0.0)),
            None => text,
        })
    };
    if action == "vm-test" {
        println!("{}", read()?);
        return Ok(());
    }
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let _lock = locked(&dir.join("claude-vm-hosts.lock"))?;
    let before = hosts();
    let mut list = before.clone();
    list.retain(|known| known != host);
    let message = if action == "vm-remove" {
        format!("{host} 등록을 해제했어.")
    } else {
        list.push(host.to_owned());
        save(&list)?;
        match read() {
            Ok(detail) => format!("{host} 등록 완료. {detail}"),
            Err(e) => {
                // A host that was already registered stays registered.
                save(&before)?;
                record(None)?;
                return Err(io::Error::other(format!("등록하지 않았어. {e}")));
            }
        }
    };
    if action == "vm-remove" {
        save(&list)?;
        record(None)?;
    }
    println!("{message}");
    Ok(())
}
