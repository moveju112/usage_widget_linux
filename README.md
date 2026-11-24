# AI Usage Tray

GNOME 패널에서 현재 사용자의 Claude Code와 Codex 한도를 확인하는 확장이야.
기본 GNOME에서는 상단, Dash to Panel에서는 설정된 가장자리에 표시돼.
좌우 패널에서는 서비스 아이콘을 세로로 배치해.
GNOME Shell 46과 Ubuntu 24.04에서 검증했어.

## 설치와 실행

처음 설치할 때는 [GitHub Releases](https://github.com/moveju112/usage_widget_linux/releases)에서 `uname -m`에 맞는 파일을 다운로드한 후 아래 명령을 실행해. 기존 버전에 업데이트 메뉴가 없으면 같은 방법으로 한 번 설치하고 다시 로그인해.

```bash
file="./ai-usage-tray-linux-$(uname -m)"
chmod u+x "$file"
"$file" install
"$file" start
```

첫 설치 시 GNOME이 새 확장을 아직 발견하지 못하면 다음 로그인에 자동 활성화되도록 등록해. 이 안내가 나오면 로그아웃 후 다시 로그인해.
Rust 실행 파일에는 확장 파일이 포함돼. 설치와 활성화에는 Ubuntu의 `glib-compile-schemas`, `gjs`, `gdbus`, `gnome-extensions`를 사용해.
기존 Python판을 설치해 둔 경우에도 같은 설치 명령을 사용해. 기존 래퍼와 캐시를 이어받고, 실행 중인 옛 확장은 다음 GNOME 로그인부터 Rust 수집기로 전환돼.
설치 시 터미널에서 찾은 CLI와 Node 실행 경로를 기록해 GUI에서도 사용할 수 있어.
한쪽 CLI만 설치한 경우에도 다른 서비스는 동작해.

패널의 **표시 방식** 메뉴 또는 GNOME 확장 앱의 **AI Usage Tray** 설정에서 모양을 선택해. 선택은 즉시 적용되고 다음 로그인에도 유지돼.

| 패널 표시 | 5시간과 7일 사용량 |
|---|---|
| 1안 · 한 줄형 | 서비스 아이콘 옆에 두 기간의 숫자를 나란히 표시해. |
| 2안 · 두 줄형 | 위쪽은 5시간, 아래쪽은 7일이야. |
| 3안 · 이중 링형 (기본) | 바깥 링은 5시간, 안쪽 링은 7일이야. 값이 없으면 점선, 실제 0%면 연속된 빈 링, 오래된 기록이면 시계 배지가 보여. |

패널을 누르면 서비스별 정확한 값, 수신·초기화 시각, 수집 상태를 볼 수 있어. 상세 메뉴의 기본 배열은 한 줄형·이중 링형에서 서비스별 행, 두 줄형에서 기간별 행이야. **상세 메뉴 모양**에서 기존 목록형·막대형·나란히 비교도 선택할 수 있어.
GNOME 46에서 실행 중인 확장 코드를 업데이트했다면 다음 로그인부터 새 화면이 적용돼.

주 패널의 알림 영역을 사용하며 Dash to Panel이나 Grove 설치는 선택 사항이야.

## 사용량 읽기

| 표시 | 의미 |
|---|---|
| `5시간 0%` | CLI가 실제 5시간 사용률 0%를 보고했어. |
| `5시간 —  7일 17%` | 7일 사용률만 제공된 경우야. |
| `32%*` | 마지막 수신 후 30분이 지난 기록이야. 메뉴에서 수신 시각을 확인할 수 있어. |
| `—` | 현재 표시할 값이 없어. 메뉴에서 첫 응답 대기, 미설치, 로그인 필요, 조회 실패 등의 사유를 확인해. |

초기화 시각이 지난 사용률은 숨겨. 알 수 없는 초기화 시각은 `초기화 —`로 표시해.
시각은 OS의 현지 시간대를 따라가. 한도 조회는 별도 Rust 프로세스에서 실행되고 메뉴는 마지막으로 받은 데이터로 즉시 열려.

- Claude Code의 공식 `statusLine` 입력에서 5시간·7일 한도만 읽어. 기존 명령은 동일한 stdin을 받고 터미널 출력과 종료 코드를 유지해. 모델 사용을 유발하는 요청은 보내지 않아.
- 설치된 CLI 버전에서 사용자 계정과 일치하는 로컬 사용량 캐시를 제공하면 원래 수신 시각으로 보완해. 이 캐시는 Claude의 내부 형식이야. Grove를 쓰는 경우에도 CLI가 새 값을 기록해야 갱신돼. 수동 새로고침에서 새 값이 없으면 `다음 Claude Code 응답 대기`가 표시돼.
- Codex는 시작, 120초 주기, 수동 새로고침에 `codex app-server`와 stdio로 통신해. 초기화 핸드셰이크 후 `account/rateLimits/read`를 호출하고 `windowDurationMins`로 기간을 구분해. 조회가 겹치면 진행 중인 결과를 공유해.
- 확장 파일은 사용자 GNOME 확장 디렉터리에 설치돼. Claude 수신값은 `$XDG_CACHE_HOME/ai-usage-tray`(기본 `~/.cache/ai-usage-tray`)에 저장해. 계정 전환 시 이전 값을 표시하지 않도록 프로필을 단방향 해시로 구분해. 대화, 원본 입력, 인증 토큰은 트레이 캐시나 로그에 기록하지 않아.

## 제거

```bash
"${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray" uninstall
```

확장을 끄고 트레이가 설치한 파일과 캐시를 제거해. Claude 설정에서는 래퍼만 풀어 다른 설정 변경을 보존해.

## 소스 빌드와 검증

```bash
cargo build --release --locked
cargo test --locked
gjs -m tests/test_display.js
./target/release/ai-usage-tray once
```

소스 빌드와 테스트에는 Rust 1.93에서 검증한 Cargo 툴체인을 사용해. 빌드한 파일을 실행할 때는 Rust 설치가 필요 없어.
마지막 명령은 로그인된 CLI의 실제 수신값과 상태를 한 번 출력해.

## 프로그램 업데이트

패널의 **프로그램 업데이트**에서 **지금 업데이트**를 누르면 최신 안정 릴리스를 확인하고 설치해.
**자동 설치**는 기본으로 켜져 있어. 확장을 처음 켠 뒤 약 20초 후, 이후 30분마다 확인해. 자동 설치를 끄면 새 버전이 있는지만 메뉴에 표시해.
설치 중에도 사용량 메뉴를 쓸 수 있고 새 확장 코드는 다음 GNOME 로그인에 적용돼. 사용량 새로고침은 별도 동작이야.

터미널에서도 설치된 실행 파일로 확인하거나 업데이트할 수 있어.

```bash
tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
"$tray" check-update
"$tray" update
```

트레이는 공개 GitHub Releases API와 시스템의 `curl`을 사용해. 릴리스의 지정된 아키텍처 파일만 받으며, GitHub가 제공한 크기와 SHA-256을 대조한 뒤 설치해.
Claude 설정, CLI 경로, 사용량 캐시, 표시 방식은 유지돼.

## 릴리스 발행

`Cargo.toml`과 `Cargo.lock`의 앱 버전, `extension/metadata.json`의 `version-name`을 맞추고 GNOME 확장용 정수 `version`도 올려.
그 커밋의 `vX.Y.Z` 태그를 GitHub에 푸시하면 Actions가 Ubuntu 24.04 x86_64와 AArch64에서 각각 테스트하고 빌드해.
두 실행 파일을 모두 업로드한 뒤 릴리스를 공개해. 사용자에게는 Grove, Rust, GitHub 로그인이 필요 없어.
