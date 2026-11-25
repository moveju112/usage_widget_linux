use crate::common::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const UUID: &str = "ai-usage-tray@local";
const MARK: &str = ".ai-usage-tray-install";
const SCHEMA: &str = "schemas/org.gnome.shell.extensions.ai-usage-tray.gschema.xml";
const ASSETS: &[(&str, &[u8])] = &[
    ("extension.js", include_bytes!("../extension/extension.js")),
    ("updates.js", include_bytes!("../extension/updates.js")),
    (
        "schedulePrefs.js",
        include_bytes!("../extension/schedulePrefs.js"),
    ),
    (
        "eventPrefs.js",
        include_bytes!("../extension/eventPrefs.js"),
    ),
    ("vmPrefs.js", include_bytes!("../extension/vmPrefs.js")),
    ("prefs.js", include_bytes!("../extension/prefs.js")),
    ("settings.js", include_bytes!("../extension/settings.js")),
    (
        "scheduleWindow.js",
        include_bytes!("../extension/scheduleWindow.js"),
    ),
    ("usageView.js", include_bytes!("../extension/usageView.js")),
    (
        "claude-symbolic.svg",
        include_bytes!("../extension/claude-symbolic.svg"),
    ),
    (
        "codex-symbolic.svg",
        include_bytes!("../extension/codex-symbolic.svg"),
    ),
    ("display.js", include_bytes!("../extension/display.js")),
    (
        "stylesheet.css",
        include_bytes!("../extension/stylesheet.css"),
    ),
    (
        "metadata.json",
        include_bytes!("../extension/metadata.json"),
    ),
    (
        SCHEMA,
        include_bytes!("../extension/schemas/org.gnome.shell.extensions.ai-usage-tray.gschema.xml"),
    ),
];
const EXTRA: &[&str] = &[
    "schemas/gschemas.compiled",
    "cli-paths.json",
    "ai-usage-tray",
];
const SUFFIX: &str = "\n) # ai-usage-tray:previous";

fn bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
pub(crate) fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}
struct Wrapper {
    capture: String,
    prefix: String,
}
enum Original<'a> {
    Absent(&'a str),
    Command(&'a str),
}
impl Wrapper {
    fn new(base: String) -> Self {
        Self {
            capture: format!("{base} capture # ai-usage-tray:"),
            prefix: format!("{base} forward | (\n:\n"),
        }
    }
    fn parse<'a>(&self, command: &'a str) -> Option<Original<'a>> {
        if let Some(inner) = command
            .strip_prefix(&self.prefix)
            .and_then(|s| s.strip_suffix(SUFFIX))
        {
            return Some(Original::Command(inner));
        }
        let flag = command.strip_prefix(&self.capture)?;
        matches!(flag, "created" | "absent" | "null").then_some(Original::Absent(flag))
    }
    fn encode(&self, original: Original) -> String {
        match original {
            Original::Command(c) => format!("{}{c}{SUFFIX}", self.prefix),
            Original::Absent(f) => format!("{}{f}", self.capture),
        }
    }
}
fn wrappers(dest: &Path) -> [Wrapper; 2] {
    [
        Wrapper::new(if dest == crate::relay::sender_dir() {
            format!("{} sender", shell_quote(&dest.join("ai-usage-tray")))
        } else {
            shell_quote(&dest.join("ai-usage-tray"))
        }),
        Wrapper::new(format!(
            "/usr/bin/python3 -B {}",
            shell_quote(&dest.join("collector.py"))
        )),
    ]
}
fn custom_reference(dest: &Path, cmd: &str) -> bool {
    ["collector.py", "ai-usage-tray"].iter().any(|file| {
        cmd.contains(dest.join(file).to_string_lossy().as_ref())
            || cmd.contains(&shell_quote(&dest.join(file)))
    })
}
fn wrap(data: &mut Value, exists: bool, dest: &Path) -> io::Result<bool> {
    let [new, old] = wrappers(dest);
    if data["statusLine"].is_null() {
        let flag = if !exists {
            "created"
        } else if data.get("statusLine").is_some() {
            "null"
        } else {
            "absent"
        };
        data["statusLine"] = json!({"type":"command","command":new.encode(Original::Absent(flag))});
        return Ok(true);
    }
    let sl = &mut data["statusLine"];
    let cmd = sl["command"]
        .as_str()
        .filter(|_| sl["type"] == "command")
        .ok_or_else(|| io::Error::other("기존 statusLine 형식을 보존할 수 없어."))?;
    if new.parse(cmd).is_some()
        || matches!(other_wrapper(dest).parse(cmd), Some(Original::Command(inner)) if new.parse(inner).is_some())
    {
        return Ok(false);
    }
    let original = match old.parse(cmd) {
        Some(original) => original,
        None if custom_reference(dest, cmd) => {
            return Err(io::Error::other(
                "수정된 statusLine이 기존 수집기를 참조하고 있어.",
            ));
        }
        None => Original::Command(cmd),
    };
    sl["command"] = new.encode(original).into();
    Ok(true)
}
fn other_wrapper(dest: &Path) -> Wrapper {
    let other = if dest == crate::relay::sender_dir() {
        destination()
    } else {
        crate::relay::sender_dir()
    };
    wrappers(&other).into_iter().next().unwrap()
}
fn unwrap(data: &mut Value, dest: &Path) -> io::Result<(bool, bool)> {
    let other = other_wrapper(dest);
    if let Some(Original::Command(inner)) = data["statusLine"]["command"]
        .as_str()
        .and_then(|s| other.parse(s))
    {
        let mut nested = data.clone();
        nested["statusLine"]["command"] = inner.into();
        let (changed, remove) = unwrap_direct(&mut nested, dest)?;
        if changed {
            let restored = match nested["statusLine"]["command"].as_str() {
                Some(command) => Original::Command(command),
                None if remove => Original::Absent("created"),
                None if nested.get("statusLine").is_some() => Original::Absent("null"),
                None => Original::Absent("absent"),
            };
            data["statusLine"]["command"] = other.encode(restored).into();
        }
        return Ok((changed, false));
    }
    unwrap_direct(data, dest)
}
fn unwrap_direct(data: &mut Value, dest: &Path) -> io::Result<(bool, bool)> {
    let Some(cmd) = data["statusLine"]["command"].as_str() else {
        return Ok((false, false));
    };
    for w in wrappers(dest) {
        match w.parse(cmd) {
            Some(Original::Command(inner)) => {
                data["statusLine"]["command"] = inner.to_owned().into()
            }
            Some(Original::Absent("null")) => data["statusLine"] = Value::Null,
            Some(Original::Absent(flag)) => {
                let created = flag == "created";
                let object = data.as_object_mut().unwrap();
                object.remove("statusLine");
                return Ok((true, created && object.is_empty()));
            }
            None => continue,
        }
        return Ok((true, false));
    }
    if custom_reference(dest, cmd) {
        return Err(io::Error::other(
            "수정된 statusLine이 수집기를 참조하고 있어.",
        ));
    }
    Ok((false, false))
}
struct Settings {
    path: PathBuf,
    original: Option<Vec<u8>>,
    data: Value,
}
impl Settings {
    fn load(dest: &Path) -> io::Result<Self> {
        let hints = json_file(&dest.join("cli-paths.json")).unwrap_or_default();
        let dir = hints["CLAUDE_CONFIG_DIR"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| env_path("CLAUDE_CONFIG_DIR"))
            .unwrap_or_else(|| home().join(".claude"));
        let path = dir.join("settings.json");
        let original = bytes(&path)?;
        let data = match &original {
            None => json!({}),
            Some(v) => serde_json::from_slice(v)
                .map_err(|_| io::Error::other("Claude 설정 JSON을 읽을 수 없어."))?,
        };
        if !data.is_object() {
            return Err(io::Error::other("Claude 설정이 객체여야 해."));
        }
        Ok(Self {
            path,
            original,
            data,
        })
    }
    fn write(&self, remove: bool) -> io::Result<()> {
        if self.original != bytes(&self.path)? {
            return Err(io::Error::other(
                "Claude 설정이 동시에 변경됐어. 다시 실행해줘.",
            ));
        }
        if remove {
            return remove_file(&self.path);
        }
        fs::create_dir_all(self.path.parent().unwrap())?;
        let target = if self.path.is_symlink() {
            self.path.canonicalize()?
        } else {
            self.path.clone()
        };
        let mode = fs::metadata(&target)
            .map(|s| s.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        let mut payload = serde_json::to_vec_pretty(&self.data)?;
        payload.push(b'\n');
        atomic(&target, &payload, mode)
    }
}

pub(crate) fn sender_hook(dest: &Path, enable: bool) -> io::Result<()> {
    let mut config = Settings::load(dest)?;
    let (changed, remove) = if enable {
        (
            wrap(&mut config.data, config.original.is_some(), dest)?,
            false,
        )
    } else {
        unwrap(&mut config.data, dest)?
    };
    if changed {
        config.write(remove)?;
    }
    Ok(())
}

fn files() -> impl Iterator<Item = &'static str> {
    // Publish the version only after all of its runtime files have been replaced.
    ASSETS
        .iter()
        .map(|(n, _)| *n)
        .filter(|name| *name != "metadata.json")
        .chain(EXTRA.iter().copied())
        .chain(std::iter::once("metadata.json"))
}
fn mode(name: &str) -> u32 {
    if name == "ai-usage-tray" {
        0o700
    } else {
        0o600
    }
}
fn checked(command: &mut Command, message: &str) -> io::Result<()> {
    if command.status()?.success() {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}
fn install(dest: &Path, mut config: Option<Settings>) -> io::Result<bool> {
    let update_only = config.is_none();
    let changed = match &mut config {
        Some(c) => wrap(&mut c.data, c.original.is_some(), dest)?,
        None => false,
    };
    let stage = tempfile::Builder::new()
        .prefix(".ai-usage-tray-")
        .tempdir_in(dest.parent().unwrap())?;
    let candidate = stage.path().join("extension");
    fs::create_dir_all(candidate.join("schemas"))?;
    for (name, content) in ASSETS {
        fs::write(candidate.join(name), content)?;
    }
    checked(
        Command::new("glib-compile-schemas")
            .arg("--strict")
            .arg(candidate.join("schemas")),
        "GNOME 설정 스키마를 컴파일할 수 없어.",
    )?;
    let exe = candidate.join("ai-usage-tray");
    fs::copy(env::current_exe()?, &exe)?;
    fs::set_permissions(exe, fs::Permissions::from_mode(0o700))?;
    if let Some(settings) = &config {
        write_cli_hints(dest, &candidate, settings)?;
    }
    fs::write(candidate.join(MARK), UUID)?;
    let fresh = !dest.exists();
    let mut backups = Vec::new();
    let update = (|| -> io::Result<()> {
        if fresh {
            fs::rename(&candidate, dest)?;
        } else {
            for name in files() {
                if update_only && name == "cli-paths.json" {
                    continue;
                }
                let target = dest.join(name);
                let previous = bytes(&target)?;
                let payload = fs::read(candidate.join(name))?;
                let old_mode = fs::metadata(&target)
                    .ok()
                    .map(|m| m.permissions().mode() & 0o777);
                if previous.as_ref() == Some(&payload) && old_mode == Some(mode(name)) {
                    continue;
                }
                atomic(&target, &payload, mode(name))?;
                backups.push((target, previous, old_mode));
            }
        }
        if changed && let Some(c) = config {
            c.write(false)?;
        }
        Ok(())
    })();
    if let Err(error) = update {
        if fresh && dest.exists() && !candidate.exists() {
            fs::rename(dest, candidate)?;
        }
        for (target, previous, old_mode) in backups.into_iter().rev() {
            match previous {
                Some(data) => atomic(&target, &data, old_mode.unwrap_or(0o600))?,
                None => remove_file(&target)?,
            }
        }
        return Err(error);
    }
    Ok(fresh || changed || !backups.is_empty())
}

fn write_cli_hints(dest: &Path, candidate: &Path, config: &Settings) -> io::Result<()> {
    // Prefer this terminal's paths on upgrade, retaining previously discovered GUI hints.
    let plain = Paths { hints: Value::Null };
    let prior = Paths {
        hints: json_file(&dest.join("cli-paths.json")).unwrap_or_default(),
    };
    let mut hints = json!({"CLAUDE_CONFIG_DIR":config.path.parent()});
    for name in ["codex", "claude", "node"] {
        if let Some(p) = plain.find_cli(name).or_else(|| prior.find_cli(name)) {
            hints[name] = json!(p);
        }
    }
    if let Some(p) = prior.configured("CODEX_HOME") {
        hints["CODEX_HOME"] = json!(p);
    }
    fs::write(
        candidate.join("cli-paths.json"),
        serde_json::to_vec(&hints)?,
    )?;
    Ok(())
}

fn known() -> io::Result<bool> {
    let owner = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            "org.gnome.Shell",
        ])
        .output()?;
    if !owner.status.success() || !String::from_utf8_lossy(&owner.stdout).contains("true") {
        return Ok(false);
    }

    Ok(Command::new("gnome-extensions")
        .args(["info", UUID])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success())
}
// GJS is part of GNOME Shell. Use its native string-vector API to preserve unrelated UUIDs.
fn next_login(enable: bool) -> io::Result<()> {
    const UPDATE: &str = r#"
const Gio = imports.gi.Gio;
const [uuid, action] = ARGV;
const settings = new Gio.Settings({schema_id:'org.gnome.shell'});
if (action === 'enable' && settings.get_boolean('disable-user-extensions'))
    throw Error('User extensions are disabled in GNOME.');
for (const key of ['disabled-extensions','enabled-extensions']) {
    const old = settings.get_strv(key);
    const next = old.filter(value => value !== uuid);
    if (action === 'enable' && key === 'enabled-extensions') next.push(uuid);
    if (JSON.stringify(next) !== JSON.stringify(old) && !settings.set_strv(key,next))
        throw Error('Could not update GNOME extension settings.');
}
Gio.Settings.sync();
"#;
    checked(
        Command::new("gjs").args([
            "-c",
            UPDATE,
            UUID,
            if enable { "enable" } else { "disable" },
        ]),
        "GNOME 확장 설정을 변경할 수 없어.",
    )
}
fn start(dest: &Path) -> io::Result<()> {
    if !dest.is_dir() {
        return Err(io::Error::other("먼저 ai-usage-tray install 을 실행해줘."));
    }
    if known()? {
        checked(
            Command::new("gnome-extensions").args(["enable", UUID]),
            "확장을 활성화할 수 없어.",
        )?;
        let info = Command::new("gnome-extensions")
            .env("LC_ALL", "C")
            .args(["info", UUID])
            .output()?;
        let live_info = String::from_utf8_lossy(&info.stdout);
        if !info.status.success() || !live_info.lines().any(|s| s.trim() == "State: ACTIVE") {
            return Err(io::Error::other(
                "GNOME에서 활성 상태를 확인할 수 없어. gnome-extensions info ai-usage-tray@local 로 확인해줘.",
            ));
        }
        let live_version = live_info.lines().find_map(|line| {
            let field = line.trim().strip_prefix("Version: ")?;
            // GNOME prints either "4" or "0.1.0 (4)".
            let number = field
                .rsplit_once(" (")
                .and_then(|(_, tail)| tail.strip_suffix(')'))
                .unwrap_or(field);
            number.parse::<u64>().ok()
        });
        let installed_version =
            json_file(&dest.join("metadata.json")).and_then(|data| data["version"].as_u64());
        if matches!((live_version, installed_version), (Some(a), Some(b)) if a != b) {
            println!(
                "현재 세션은 이전 AI Usage Tray를 실행 중이야. 새 버전은 다음 GNOME 로그인부터 적용돼."
            );
        } else {
            println!("GNOME 패널에서 AI Usage Tray를 활성화했어.");
        }
    } else {
        next_login(true)?;
        println!("GNOME에서 로그아웃 후 다시 로그인하면 활성화돼.");
    }
    Ok(())
}
fn uninstall(dest: &Path, mut config: Settings) -> io::Result<()> {
    let (changed, remove) = unwrap(&mut config.data, dest)?;
    if dest.exists() {
        if known()? {
            checked(
                Command::new("gnome-extensions").args(["disable", UUID]),
                "확장을 비활성화할 수 없어.",
            )?;
        }
        next_login(false)?;
    }
    if changed {
        config.write(remove)?;
    }
    if dest.exists() {
        // An upgraded, already running GNOME session can still have the old JS loaded.
        // Keep its helper until uninstall; never remove a user-created file of the same name.
        let legacy = dest.join("collector.py");
        if let Some(old) = bytes(&legacy)?
            && format!("{:x}", Sha256::digest(old))
                == "7913b67fe034ebde555b6f7f63ffa62c4288e2f3f056f1f5f5a7e070ab3a2b58"
        {
            remove_file(&legacy)?;
        }
        for name in files().chain([MARK]) {
            remove_file(&dest.join(name))?;
        }
        let _ = fs::remove_dir(dest.join("schemas"));
        let _ = fs::remove_dir(dest);
    }
    let state = cache();
    if state.is_dir() {
        remove_file(&state.join("claude.json"))?;
        remove_file(&state.join("capture.lock"))?;
        remove_file(&state.join("claude-remote.json"))?;
        remove_file(&state.join("receive.lock"))?;
        let _ = fs::remove_dir(state);
    }
    let config = crate::schedule::config_dir();
    remove_file(&config.join(crate::claude::events::SETTING))?;
    let state = crate::schedule::state_dir();
    for provider in crate::schedule::Provider::ALL {
        remove_file(&config.join(provider.config_file()))?;
        let record = state.join(provider.record_file());
        remove_file(&record)?;
        remove_file(&record.with_extension("lock"))?;
    }
    let _ = fs::remove_dir(config);
    let _ = fs::remove_dir(state);
    println!("제거 완료");
    Ok(())
}
pub fn destination() -> PathBuf {
    env_path("XDG_DATA_HOME")
        .unwrap_or_else(|| home().join(".local/share"))
        .join("gnome-shell/extensions")
        .join(UUID)
}

pub fn installed_version(dest: &Path) -> io::Result<String> {
    if dest.is_symlink() || bytes(&dest.join(MARK))?.as_deref() != Some(UUID.as_bytes()) {
        return Err(io::Error::other(
            "AI Usage Tray 설치를 확인할 수 없어. 먼저 install 을 실행해줘.",
        ));
    }
    let metadata = json_file(&dest.join("metadata.json")).unwrap_or_default();
    match metadata.get("version-name") {
        Some(Value::String(version)) if crate::update::version(version).is_some() => {
            Ok(version.clone())
        }
        None if metadata["uuid"] == UUID && matches!(metadata["version"].as_u64(), Some(1 | 2)) => {
            Ok("0.0.0".into())
        }
        _ => Err(io::Error::other("설치 버전을 확인할 수 없어.")),
    }
}

// The new binary checks the installed version under the same lock as install/uninstall.
pub fn apply_update(expected: &str) -> io::Result<()> {
    let candidate = env!("CARGO_PKG_VERSION");
    if candidate != expected {
        return Err(io::Error::other("릴리스 태그와 실행 파일 버전이 달라."));
    }
    let dest = destination();
    let lock = fs::File::open(dest.parent().unwrap())?;
    lock.lock()?;
    let installed = installed_version(&dest)?;
    if crate::update::installed_order(&installed) >= crate::update::version(candidate) {
        println!("current {installed}");
    } else {
        install(&dest, None)?;
        println!("installed {candidate}");
    }
    Ok(())
}

pub fn run(action: &str) -> io::Result<()> {
    let dest = destination();
    let root = dest.parent().unwrap();
    if action != "start" {
        fs::create_dir_all(root)?;
    }
    let lock = fs::File::open(root).map_err(|e| {
        if action == "start" && e.kind() == io::ErrorKind::NotFound {
            io::Error::other("먼저 ai-usage-tray install 을 실행해줘.")
        } else {
            e
        }
    })?;
    lock.lock()?;
    if dest.is_symlink()
        || dest.exists() && bytes(&dest.join(MARK))?.as_deref() != Some(UUID.as_bytes())
    {
        return Err(io::Error::other("설치 위치에 다른 파일이 있어."));
    }
    match action {
        "install" => {
            let changed = install(&dest, Some(Settings::load(&dest)?))?;
            println!(
                "{}",
                if changed {
                    "설치 완료"
                } else {
                    "이미 설치되어 있어"
                }
            );
            Ok(())
        }
        "uninstall" => uninstall(&dest, Settings::load(&dest)?),
        _ => start(&dest),
    }
}
