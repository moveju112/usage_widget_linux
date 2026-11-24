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
    let receive = |ready: fn(&Value) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let state = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if ready(&state) {
                return state;
            }
        }
    };
    assert_eq!(receive(|_| true)["codex"]["status"], "loading");
    u.capture(br#"{"rate_limits":{"five_hour":{"used_percentage":0}}}"#);
    pipe.write_all(b"refresh\n").unwrap();
    let first = receive(|state| state["claude"]["windows"][0]["percent"] == 0);
    assert_eq!(first["codex"]["status"], "loading", "{first}");
    pipe.write_all(b"refresh\n").unwrap();
    let second = receive(|state| state["notice"] == "다음 Claude Code 응답 대기");
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
    let mut legacy_release = release.clone();
    legacy_release["tag_name"] = "v1.1.0".into();
    legacy_release["assets"] = json!([]);
    publish(&legacy_release);
    for action in ["check-update", "update"] {
        assert_eq!(
            status(action),
            json!({"state":"unpublished", "version":version})
        );
    }
    assert_eq!(load(&metadata)["version-name"], version);

    let next_minor = env!("CARGO_PKG_VERSION_MINOR").parse::<u64>().unwrap() + 1;
    let next = format!("{}.{next_minor}.0", env!("CARGO_PKG_VERSION_MAJOR"));
    let mut next_release = release.clone();
    next_release["tag_name"] = format!("v{next}").into();
    next_release["assets"][0]["browser_download_url"] = format!(
        "https://github.com/moveju112/usage_widget_linux/releases/download/v{next}/{asset_name}"
    )
    .into();
    publish(&next_release);
    assert_eq!(
        status("check-update"),
        json!({"state":"available", "version":next})
    );

    publish(&release);
    let mut legacy_installed = load(&metadata);
    legacy_installed["version-name"] = "1.1.0".into();
    legacy_installed["version"] = 3.into();
    fs::write(&metadata, legacy_installed.to_string()).unwrap();
    assert_eq!(
        status("update"),
        json!({"state":"installed", "version":version})
    );
    assert_eq!(load(&metadata)["version-name"], version);
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

// A disposable app-server reports a ChatGPT account and records only turn/start.
const SCHEDULE_SERVER: &str = r#"
while IFS= read -r message; do
    case "$message" in
        *'"method":"initialize"'*) echo '{"id":1,"result":{}}' ;;
        *'"method":"account/read"'*)
            account=chatgpt
            if test "$SCENARIO" = api; then account=apiKey; fi
            echo "{\"id\":2,\"result\":{\"account\":{\"type\":\"$account\"}}}" ;;
        *'"method":"account/rateLimits/read"'*)
            id=2
            case "$message" in *'"id":5'*) id=5 ;; esac
            used=5
            if test "$SCENARIO" = limited; then used=100; fi
            echo "{\"id\":$id,\"result\":{\"rateLimits\":{\"secondary\":{\"usedPercent\":$used,\"windowDurationMins\":10080,\"resetsAt\":2200000000}}}}" ;;
        *'"method":"model/list"'*)
            if test "$SCENARIO" = no_model; then echo '{"id":6,"result":{"data":[]}}'
            elif test "$SCENARIO" != series; then exit 3
            else
                case "$message" in
                    *'"cursor":"page2"'*) echo '{"id":6,"result":{"data":[{"model":"gpt-5.10-luna"},{"model":"gpt-9-luna","hidden":true}]}}' ;;
                    *) echo '{"id":6,"result":{"data":[{"model":"gpt-5.9-luna"},{"model":"gpt-10-sol"}],"nextCursor":"page2"}}' ;;
                esac
            fi ;;
        *'"method":"thread/start"'*)
            case "$message" in *'"ephemeral":true'*) ;; *) exit 2 ;; esac
            case "$message" in *'"sandbox":"read-only"'*) ;; *) exit 2 ;; esac
            model=chosen-model
            if test "$SCENARIO" = series; then model=gpt-5.10-luna; fi
            case "$message" in *"\"model\":\"$model\""*) ;; *) exit 2 ;; esac
            if test "$SCENARIO" = changed; then
                echo '{"enabled":false}' > "$HOME/.config/ai-usage-tray/schedule.json"
            fi
            echo "{\"id\":3,\"result\":{\"thread\":{\"id\":\"thread\"},\"model\":\"$model\",\"modelProvider\":\"openai\"}}" ;;
        *'"method":"turn/start"'*)
            echo sent >> "$HOME/attempts"
            echo '{"id":4,"result":{"turn":{"id":"turn"}}}'
            status=completed
            if test "$SCENARIO" = failure; then status=failed; fi
            echo "{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread\",\"turn\":{\"id\":\"turn\",\"status\":\"$status\"}}}" ;;
    esac
done
"#;

fn scheduled_user(start: &str, end: &str) -> (User, String) {
    use chrono::{DateTime, Timelike, Utc};
    use sha2::{Digest, Sha256};
    let u = User::new();
    u.fake("codex", SCHEDULE_SERVER);
    let time: DateTime<Utc> = SystemTime::now().into();
    // Choose a timezone in which the test always starts around noon.
    let tz = format!("UTC{}", time.hour() as i32 - 12);
    let date = time.date_naive();
    let seed = (0u32..)
        .find(|n| {
            let hash = Sha256::digest(format!("{n}:{date}"));
            u64::from_be_bytes(hash[..8].try_into().unwrap()) % 1440 < 600
        })
        .unwrap()
        .to_string();
    let dir = u.path("home/.config/ai-usage-tray");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("schedule.json"),
        serde_json::to_vec(&json!({
            "enabled":true,"start":start,"end":end,"seed":seed,"model":"chosen-model"
        }))
        .unwrap(),
    )
    .unwrap();
    (u, tz)
}
fn scheduled_worker(
    u: &User,
    tz: &str,
    scenario: &str,
) -> (std::process::Child, mpsc::Receiver<Value>) {
    let installed = u.dest().join("ai-usage-tray");
    let mut child = u
        .command(if installed.exists() {
            &installed
        } else {
            Path::new(BIN)
        })
        .arg("serve")
        .env("TZ", tz)
        .env("SCENARIO", scenario)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.stdout.take().unwrap();
    let (send, recv) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if send.send(serde_json::from_str(&line).unwrap()).is_err() {
                break;
            }
        }
    });
    (child, recv)
}
fn wait_schedule(
    recv: &mpsc::Receiver<Value>,
    key: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let state = recv
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if predicate(&state[key]) {
            return state[key].clone();
        }
    }
}
#[test]
fn schedule_claim_survives_races_failures_and_restarts() {
    for (scenario, expected, turns) in [
        ("success", "ok", 1),
        ("failure", "error", 1),
        ("limited", "limited", 0),
        ("api", "login", 0),
        ("changed", "changed", 0),
    ] {
        let (u, tz) = scheduled_user("00:00", "23:59");
        let (mut first, rx) = scheduled_worker(&u, &tz, scenario);
        let (mut other, rx2) = scheduled_worker(&u, &tz, scenario);
        // Either process can win the lock. A changed config is also observed as off.
        if scenario == "changed" {
            let until = Instant::now() + Duration::from_secs(5);
            let record = u.path("home/.local/state/ai-usage-tray/scheduled-call.json");
            while json_record_status(&record).as_deref() != Some(expected) {
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(20));
            }
        } else {
            wait_schedule(&rx, "schedule", |s| s["last"]["status"] == expected);
            wait_schedule(&rx2, "schedule", |s| s["last"]["status"] == expected);
        }
        first.stdin.take();
        other.stdin.take();
        assert!(first.wait().unwrap().success() && other.wait().unwrap().success());
        if scenario != "changed" {
            let (mut again, rx) = scheduled_worker(&u, &tz, scenario);
            wait_schedule(&rx, "schedule", |s| {
                s["last"]["status"] == expected
                    && s["next"]
                        .as_f64()
                        .is_some_and(|t| t > s["last"]["time"].as_f64().unwrap())
            });
            again.stdin.take();
            assert!(again.wait().unwrap().success());
        }
        assert_eq!(
            fs::read_to_string(u.path("home/attempts"))
                .unwrap_or_default()
                .lines()
                .count(),
            turns
        );
    }
}
fn json_record_status(path: &Path) -> Option<String> {
    serde_json::from_slice::<Value>(&fs::read(path).ok()?).ok()?["status"]
        .as_str()
        .map(str::to_owned)
}
#[test]
fn missed_window_and_unreadable_record_do_not_send() {
    let (u, tz) = scheduled_user("03:10", "03:10");
    let (mut child, rx) = scheduled_worker(&u, &tz, "success");
    let state = wait_schedule(&rx, "schedule", |s| s["next"].is_number());
    assert!(
        state["next"].as_u64().unwrap()
            > SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
    );
    let dir = u.path("home/.local/state/ai-usage-tray");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("scheduled-call.json"), b"null").unwrap();
    wait_schedule(&rx, "schedule", |s| s["status"] == "error");
    child.stdin.take();
    assert!(child.wait().unwrap().success());
    assert!(!u.path("home/attempts").exists());
}

const SCHEDULE_CLAUDE: &str = r#"
test -z "$CLAUDE_CONFIG_DIR" || exit 10
case " $* " in
    *' auth status --json '*)
        if test "$SCENARIO" = claude_key; then
            echo '{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}'
        else
            echo '{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}'
        fi
        if test "$SCENARIO" = claude_changed; then
            echo '{"enabled":false}' > "$HOME/.config/ai-usage-tray/claude-schedule.json"
        fi
        ;;
    *' --input-format stream-json '*)
        IFS= read -r request
        case "$request" in *'"subtype":"initialize"'*) ;; *) exit 11;; esac
        models='[{"value":"haiku","displayName":"Haiku 5.9"},{"value":"claude-haiku-5-10","displayName":"Haiku 5.10"},{"value":"claude-sonnet-9","displayName":"Sonnet 9"}]'
        if test "$SCENARIO" = no_model; then models='[]'; fi
        echo "{\"type\":\"control_response\",\"response\":{\"request_id\":\"catalog\",\"response\":{\"models\":$models}}}" ;;
    *' --print '*)
        echo called >> "$HOME/claude-attempts"
        test ! -e "$PWD/Cargo.toml" || exit 4
        case " $* " in *' --safe-mode --restricted --strict-mcp-config '*) ;; *) exit 5;; esac
        model='sonnet[1m]'
        if test "$SCENARIO" = series; then model=claude-haiku-5-10; fi
        case " $* " in *" --model $model --max-turns 1 --tools  --permission-mode dontAsk --no-session-persistence "*) ;; *) exit 6;; esac
        case " $* " in *' -- Reply with OK only. ') ;; *) exit 7;; esac
        if test "$SCENARIO" = claude_wait; then exec sleep 60; fi
        if test "$SCENARIO" = claude_error; then
            echo '{"type":"result","subtype":"success","is_error":true}'
            exit 1
        fi
        echo '{"type":"result","subtype":"success","is_error":false,"result":"PRIVATE-RESPONSE"}'
        ;;
    *) exit 3;;
esac
"#;

#[test]
fn independent_claude_and_codex_reservations() {
    for (scenario, expected, calls) in [
        ("success", "ok", 1),
        ("claude_key", "login", 0),
        ("claude_changed", "changed", 0),
        ("claude_limited", "limited", 0),
        ("claude_expired_limit", "ok", 1),
        ("claude_error", "error", 1),
        ("claude_wait", "timeout", 1),
        ("claude_off", "off", 0),
    ] {
        let (u, tz) = scheduled_user("00:00", "23:59");
        u.fake("claude", SCHEDULE_CLAUDE);
        u.run("install");
        let config = u.path("home/.config/ai-usage-tray");
        let mut claude = load(&config.join("schedule.json"));
        claude["model"] = "sonnet[1m]".into();
        claude["termsConsentVersion"] = 1.into();
        if scenario != "claude_off" {
            fs::write(config.join("claude-schedule.json"), claude.to_string()).unwrap();
        }
        if matches!(scenario, "claude_limited" | "claude_expired_limit") {
            let reset = if scenario == "claude_limited" {
                4_000_000_000_i64
            } else {
                1
            };
            let usage =
                json!({"rate_limits":{"five_hour":{"used_percentage":100,"resets_at":reset}}});
            u.capture(usage.to_string().as_bytes());
        }
        let state_dir = u.path("home/.local/state/ai-usage-tray");
        let record = state_dir.join("claude-scheduled-call.json");
        // Watch the real workers concurrently. Codex must complete even if Claude hangs.
        let (mut first, rx) = scheduled_worker(&u, &tz, scenario);
        let (mut second, _second_updates) = scheduled_worker(&u, &tz, scenario);
        wait_schedule(&rx, "schedule", |s| s["last"]["status"] == "ok");
        let until = Instant::now() + Duration::from_secs(5);
        if scenario == "claude_wait" {
            while !u.path("home/claude-attempts").exists() {
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(10));
            }
        } else if scenario != "claude_off" {
            while json_record_status(&record).as_deref() != Some(expected) {
                assert!(Instant::now() < until, "Claude scenario {scenario}");
                thread::sleep(Duration::from_millis(10));
            }
        }
        first.stdin.take();
        second.stdin.take();
        assert!(first.wait().unwrap().success() && second.wait().unwrap().success());
        assert_eq!(
            fs::read_to_string(u.path("home/attempts"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        if scenario == "claude_off" {
            assert!(!record.exists());
        } else {
            assert_eq!(load(&record)["status"], expected);
            assert!(
                !fs::read_to_string(&record)
                    .unwrap()
                    .contains("PRIVATE-RESPONSE")
            );
            let (mut resumed, rx) = scheduled_worker(&u, &tz, scenario);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let event = rx
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if event["claudeSchedule"]["last"]["status"] == expected
                    || (scenario == "claude_changed" && event["claudeSchedule"]["status"] == "off")
                {
                    break;
                }
            }
            resumed.stdin.take();
            assert!(resumed.wait().unwrap().success());
        }
        assert_eq!(
            fs::read_to_string(u.path("home/claude-attempts"))
                .unwrap_or_default()
                .lines()
                .count(),
            calls
        );
        u.run("uninstall");
        assert!(
            !config.join("claude-schedule.json").exists()
                && !record.exists()
                && !record.with_extension("lock").exists()
        );
    }
}

#[test]
fn claude_without_consent_waits_without_claiming_a_due_call() {
    let (u, tz) = scheduled_user("00:00", "23:59");
    u.fake("claude", SCHEDULE_CLAUDE);
    let dir = u.path("home/.config/ai-usage-tray");
    let file = dir.join("claude-schedule.json");
    let mut config = load(&dir.join("schedule.json"));
    config["model"] = "sonnet[1m]".into();
    let record = u.path("home/.local/state/ai-usage-tray/claude-scheduled-call.json");
    for marker in [None, Some(json!(true)), Some(json!(2))] {
        if let Some(value) = marker {
            config["termsConsentVersion"] = value;
        }
        fs::write(&file, config.to_string()).unwrap();
        let (mut child, rx) = scheduled_worker(&u, &tz, "success");
        wait_schedule(&rx, "claudeSchedule", |s| s["status"] == "consent_required");
        child.stdin.take();
        assert!(child.wait().unwrap().success());
        assert!(!record.exists() && !u.path("home/claude-attempts").exists());
    }
    let (mut child, rx) = scheduled_worker(&u, &tz, "success");
    wait_schedule(&rx, "claudeSchedule", |s| s["status"] == "consent_required");
    config["termsConsentVersion"] = 1.into();
    fs::write(&file, config.to_string()).unwrap();
    wait_schedule(&rx, "claudeSchedule", |s| s["last"]["status"] == "ok");
    child.stdin.take();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(u.path("home/claude-attempts"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn repeated_calls_use_the_latest_available_model_and_resume_once() {
    let keys = [
        (
            "schedule",
            "scheduled-call.json",
            "home/attempts",
            "gpt-5.10-luna",
        ),
        (
            "claudeSchedule",
            "claude-scheduled-call.json",
            "home/claude-attempts",
            "Haiku 5.10",
        ),
    ];
    for scenario in ["series", "no_model"] {
        let (u, tz) = scheduled_user("00:00", "23:59");
        u.fake("claude", SCHEDULE_CLAUDE);
        let dir = u.path("home/.config/ai-usage-tray");
        let mut config = load(&dir.join("schedule.json"));
        config["model"] = "".into();
        config["repeatCount"] = 3.into();
        config["followUpMinutes"] = 15.into();
        fs::write(dir.join("schedule.json"), config.to_string()).unwrap();
        config["termsConsentVersion"] = 1.into();
        fs::write(dir.join("claude-schedule.json"), config.to_string()).unwrap();
        let status = if scenario == "series" {
            "ok"
        } else {
            "no_model"
        };
        let final_round = if scenario == "series" { 3 } else { 1 };
        for ordinal in 1..=final_round {
            let (mut worker, rx) = scheduled_worker(&u, &tz, scenario);
            let (mut other, _rx2) = scheduled_worker(&u, &tz, scenario);
            let mut seen = std::collections::HashMap::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            while seen.len() < keys.len() {
                let event = rx
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                for (key, _, _, _) in keys {
                    let state = &event[key];
                    let last = &state["last"];
                    if last["status"] != status || last["ordinal"] != ordinal {
                        continue;
                    }
                    if ordinal == final_round {
                        seen.insert(key, 0);
                    } else if state["ordinal"] == ordinal + 1 {
                        let next = state["next"].as_i64().unwrap();
                        let delta = next - last["time"].as_i64().unwrap();
                        assert!((18000..18900).contains(&delta));
                        seen.insert(key, next);
                    }
                }
            }
            for worker in [&mut worker, &mut other] {
                worker.stdin.take();
                assert!(worker.wait().unwrap().success());
            }
            if ordinal < final_round {
                // Move only the isolated fixture's previous dispatch into the past,
                // so the scheduled worker sees the same next slot as due on restart.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                for (key, record, calls, _) in keys {
                    assert_eq!(
                        fs::read_to_string(u.path(calls)).unwrap().lines().count(),
                        ordinal as usize
                    );
                    let path = u.path("home/.local/state/ai-usage-tray").join(record);
                    let mut last = load(&path);
                    let past = last["time"].as_i64().unwrap() - (seen[key] - now + 1);
                    last["time"] = past.into();
                    last["pending"][0]["time"] = past.into();
                    fs::write(&path, last.to_string()).unwrap();
                }
            }
        }
        for (_, record, calls, model) in keys {
            let last = load(&u.path("home/.local/state/ai-usage-tray").join(record));
            assert_eq!(
                fs::read_to_string(u.path(calls))
                    .unwrap_or_default()
                    .lines()
                    .count(),
                if scenario == "series" { 3 } else { 0 }
            );
            if scenario == "series" {
                assert_eq!(last["model"], model);
            }
        }
    }
}

#[test]
fn old_pending_repeats_move_to_the_five_hour_window_and_expire() {
    use sha2::{Digest, Sha256};
    let (u, tz) = scheduled_user("07:30", "07:45");
    let file = u.path("home/.config/ai-usage-tray/schedule.json");
    let mut config = load(&file);
    config["repeatCount"] = 3.into();
    config["minDelaySeconds"] = 30.into();
    config["maxDelaySeconds"] = 90.into();
    fs::write(&file, config.to_string()).unwrap();
    let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&config).unwrap()));
    let path = u.path("home/.local/state/ai-usage-tray/scheduled-call.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let time: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
    for missed in [false, true] {
        let sent = time.timestamp() - if missed { 6 * 3600 } else { 120 };
        let legacy = json!({"day":time.date_naive().to_string(),"time":sent,"ordinal":1,
            "total":3,"status":"ok","next":sent+30,"scheduleId":id});
        fs::write(&path, legacy.to_string()).unwrap();
        let (mut worker, rx) = scheduled_worker(&u, &tz, "success");
        let state = wait_schedule(&rx, "schedule", |s| s["next"].is_number());
        if missed {
            assert_eq!(state["ordinal"], 1);
            assert!(state["next"].as_i64().unwrap() > time.timestamp());
        } else {
            assert_eq!(state["ordinal"], 2);
            assert!((18000..18900).contains(&(state["next"].as_i64().unwrap() - sent)));
        }
        worker.stdin.take();
        assert!(worker.wait().unwrap().success());
        assert!(!u.path("home/attempts").exists());
    }
}

#[test]
fn overlapping_daily_reservations_keep_both_pending_repeats() {
    use sha2::{Digest, Sha256};
    let (u, tz) = scheduled_user("00:00", "23:59");
    let file = u.path("home/.config/ai-usage-tray/schedule.json");
    let mut config = load(&file);
    config["repeatCount"] = 3.into();
    config["followUpMinutes"] = 600.into();
    fs::write(&file, config.to_string()).unwrap();
    let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&config).unwrap()));
    let now: chrono::DateTime<chrono::Utc> = SystemTime::now().into();
    let yesterday = now.date_naive().pred_opt().unwrap().to_string();
    let earlier = now.timestamp() - 4 * 3600;
    let record = json!({"day":yesterday,"ordinal":2,"time":earlier,"status":"ok","scheduleId":id,
        "first":{"day":yesterday,"time":now.timestamp()-86400},
        "pending":[{"day":yesterday,"ordinal":2,"time":earlier}]});
    let path = u.path("home/.local/state/ai-usage-tray/scheduled-call.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, record.to_string()).unwrap();
    let (mut worker, rx) = scheduled_worker(&u, &tz, "success");
    let state = wait_schedule(&rx, "schedule", |s| {
        s["last"]["pending"]
            .as_array()
            .is_some_and(|p| p.len() == 2)
    });
    let pending = state["last"]["pending"].as_array().unwrap();
    assert_eq!(pending[0], record["pending"][0]);
    assert_eq!(pending[1]["day"], now.date_naive().to_string());
    assert_eq!(pending[1]["ordinal"], 1);
    worker.stdin.take();
    assert!(worker.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(u.path("home/attempts"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}
