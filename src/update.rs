use crate::install::{destination, installed_version};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

const REPO: &str = "moveju112/usage_widget_linux";
const MAX_BINARY: u64 = 32 * 1024 * 1024;

pub fn version(text: &str) -> Option<[u64; 3]> {
    let mut numbers = [0; 3];
    let mut parts = text.split('.');
    for n in &mut numbers {
        let part = parts.next()?;
        if part.is_empty()
            || !part.bytes().all(|c| c.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *n = part.parse().ok()?;
    }
    parts.next().is_none().then_some(numbers)
}

fn fetch(url: &str, file: &Path, limit: u64, asset: bool) -> io::Result<u16> {
    let mut curl = Command::new("curl");
    curl.args([
        "-q",
        "--silent",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "8",
        "--max-time",
        if asset { "90" } else { "15" },
        "--max-filesize",
        &limit.to_string(),
        "--write-out",
        "%{http_code}",
    ]);
    if asset {
        curl.args(["--location", "--max-redirs", "5"]);
    } else {
        curl.args([
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
        ]);
    }
    let out = curl
        .arg("--output")
        .arg(file)
        .arg(url)
        .stderr(Stdio::null())
        .output()
        .map_err(|_| io::Error::other("업데이트에 필요한 curl을 실행할 수 없어."))?;
    match out.status.code() {
        Some(0) => String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .map_err(|_| io::Error::other("GitHub 응답을 확인할 수 없어.")),
        Some(28) => Err(io::Error::other("GitHub 응답 시간이 초과됐어.")),
        _ => Err(io::Error::other("GitHub에서 다운로드할 수 없어.")),
    }
}

fn http_ok(code: u16) -> io::Result<()> {
    match code {
        200 => Ok(()),
        403 | 429 => Err(io::Error::other(format!(
            "GitHub 요청이 제한됐어 (HTTP {code})."
        ))),
        _ => Err(io::Error::other(format!("GitHub 요청 실패 (HTTP {code})."))),
    }
}

struct Release<'a> {
    number: &'a str,
    url: String,
    size: u64,
    sha: &'a str,
}

fn newer<'a>(data: &'a Value, installed: &str) -> io::Result<Option<Release<'a>>> {
    let bad_release = || io::Error::other("안정 릴리스 버전을 확인할 수 없어.");
    if data["draft"] != false || data["prerelease"] != false {
        return Err(bad_release());
    }
    let number = data["tag_name"]
        .as_str()
        .and_then(|s| s.strip_prefix('v'))
        .ok_or_else(bad_release)?;
    if version(number).ok_or_else(bad_release)? <= version(installed).ok_or_else(bad_release)? {
        return Ok(None);
    }
    let name = format!("ai-usage-tray-linux-{}", env::consts::ARCH);
    let asset = data["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a["name"] == name && a["state"] == "uploaded")
        })
        .ok_or_else(|| {
            io::Error::other(format!("이 릴리스에 {}용 파일이 없어.", env::consts::ARCH))
        })?;
    let url = format!("https://github.com/{REPO}/releases/download/v{number}/{name}");
    if asset["browser_download_url"] != url {
        return Err(io::Error::other("릴리스 파일 주소가 올바르지 않아."));
    }
    let size = asset["size"]
        .as_u64()
        .filter(|s| (1..=MAX_BINARY).contains(s))
        .ok_or_else(|| io::Error::other("릴리스 파일 크기가 올바르지 않아."))?;
    let sha = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| io::Error::other("릴리스에 SHA-256 검증값이 없어."))?;
    Ok(Some(Release {
        number,
        url,
        size,
        sha,
    }))
}

fn report(state: &str, number: &str, as_json: bool) {
    if as_json {
        println!("{}", json!({"state":state,"version":number}));
    } else {
        match state {
            "unpublished" => println!("공개된 안정 릴리스가 없어."),
            "available" => println!("v{number} 업데이트 가능"),
            "installed" => println!("v{number} 설치 완료 · 다음 GNOME 로그인부터 적용"),
            _ => println!("v{number} 최신 버전"),
        }
    }
}

pub fn run(action: &str, as_json: bool) -> io::Result<()> {
    let dest = destination();
    let installed = installed_version(&dest)?;
    // Stage alongside the installation, including on systems that mount /tmp noexec.
    let stage = tempfile::Builder::new()
        .prefix(".ai-usage-update-")
        .tempdir_in(dest.parent().unwrap())?;
    let index = stage.path().join("release.json");
    let code = fetch(
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        &index,
        1024 * 1024,
        false,
    )?;
    if code == 404 {
        report("unpublished", &installed, as_json);
        return Ok(());
    }
    http_ok(code)?;
    let data = serde_json::from_reader(fs::File::open(index)?)
        .map_err(|_| io::Error::other("GitHub 릴리스 정보를 읽을 수 없어."))?;
    let Some(release) = newer(&data, &installed)? else {
        report("current", &installed, as_json);
        return Ok(());
    };
    if action == "check-update" {
        report("available", release.number, as_json);
        return Ok(());
    }
    let candidate = stage.path().join("ai-usage-tray");
    http_ok(fetch(&release.url, &candidate, release.size, true)?)?;
    let mut hash = Sha256::new();
    let length = io::copy(&mut fs::File::open(&candidate)?, &mut hash)?;
    if length != release.size || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(release.sha)
    {
        return Err(io::Error::other(
            "릴리스 파일 무결성 검증 실패. 설치를 중단했어.",
        ));
    }
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))?;
    let applied = Command::new(candidate)
        .args(["apply-update", release.number])
        .output()?;
    let result = std::str::from_utf8(&applied.stdout)
        .ok()
        .and_then(|s| s.trim().split_once(' '));
    match result {
        Some(("installed", n)) if applied.status.success() && n == release.number => {
            report("installed", n, as_json)
        }
        Some(("current", n)) if applied.status.success() && version(n).is_some() => {
            report("current", n, as_json)
        }
        _ => {
            return Err(io::Error::other(
                "새 버전을 적용할 수 없어. 설치 상태와 릴리스 버전을 확인해줘.",
            ));
        }
    }
    Ok(())
}
