use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_ai-usage-tray");
struct User(TempDir);
impl User {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let user = Self(tmp);
        for name in ["bin", "home/.claude"] {
            fs::create_dir_all(user.path(name)).unwrap();
        }
        user.fake("claude", "exit 0");
        user
    }
    fn path(&self, s: &str) -> PathBuf {
        self.0.path().join(s)
    }
    fn fake(&self, name: &str, body: &str) {
        let path = self.path("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn command(&self, prog: impl AsRef<Path>) -> Command {
        let mut cmd = Command::new(prog.as_ref());
        cmd.env_clear()
            .env("HOME", self.path("home"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_CACHE_HOME", self.path("cache"))
            .env("GSETTINGS_BACKEND", "memory")
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.path("bin").display()),
            );
        cmd
    }
    fn run(&self, action: &str) -> Output {
        let o = self.command(BIN).arg(action).output().unwrap();
        assert!(
            o.status.success(),
            "{}: {}",
            action,
            String::from_utf8_lossy(&o.stderr)
        );
        o
    }
    fn state(&self) -> Value {
        serde_json::from_slice(&self.run("once").stdout).unwrap()
    }
    fn capture(&self, data: &[u8]) {
        let o = input(self.command(BIN).arg("capture"), data);
        assert!(o.status.success() && o.stderr.is_empty());
    }
    fn cfg(&self) -> PathBuf {
        self.path("home/.claude/settings.json")
    }
    fn dest(&self) -> PathBuf {
        self.path("data/gnome-shell/extensions/ai-usage-tray@local")
    }
    fn server(&self, reply: Value, account: Option<Value>) {
        let check = |method: &str| {
            format!(
                "IFS= read -r msg || exit 2; case \"$msg\" in *'\"method\":\"{method}\"'*) ;; *) exit 3;; esac\n"
            )
        };
        let mut script = check("initialize");
        script.push_str("echo '{\"id\":1,\"result\":{}}'\n");
        script += &check("initialized");
        script += &check("account/rateLimits/read");
        script.push_str("echo '{\"method\":\"account/updated\"}'\n");
        let mut packet = reply;
        packet["id"] = 2.into();
        script += &format!("echo '{packet}'\n");
        if let Some(result) = account {
            script += &check("account/read");
            script += &format!("echo '{}'\n", json!({"id":3,"result":result}));
        }
        self.fake("codex", &script);
    }
}
fn input(cmd: &mut Command, raw: &[u8]) -> Output {
    let mut p = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(raw).unwrap();
    p.wait_with_output().unwrap()
}
fn load(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
#[test]
fn source_privacy_and_account_switch() {
    let u = User::new();
    assert_eq!(u.state()["claude"]["status"], "waiting");
    u.capture(br#"{"context_window":{"used_percentage":99,"current_usage":{}}}"#);
    assert_eq!(u.state()["claude"]["status"], "unavailable");
    u.capture(br#"{"context_window":{"used_percentage":87},"transcript_path":"EXCLUDED","rate_limits":{"five_hour":{"used_percentage":0,"resets_at":2000000000}}}"#);
    let v = u.state();
    assert_eq!(v["claude"]["windows"][0]["percent"], 0);
    assert_eq!(v["codex"]["status"], "missing");
    let saved = u.path("cache/ai-usage-tray/claude.json");
    assert_eq!(
        fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let raw = fs::read_to_string(saved).unwrap();
    for s in [
        "context_window",
        "EXCLUDED",
        "rate_limits",
        "transcript_path",
    ] {
        assert!(!raw.contains(s));
    }
    u.capture(b"{}");
    assert_eq!(v["claude"], u.state()["claude"]);
    let profile = u.path("home/.claude.json");
    let fetched = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 1900;
    let mut data = json!({"oauthAccount":{"accountUuid":"sample"},"cachedUsageUtilization":{"accountUuid":"sample","fetchedAtMs":fetched*1000,"utilization":{"seven_day":{"utilization":12,"resets_at":"2030-01-02T00:00:00Z"}}}});
    fs::write(&profile, data.to_string()).unwrap();
    let cached = u.state()["claude"].clone();
    assert_eq!(cached["windows"][0]["resets"].as_f64(), Some(1893542400.));
    assert_eq!(cached["received"].as_f64(), Some(fetched as f64));
    assert_eq!(cached, u.state()["claude"]);
    data["oauthAccount"]["accountUuid"] = "another".into();
    fs::write(&profile, data.to_string()).unwrap();
    assert_eq!(u.state()["claude"]["status"], "waiting");
}
#[test]
fn codex_actual_periods_and_errors() {
    let u = User::new();
    let week = json!({"usedPercent":17,"windowDurationMins":10080,"resetsAt":2000000000});
    for (secondary, durations) in [
        (Value::Null, vec![10080]),
        (
            json!({"usedPercent":0,"windowDurationMins":300}),
            vec![300, 10080],
        ),
        (
            json!({"usedPercent":1,"windowDurationMins":15}),
            vec![15, 10080],
        ),
    ] {
        u.server(json!({"result":{"rateLimitsByLimitId":{"codex":{"primary":week,"secondary":secondary}}}}),None);
        let state = u.state();
        assert_eq!(state["codex"]["status"], "ok");
        assert_eq!(
            state["codex"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w["minutes"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            durations
        );
    }
    for (reply, account, status) in [
        (json!({"error":{"message":"unauthorized"}}), None, "login"),
        (
            json!({"error":{"message":"failed"}}),
            Some(json!({"account":null,"requiresOpenaiAuth":true})),
            "login",
        ),
        (
            json!({"error":{"message":"failed"}}),
            Some(json!({"account":{"type":"apiKey"}})),
            "unavailable",
        ),
        (json!({"result":{"rateLimits":{}}}), None, "unavailable"),
    ] {
        u.server(reply, account);
        assert_eq!(u.state()["codex"]["status"], status);
    }
}
#[test]
fn legacy_upgrade_passthrough_and_restore() {
    let u = User::new();
    let old = "cat; echo diagnostic >&2; exit 7";
    let orig = json!({"type":"command","command":old,"padding":3});
    let base = json!({"statusLine":orig,"unrelated":"keep"});
    fs::write(u.cfg(), base.to_string()).unwrap();
    u.run("install");
    let dest = u.dest();
    let mut data = load(&u.cfg());
    data["statusLine"]["command"] = format!(
        "/usr/bin/python3 -B {} forward | (\n:\n{old}\n) # ai-usage-tray:previous",
        dest.join("collector.py").display()
    )
    .into();
    fs::write(u.cfg(), data.to_string()).unwrap();
    u.run("install");
    let wrapped = fs::read(u.cfg()).unwrap();
    u.run("install");
    assert_eq!(wrapped, fs::read(u.cfg()).unwrap());
    let mut data = load(&u.cfg());
    let cmd = data["statusLine"]["command"].as_str().unwrap();
    assert!(!cmd.contains("python3"));
    let raw = b" { \"rate_limits\":{\"five_hour\":{\"used_percentage\":32}} } \n\n";
    let result = input(u.command("/bin/sh").args(["-c", cmd]), raw);
    assert_eq!(result.stdout, raw);
    assert_eq!(result.stderr, b"diagnostic\n");
    assert_eq!(result.status.code(), Some(7));
    let gui = u
        .command(dest.join("ai-usage-tray"))
        .arg("once")
        .env("PATH", "/usr/bin")
        .output()
        .unwrap();
    let state: Value = serde_json::from_slice(&gui.stdout).unwrap();
    assert_eq!(state["claude"]["windows"][0]["percent"], 32);
    data["later"] = true.into();
    fs::write(u.cfg(), data.to_string()).unwrap();
    fs::write(dest.join("personal"), b"keep").unwrap();
    u.run("uninstall");
    let mut expected = base;
    expected["later"] = true.into();
    assert_eq!(load(&u.cfg()), expected);
    assert_eq!(fs::read(dest.join("personal")).unwrap(), b"keep");
}
#[test]
fn settings_origins_and_retries() {
    let u = User::new();
    for initial in [
        None,
        Some(json!({"x":1})),
        Some(json!({"statusLine":null,"x":1})),
    ] {
        if let Some(data) = &initial {
            fs::write(u.cfg(), data.to_string()).unwrap();
        }
        u.run("install");
        u.run("uninstall");
        u.run("uninstall");
        assert_eq!(u.cfg().exists().then(|| load(&u.cfg())), initial);
    }
    fs::write(u.cfg(), br#"{"statusLine":{"type":"custom"}}"#).unwrap();
    assert!(!u.command(BIN).arg("install").status().unwrap().success());
    assert!(!u.dest().exists());
}
#[test]
fn slow_query_and_manual_refresh() {
    let u = User::new();
    u.fake("codex", "exec sleep 60");
    let mut p = u
        .command(BIN)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = p.stdin.take().unwrap();
    let out = p.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx
                .send(serde_json::from_str::<Value>(&line).unwrap())
                .is_err()
            {
                break;
            }
        }
    });
    let receive = || rx.recv_timeout(Duration::from_secs(4)).unwrap();
    assert_eq!(receive()["codex"]["status"], "loading");
    u.capture(br#"{"rate_limits":{"five_hour":{"used_percentage":0}}}"#);
    pipe.write_all(b"refresh\n").unwrap();
    let first = receive();
    assert_eq!(first["claude"]["windows"][0]["percent"], 0);
    pipe.write_all(b"refresh\n").unwrap();
    let second = receive();
    assert_eq!(second["notice"], "다음 Claude Code 응답 대기");
    assert_eq!(first["claude"]["received"], second["claude"]["received"]);
    let start = Instant::now();
    drop(pipe);
    assert!(p.wait().unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(3));
}
#[test]
fn timeout_is_independent() {
    let u = User::new();
    u.fake("codex", "exec sleep 60");
    u.capture(br#"{"rate_limits":{"seven_day":{"used_percentage":17}}}"#);
    let state = u.state();
    assert_eq!(state["codex"]["status"], "timeout");
    assert_eq!(state["claude"]["windows"][0]["percent"], 17);
}

#[test]
fn github_release_update_and_recovery() {
    use sha2::{Digest, Sha256};
    let u = User::new();
    let version = env!("CARGO_PKG_VERSION");
    let asset_name = format!("ai-usage-tray-linux-{}", std::env::consts::ARCH);
    let binary = fs::read(BIN).unwrap();
    let digest = format!("sha256:{:x}", Sha256::digest(&binary));
    let metadata = u.dest().join("metadata.json");
    let release = json!({
        "tag_name":format!("v{version}"), "draft":false, "prerelease":false,
        "assets":[{"name":asset_name, "state":"uploaded", "size":binary.len(),
            "digest":digest,
            "browser_download_url":format!("https://github.com/moveju112/usage_widget_linux/releases/download/v{version}/{asset_name}")}]
    });
    let publish = |v: &Value| fs::write(u.path("index"), serde_json::to_vec(v).unwrap()).unwrap();
    let invoke = |action: &str| u.command(BIN).args([action, "--json"]).output().unwrap();
    let status = |action: &str| {
        let result = invoke(action);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice::<Value>(&result.stdout).unwrap()
    };
    u.fake(
        "curl",
        r#"
        while [ "$1" != "--output" ]; do shift; done
        case "$3" in
            https://api.github.com/repos/moveju112/usage_widget_linux/releases/latest)
                if test -e "$HOME/../timeout"; then exit 28; fi
                if ! test -f "$HOME/../index"; then printf '404'; exit; fi
                cp "$HOME/../index" "$2";;
            https://github.com/moveju112/usage_widget_linux/releases/download/*)
                cp "$HOME/../asset" "$2" || exit 1;;
            *) exit 1;;
        esac
        printf '200'
    "#,
    );
    assert!(!invoke("update").status.success());
    u.run("install");
    fs::write(&metadata, br#"{"uuid":"ai-usage-tray@local","version":2}"#).unwrap();
    assert_eq!(status("update")["state"], "unpublished");
    fs::write(u.path("timeout"), "").unwrap();
    assert!(String::from_utf8_lossy(&invoke("check-update").stderr).contains("초과"));
    fs::remove_file(u.path("timeout")).unwrap();

    publish(&release);
    assert_eq!(
        status("check-update"),
        json!({"state":"available","version":version})
    );
    fs::write(u.path("asset"), b"invalid").unwrap();
    assert!(String::from_utf8_lossy(&invoke("update").stderr).contains("무결성"));
    fs::write(u.path("asset"), &binary).unwrap();
    for (k, v) in [
        ("digest", json!(format!("sha256:{}", "0".repeat(64)))),
        ("digest", Value::Null),
        ("name", json!("ai-usage-tray-other")),
        (
            "browser_download_url",
            json!("https://example.invalid/file"),
        ),
    ] {
        let mut bad = release.clone();
        bad["assets"][0][k] = v;
        publish(&bad);
        assert!(!invoke("update").status.success());
        assert_eq!(load(&metadata)["version"], 2);
    }
    let mut preview = release.clone();
    preview["prerelease"] = true.into();
    publish(&preview);
    assert!(!invoke("check-update").status.success());

    // The release upgrade must never repair or rewrite the user's CLI settings.
    fs::write(
        u.cfg(),
        br#"{"statusLine":{"type":"command","command":"user-customized"}}"#,
    )
    .unwrap();
    let settings = fs::read(u.cfg()).unwrap();
    let hints = fs::read(u.dest().join("cli-paths.json")).unwrap();
    let extension = u.dest().join("extension.js");
    fs::write(&extension, b"prior extension").unwrap();
    let blocked_file = u.dest().join("updates.js");
    fs::remove_file(&blocked_file).unwrap();
    fs::create_dir(&blocked_file).unwrap();
    publish(&release);
    assert!(!invoke("update").status.success());
    assert_eq!(load(&metadata)["version"], 2);
    assert_eq!(fs::read(&extension).unwrap(), b"prior extension");
    fs::remove_dir(blocked_file).unwrap();

    // Two concurrent installs converge, including the candidate's check under lock.
    let (left, right) = thread::scope(|scope| {
        let left = scope.spawn(|| status("update"));
        (status("update"), left.join().unwrap())
    });
    let mut states = [
        left["state"].as_str().unwrap(),
        right["state"].as_str().unwrap(),
    ];
    states.sort_unstable();
    assert_eq!(states, ["current", "installed"]);
    assert_eq!(load(&metadata)["version-name"], version);
    assert_eq!(fs::read(u.cfg()).unwrap(), settings);
    assert_eq!(fs::read(u.dest().join("cli-paths.json")).unwrap(), hints);
    assert_eq!(status("update")["state"], "current");
    let mut newer_installed = load(&metadata);
    newer_installed["version-name"] = "20.0.0".into();
    fs::write(&metadata, newer_installed.to_string()).unwrap();
    assert_eq!(status("update")["version"], "20.0.0");
    assert!(
        !u.command(BIN)
            .args(["apply-update", "99.1.0"])
            .status()
            .unwrap()
            .success()
    );
    u.run("uninstall");
    assert!(
        !u.command(BIN)
            .args(["apply-update", version])
            .status()
            .unwrap()
            .success()
    );
    assert!(!u.dest().exists());
}
