use crate::{claude, codex, common::*};
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::Duration,
};

pub fn config_dir() -> PathBuf {
    env_path("XDG_CONFIG_HOME")
        .unwrap_or_else(|| home().join(".config"))
        .join("ai-usage-tray")
}
pub fn state_dir() -> PathBuf {
    env_path("XDG_STATE_HOME")
        .unwrap_or_else(|| home().join(".local/state"))
        .join("ai-usage-tray")
}

#[derive(Clone, Copy)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    pub fn config_file(self) -> &'static str {
        match self {
            Self::Claude => "claude-schedule.json",
            Self::Codex => "schedule.json",
        }
    }

    pub fn record_file(self) -> &'static str {
        match self {
            Self::Claude => "claude-scheduled-call.json",
            Self::Codex => "scheduled-call.json",
        }
    }

    pub fn state_key(self) -> &'static str {
        match self {
            Self::Claude => "claudeSchedule",
            Self::Codex => "schedule",
        }
    }
}

struct Schedule<'a> {
    start: NaiveTime,
    end: NaiveTime,
    model: &'a str,
    seed: &'a str,
    count: i64,
    follow_up_minutes: i64,
}
impl<'a> Schedule<'a> {
    fn parse(value: &'a Value, provider: Provider) -> Option<Self> {
        let model = value["model"].as_str()?;
        let base = match provider {
            Provider::Claude => model.strip_suffix("[1m]").unwrap_or(model),
            Provider::Codex => model,
        };
        if !model.is_empty()
            && (!base.bytes().next()?.is_ascii_alphanumeric()
                || model.len() > 128
                || !base
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c)))
        {
            return None;
        }
        let integer = |key: &str, default: i64| value.get(key).map_or(Some(default), Value::as_i64);
        let count = integer("repeatCount", 1)?;
        let follow_up_minutes = integer("followUpMinutes", 15)?;
        let start = NaiveTime::parse_from_str(value["start"].as_str()?, "%H:%M").ok()?;
        let end = NaiveTime::parse_from_str(value["end"].as_str()?, "%H:%M").ok()?;
        if !(1..=3).contains(&count) || !(1..=1440).contains(&follow_up_minutes) {
            return None;
        }
        Some(Self {
            start,
            end,
            count,
            follow_up_minutes,
            model,
            seed: value["seed"].as_str().filter(|s| !s.is_empty())?,
        })
    }

    fn follow_up(&self, previous: &Value) -> Option<Window> {
        let day: NaiveDate = previous["day"].as_str()?.parse().ok()?;
        let ordinal = previous["ordinal"].as_i64()?.checked_add(1)?;
        if !(2..=self.count).contains(&ordinal) {
            return None;
        }
        let start = previous["time"].as_i64()?.checked_add(5 * 3600)?;
        let until = start.checked_add(self.follow_up_minutes * 60)?;
        let digest = Sha256::digest(format!("{}:{day}:follow-up:{ordinal}", self.seed));
        let draw = u64::from_be_bytes(digest[..8].try_into().unwrap());
        // Ten-second slots vary even for a one-minute window and leave time to dispatch.
        let at = start + 10 * (draw % (self.follow_up_minutes * 6) as u64) as i64;
        Some(Window {
            day,
            at,
            until,
            ordinal,
        })
    }

    fn window(&self, day: NaiveDate) -> Option<Window> {
        let end_day = if self.end < self.start {
            day.succ_opt()?
        } else {
            day
        };
        let start = local_epoch(day, self.start)?;
        let until = local_epoch(end_day, self.end)? + 60;
        if until <= start {
            return None;
        }
        let digest = Sha256::digest(format!("{}:{day}", self.seed));
        let n = u64::from_be_bytes(digest[..8].try_into().unwrap());
        let minutes = ((until - 1 - start) / 60 + 1) as u64;
        Some(Window {
            day,
            ordinal: 1,
            at: start + (n % minutes) as i64 * 60,
            until,
        })
    }
}
pub struct Call {
    pub model: String,
    until: i64,
    config_file: PathBuf,
    config: Value,
}
impl Call {
    pub fn skipped(&self) -> Option<&'static str> {
        if now() >= self.until as f64 {
            return Some("expired");
        }
        (read_optional(&self.config_file).ok().as_ref() != Some(&self.config)).then_some("changed")
    }
}
#[derive(Clone, Copy)]
struct Window {
    ordinal: i64,
    day: NaiveDate,
    at: i64,
    until: i64,
}
impl Window {
    fn eligible(&self, current: i64, previous: Option<(NaiveDate, NaiveDate)>) -> bool {
        self.until > current
            && previous.is_none_or(|(anchor, attempted)| {
                self.day > anchor
                    && local_day(self.at.max(current)).is_some_and(|day| day > attempted)
            })
    }
}
fn local_day(at: i64) -> Option<NaiveDate> {
    let raw: libc::time_t = at;
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&raw, &mut tm) }.is_null() {
        return None;
    }
    NaiveDate::from_ymd_opt(tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
}
fn local_epoch(day: NaiveDate, time: NaiveTime) -> Option<i64> {
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    tm.tm_year = day.year() - 1900;
    tm.tm_mon = day.month0() as i32;
    tm.tm_mday = day.day() as i32;
    tm.tm_hour = time.hour() as i32;
    tm.tm_min = time.minute() as i32;
    tm.tm_isdst = -1;
    let at = unsafe { libc::mktime(&mut tm) };
    // Skip an interval with a nonexistent endpoint during the spring DST transition.
    (at >= 0 && tm.tm_hour == time.hour() as i32 && tm.tm_min == time.minute() as i32).then_some(at)
}
fn read_optional(path: &Path) -> io::Result<Value> {
    match fs::read(path) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)?;
            if !value.is_object() {
                return Err(io::Error::other("잘못된 예약 파일 형식"));
            }
            Ok(value)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e),
    }
}
fn record(path: &Path, data: &Value) -> io::Result<()> {
    atomic(path, &serde_json::to_vec(data)?, 0o600)?;
    fs::File::open(path)?.sync_all()?;
    fs::File::open(path.parent().unwrap())?.sync_all()
}
fn last_attempt(last: &Value) -> io::Result<Option<(NaiveDate, NaiveDate)>> {
    if last.is_null() {
        return Ok(None);
    }
    last["day"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .zip(last["time"].as_i64().and_then(local_day))
        .map(Some)
        .ok_or_else(|| io::Error::other("예약 기록을 읽을 수 없어."))
}
fn tick(
    provider: Provider,
    paths: &Paths,
    cancel: &AtomicBool,
    publish: &mut impl FnMut(Value),
) -> io::Result<()> {
    let config_file = config_dir().join(provider.config_file());
    let config = read_optional(&config_file)?;
    if config.is_null() || config["enabled"] == false {
        publish(json!({"status":"off"}));
        return Ok(());
    }
    let invalid = || io::Error::other("예약 설정을 읽을 수 없어.");
    if config["enabled"] != true {
        return Err(invalid());
    }
    if matches!(provider, Provider::Claude) && config["termsConsentVersion"].as_u64() != Some(1) {
        publish(json!({"status":"consent_required"}));
        return Ok(());
    }
    let schedule = Schedule::parse(&config, provider).ok_or_else(invalid)?;
    let model_label = if schedule.model.is_empty() {
        match provider {
            Provider::Claude => "자동 · 최신 Haiku",
            Provider::Codex => "자동 · 최신 Luna",
        }
    } else {
        schedule.model
    };
    let dir = state_dir();
    let path = dir.join(provider.record_file());
    let mut last = read_optional(&path)?;
    let first = last.get("first").unwrap_or(&last);
    let done = last_attempt(first)?;
    let current = now() as i64;
    let today = local_day(current).ok_or_else(invalid)?;
    let config_id = format!("{:x}", Sha256::digest(serde_json::to_vec(&config)?));
    // Old records stored an absolute "next" a few seconds after the call. Migrate
    // only an outstanding successful repeat and derive its window from the attempt.
    let predecessors = if last["scheduleId"] != config_id {
        &[][..]
    } else if let Some(saved) = last.get("pending") {
        saved.as_array().ok_or_else(invalid)?.as_slice()
    } else if last["status"] == "ok" && last["next"].is_number() {
        std::slice::from_ref(&last)
    } else {
        &[]
    };
    let mut pending = Vec::new();
    for previous in predecessors {
        let window = schedule.follow_up(previous).ok_or_else(invalid)?;
        if current < window.until {
            pending.push((window, previous));
        }
    }
    let first_window = [-1, 0, 1, 2]
        .into_iter()
        .filter_map(|n| schedule.window(today.checked_add_signed(chrono::TimeDelta::days(n))?))
        .find(|w| w.eligible(current, done));
    let window = first_window
        .into_iter()
        .chain(pending.iter().map(|(window, _)| *window))
        .min_by_key(|window| window.at)
        .ok_or_else(invalid)?;
    let ordinal = window.ordinal;
    publish(
        json!({"status":"on","next":window.at,"model":model_label,"ordinal":ordinal,"total":schedule.count,"last":last}),
    );
    if current < window.at {
        return Ok(());
    }

    fs::create_dir_all(&dir)?;
    let lock = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path.with_extension("lock"))?;
    match lock.try_lock() {
        Ok(()) => (),
        Err(fs::TryLockError::WouldBlock) => return Ok(()),
        Err(fs::TryLockError::Error(e)) => return Err(e),
    }
    if read_optional(&path)? != last
        || read_optional(&config_file)? != config
        || now() >= window.until as f64
    {
        return Ok(());
    }
    // Claim durably before starting the CLI. An interrupted attempt is never replayed.
    let first_marker = if ordinal == 1 {
        json!({"day":window.day.to_string(),"time":current})
    } else {
        json!({"day":first["day"],"time":first["time"]})
    };
    // Another day's first call can fall between two repeats. Keep both chains
    // in the same atomic claim and retire only the slot about to be attempted.
    let remaining: Vec<_> = pending
        .into_iter()
        .filter(|(w, _)| w.day != window.day || w.ordinal != ordinal)
        .map(|(_, p)| json!({"day":p["day"],"ordinal":p["ordinal"],"time":p["time"]}))
        .collect();
    last = json!({"day":window.day.to_string(),"time":current,"model":model_label,"status":"unconfirmed",
        "scheduleId":config_id,"ordinal":ordinal,"total":schedule.count,"first":first_marker,"pending":remaining});
    record(&path, &last)?;
    publish(json!({"status":"running","model":model_label,"last":last}));
    let call = Call {
        model: schedule.model.to_owned(),
        until: window.until,
        config_file,
        config: config.clone(),
    };
    let result = match provider {
        Provider::Claude => claude::scheduled(paths, cancel, call),
        Provider::Codex => codex::scheduled(paths, cancel, call),
    };
    last["status"] = result["status"].clone();
    if let Some(model) = result.get("selectedModel") {
        last["model"] = model.clone();
    }
    if result["status"] == "ok" {
        let sent_at = result["sentAt"].as_i64().ok_or_else(invalid)?;
        last["time"] = sent_at.into();
        if ordinal < schedule.count {
            last["pending"]
                .as_array_mut()
                .unwrap()
                .push(json!({"day":window.day.to_string(),"ordinal":ordinal,"time":sent_at}));
        }
    }
    record(&path, &last)?;
    publish(json!({"status":"on","model":last["model"],"last":last}));
    Ok(())
}

pub fn watch(
    provider: Provider,
    paths: Arc<Paths>,
    cancel: Arc<AtomicBool>,
    stop: mpsc::Receiver<()>,
    mut publish: impl FnMut(Value),
) {
    let mut previous = Value::Null;
    let mut changed = |value: Value| {
        if value != previous {
            previous = value.clone();
            publish(value);
        }
    };
    loop {
        if tick(provider, &paths, &cancel, &mut changed).is_err() {
            changed(json!({"status":"error"}));
        }
        if !matches!(
            stop.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_have_their_own_window_five_hours_after_each_actual_call() {
        let mut config = json!({"start":"23:58","end":"23:58","model":"","seed":"fixture",
            "repeatCount":3,"minDelaySeconds":30,"maxDelaySeconds":90});
        let schedule = Schedule::parse(&config, Provider::Claude).unwrap();
        assert_eq!(schedule.follow_up_minutes, 15); // Old settings use the new default.
        let mut day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut pairs = std::collections::HashSet::new();
        for _ in 0..40 {
            let first = schedule.window(day).unwrap();
            assert_eq!(first.at, local_epoch(day, schedule.start).unwrap());
            let sent = first.at + 13;
            let previous = json!({"day":day.to_string(),"ordinal":1,"time":sent});
            let second = schedule.follow_up(&previous).unwrap();
            assert_eq!(schedule.follow_up(&previous).unwrap().at, second.at);
            let sent_again = second.at + 7;
            let third = schedule
                .follow_up(&json!({"day":day.to_string(),"ordinal":2,"time":sent_again}))
                .unwrap();
            assert_eq!(second.until, sent + 5 * 3600 + 900);
            assert_eq!(third.until, sent_again + 5 * 3600 + 900);
            assert_eq!(local_day(second.at).unwrap(), day.succ_opt().unwrap());
            let offsets = [second.at - sent - 18000, third.at - sent_again - 18000];
            assert!(offsets.iter().all(|s| (0..900).contains(s)));
            pairs.insert(offsets);
            day = day.succ_opt().unwrap();
        }
        assert!(pairs.iter().any(|p| p[0] != p[1]));
        for n in [0, 1, 1440, 1441] {
            config["followUpMinutes"] = n.into();
            assert_eq!(
                Schedule::parse(&config, Provider::Codex).is_some(),
                (1..=1440).contains(&n)
            );
        }
    }

    #[test]
    fn daily_window_is_stable_and_respects_local_boundaries() {
        let config = json!({"start":"23:50", "end":"00:10", "model":"sample-model", "seed":"test"});
        let schedule = Schedule::parse(&config, Provider::Codex).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        let mut offsets = std::collections::BTreeSet::new();
        for offset in 0..20 {
            let date = day + chrono::TimeDelta::days(offset);
            let window = schedule.window(date).unwrap();
            let start = local_epoch(date, schedule.start).unwrap();
            let end = local_epoch(date.succ_opt().unwrap(), schedule.end).unwrap();
            assert_eq!(window.until, end + 60);
            assert!((start..=end).contains(&window.at));
            assert_eq!(window.at, schedule.window(date).unwrap().at);
            offsets.insert(window.at - start);
        }
        assert!(offsets.len() > 1);
        let noon = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let tonight = local_epoch(day, noon).unwrap();
        let current_window = Window {
            ordinal: 1,
            day,
            at: tonight,
            until: tonight + 60,
        };
        // Last night's reservation ran after midnight. A second turn today is forbidden.
        assert!(!current_window.eligible(tonight, Some((day.pred_opt().unwrap(), day))));
        assert!(current_window.eligible(
            tonight,
            Some((day.pred_opt().unwrap(), day.pred_opt().unwrap()))
        ));
        let fixed = Schedule {
            end: schedule.start,
            ..schedule
        };
        assert_eq!(
            fixed.window(day).unwrap().at,
            local_epoch(day, fixed.start).unwrap()
        );
    }
}
