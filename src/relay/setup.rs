use super::*;
use crate::install::{destination, installed_version, sender_hook, shell_quote};
use std::ffi::OsString;

const NAME: &str = "ai-usage-tray-sender";
const WATCH: &str = "ai-usage-tray-sender.path";
const SERVICE: &str = "ai-usage-tray-sender.service";
const HEADER: &str = "# Managed by ai-usage-tray sender-install\n";
const MARK: &str = ".ai-usage-tray-sender";

fn units() -> PathBuf {
    env_path("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home().join(".config"))
        .join("systemd/user")
}
fn systemctl(args: &[&str]) -> io::Result<()> {
    if Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .status()?
        .success()
    {
        Ok(())
    } else {
        Err(io::Error::other(
            "systemd 사용자 서비스 관리자에 연결할 수 없어. sender-install 또는 sender-uninstall을 다시 실행해줘.",
        ))
    }
}
fn unit_path(path: &Path) -> io::Result<String> {
    let s = path
        .to_str()
        .filter(|s| s.starts_with('/') && !s.contains('\\') && !s.chars().any(char::is_control))
        .ok_or_else(|| {
            io::Error::other(
                "systemd 경로는 절대 경로여야 하고 제어 문자나 역슬래시를 포함할 수 없어.",
            )
        })?;
    Ok(s.replace('%', "%%"))
}
fn managed(path: &Path) -> io::Result<()> {
    if path.is_symlink() {
        return Err(io::Error::other("서비스 경로가 심볼릭 링크야."));
    }
    match fs::read(path) {
        Ok(data) if !data.starts_with(HEADER.as_bytes()) => {
            Err(io::Error::other("같은 이름의 다른 systemd 유닛이 있어."))
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
fn public_key(text: &str) -> io::Result<String> {
    let mut fields = text.split_whitespace();
    if fields.next() != Some("ssh-ed25519") {
        return Err(io::Error::other("VM별 전용 Ed25519 공개키가 필요해."));
    }
    let key = fields
        .next()
        .filter(|s| {
            s.len() == 68
                && s.starts_with("AAAAC3NzaC1lZDI1NTE5AAAAI")
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+/".contains(&c))
        })
        .ok_or_else(invalid)?;
    Ok(format!("ssh-ed25519 {key}"))
}
fn ensure_owned(dir: &Path) -> io::Result<()> {
    if dir.is_symlink() || fs::read(dir.join(MARK)).ok().as_deref() != Some(NAME.as_bytes()) {
        return Err(io::Error::other("송신기 설치 경로에 다른 파일이 있어."));
    }
    Ok(())
}

pub fn install(paths: &Paths, args: &[OsString]) -> io::Result<()> {
    let [host, key] = args else {
        return Err(io::Error::other(
            "사용법: sender-install SSH_HOST ED25519_PRIVATE_KEY",
        ));
    };
    let host = host
        .to_str()
        .filter(|s| {
            !s.is_empty()
                && !s.starts_with('-')
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-._@:[]".contains(&c))
        })
        .ok_or_else(|| io::Error::other("SSH 호스트 또는 user@host를 입력해줘."))?;
    let key = fs::canonicalize(key)?;
    if !key.is_file() {
        return Err(io::Error::other("SSH 개인키 경로를 확인해줘."));
    }
    let mut pubfile = key.as_os_str().to_owned();
    pubfile.push(".pub");
    let public = public_key(&fs::read_to_string(pubfile)?)?;
    let account = json_file(&paths.profile())
        .and_then(|v| account_tag(&v))
        .ok_or_else(|| io::Error::other("이 VM의 Claude Code 구독 계정에 먼저 로그인해줘."))?;
    let dir = sender_dir();
    let service = units().join(SERVICE);
    let watch = units().join(WATCH);
    let exe = dir.join("ai-usage-tray");
    let command = unit_path(&exe)?.replace('"', "\\\"");
    let pending = unit_path(&dir.join("pending.json"))?;
    let service_text = format!(
        "{HEADER}[Unit]\nDescription=Send Claude quota to the tray over SSH\nStartLimitIntervalSec=0\n[Service]\nType=oneshot\nExecStart=:\"{command}\" sender-flush\nRestart=on-failure\nRestartSec=5min\n"
    );
    let path_text = format!(
        "{HEADER}[Unit]\nDescription=Watch for pending Claude quota\n[Path]\nPathExists={pending}\nTriggerLimitIntervalSec=0\n[Install]\nWantedBy=paths.target\n"
    );
    fs::create_dir_all(dir.parent().unwrap())?;
    let install_lock = fs::File::open(dir.parent().unwrap())?;
    install_lock.lock()?;
    managed(&watch)?;
    managed(&service)?;
    systemctl(&["show", "--property=Version", "--value"])?;
    match fs::create_dir(&dir) {
        Ok(()) => atomic(&dir.join(MARK), NAME.as_bytes(), 0o600)?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => ensure_owned(&dir)?,
        Err(e) => return Err(e),
    }
    let config = json!({"host":host,"key":key,"account":account});
    let profile_dir = paths
        .configured("CLAUDE_CONFIG_DIR")
        .unwrap_or_else(|| home().join(".claude"));
    let hints = json!({"CLAUDE_CONFIG_DIR":profile_dir});
    if json_file(&dir.join("sender.json")).is_some_and(|old| old != config)
        || json_file(&dir.join("cli-paths.json")).is_some_and(|old| old != hints)
    {
        return Err(io::Error::other(
            "송신 대상, 키, 계정 또는 프로필을 변경하려면 먼저 sender-uninstall을 실행해줘.",
        ));
    }
    atomic(&exe, &fs::read(env::current_exe()?)?, 0o700)?;
    atomic(
        &dir.join("cli-paths.json"),
        &serde_json::to_vec(&hints)?,
        0o600,
    )?;
    atomic(
        &dir.join("sender.json"),
        &serde_json::to_vec(&config)?,
        0o600,
    )?;
    fs::create_dir_all(units())?;
    atomic(&service, service_text.as_bytes(), 0o600)?;
    atomic(&watch, path_text.as_bytes(), 0o600)?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", WATCH])?;
    sender_hook(&dir, true)?;
    println!("{}", json!({"account":account,"publicKey":public}));
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    let dir = sender_dir();
    if !dir.exists() {
        return Ok(());
    }
    let install_lock = fs::File::open(dir.parent().unwrap())?;
    install_lock.lock()?;
    ensure_owned(&dir)?;
    let watch = units().join(WATCH);
    let service = units().join(SERVICE);
    managed(&watch)?;
    managed(&service)?;
    sender_hook(&dir, false)?;
    if watch.exists() {
        systemctl(&["disable", "--now", WATCH])?;
    }
    if service.exists() {
        systemctl(&["stop", SERVICE])?;
    }
    remove_file(&watch)?;
    remove_file(&service)?;
    systemctl(&["daemon-reload"])?;
    let _lock = locked(&dir.join("queue.lock"))?;
    for name in [
        "sender.json",
        "cli-paths.json",
        "pending.json",
        "sent.json",
        "identity.json",
        "queue.lock",
        "send.lock",
        "ai-usage-tray",
        MARK,
    ] {
        remove_file(&dir.join(name))?;
    }
    match fs::remove_dir(&dir) {
        Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        other => other,
    }
}

// Output one authorized_keys entry. The administrator chooses which VM keys to trust.
pub fn receiver_key() -> io::Result<()> {
    let pair = read_input()?;
    let tag = pair["account"]
        .as_str()
        .filter(|s| valid_tag(s))
        .ok_or_else(invalid)?;
    let key = public_key(pair["publicKey"].as_str().ok_or_else(invalid)?)?;
    let dest = destination();
    if crate::update::installed_order(&installed_version(&dest)?) < crate::update::version("0.13.0")
    {
        return Err(io::Error::other(
            "메인 PC에 트레이 0.13.0 이상을 먼저 설치해줘.",
        ));
    }
    let exe = dest.join("ai-usage-tray");
    let cache = cache();
    let cache_home = cache.parent().unwrap();
    if [&exe, cache_home].iter().any(|p| {
        p.to_str()
            .is_none_or(|s| !p.is_absolute() || s.chars().any(char::is_control))
    }) {
        return Err(invalid());
    }
    let command = format!(
        "XDG_CACHE_HOME={} /usr/bin/timeout -k 2s 15s {} receive {tag}",
        shell_quote(cache_home),
        shell_quote(&exe)
    );
    let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
    println!("restrict,command=\"{escaped}\" {key} ai-usage-tray-sender");
    Ok(())
}
