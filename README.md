# AI Usage Tray

GNOME 패널에서 현재 사용자의 Claude Code와 Codex 한도를 확인하는 확장이야.
기본 GNOME에서는 상단, Dash to Panel에서는 설정된 가장자리에 표시돼.
좌우 패널에서는 서비스 아이콘을 세로로 배치해.
지원하는 GNOME Shell 버전은 46과 50이야. 47~49는 아직 지원 목록에 없어서 GNOME이 확장을 불러오지 않아. Ubuntu 24.04의 GNOME 46.0과 Ubuntu 26.04의 GNOME 50.1에서 확장 버전 검사를 켠 Wayland 세션으로 확인했어.

- **사용량 표시**: 패널에 Claude와 Codex의 5시간·7일 사용률을 보여주고, 누르면 초기화 시각과 수집 상태를 보여줘.
- **모델 호출 없는 수집**: Claude는 CLI가 알려준 값만 읽고, Codex는 로그인된 CLI의 공식 한도 조회만 사용해.
- **예약 호출**: 원하면 정해 둔 시간대에 짧은 메시지를 보내 5시간 한도를 미리 시작해. 기본은 꺼져 있어.
- **여러 PC**: 다른 VM이나 서버에서 쓴 Claude 사용량을 SSH로 받아 한곳에 표시해.

## 설치와 실행

`gnome-shell --version`으로 버전을 확인해. GNOME 50은 트레이 `v0.8.0`부터 지원해.
아래 명령은 [GitHub Releases](https://github.com/moveju112/usage_widget_linux/releases/latest)의 최신 릴리스에서 `uname -m`에 맞는 파일을 받아 설치해. 이전 번호 체계의 `v1.1.0` 사용자도 새 체계로 전환할 때는 한 번 직접 설치해.

```bash
file="ai-usage-tray-linux-$(uname -m)"
curl -fL -o "$file" "https://github.com/moveju112/usage_widget_linux/releases/latest/download/$file"
chmod u+x "$file"
"./$file" install
"./$file" start
```

`sudo` 없이 일반 사용자로 실행해. `x86_64`와 `aarch64` 파일을 제공해.

첫 설치 시 GNOME이 새 확장을 아직 발견하지 못하면 다음 로그인에 자동 활성화되도록 등록해. 이 안내가 나오면 로그아웃 후 다시 로그인해. 그 뒤로는 재부팅해도 GNOME에 로그인할 때마다 자동으로 켜져.
Rust 실행 파일에는 확장 파일이 포함돼. 설치와 활성화에는 Ubuntu의 `glib-compile-schemas`, `gjs`, `gdbus`, `gnome-extensions`를 사용해.
기존 Python판을 설치해 둔 경우에도 같은 설치 명령을 사용해. 기존 래퍼와 캐시를 이어받고, 실행 중인 옛 확장은 다음 GNOME 로그인부터 Rust 수집기로 전환돼.
설치 시 터미널에서 찾은 CLI와 Node 실행 경로를 기록해 GUI에서도 사용할 수 있어.
한쪽 CLI만 설치한 경우에도 다른 서비스는 동작해.

패널을 누르면 사용량이 보여. 메뉴 오른쪽 위에서 톱니바퀴 왼쪽의 원형 화살표를 누르면 사용량을 새로고침해. 톱니바퀴로 **트레이 설정**을 열고 **표시 방식**과 **상세 메뉴 모양**을 선택해. GNOME 확장 앱에서도 같은 설정 창을 열 수 있어. 선택은 즉시 적용되고 다음 로그인에도 유지돼.

| 패널 표시 | 5시간과 7일 사용량 |
|---|---|
| 1안 · 한 줄형 | 서비스 아이콘 옆에 5시간과 7일 사용률을 `5% / 19%` 순서로 표시해. 한 기간만 있으면 `88%`처럼 그 값만 보여. 오래된 기록은 아이콘에 시계를 붙여. |
| 2안 · 두 줄형 | 위는 `H`(5시간), 아래는 `W`(7일)야. 사용률은 오른쪽에 맞춰 표시해. |
| 3안 · 이중 링형 (기본) | 바깥 링은 5시간, 안쪽 링은 7일이야. 값이 없으면 점선, 실제 0%면 연속된 빈 링, 오래된 기록이면 시계 배지가 보여. |

패널을 누르면 서비스별 정확한 값, 수신·초기화 시각, 수집 상태를 볼 수 있어. 상세 메뉴의 기본 배열은 한 줄형·이중 링형에서 서비스별 행, 두 줄형에서 기간별 행이야. **상세 메뉴 모양**에서 목록형·서비스별 게이지·나란히 비교도 선택할 수 있어. 서비스별 게이지에서는 수신한 기간마다 사용률·막대·초기화 시각을 한 줄에 표시해. 주간 한도만 있는 계정은 7일 게이지만 보여.
실행 중인 확장 코드를 업데이트했다면 다음 GNOME 로그인부터 새 화면이 적용돼.

주 패널의 알림 영역을 사용하며 Dash to Panel이나 Grove 설치는 선택 사항이야.

## 사용량 읽기

| 표시 | 의미 |
|---|---|
| `5시간 0%` | CLI가 실제 5시간 사용률 0%를 보고했어. |
| 한 줄형 `17%` | 한 기간만 제공된 경우야. 패널을 누르면 해당 기간을 확인할 수 있어. |
| `32%*` | 마지막 수신 후 30분이 지난 기록이야. 메뉴에서 수신 시각을 확인할 수 있어. |
| `—` | 현재 표시할 값이 없어. 메뉴에서 첫 응답 대기, 미설치, 로그인 필요, 조회 실패 등의 사유를 확인해. |

초기화 시각이 지난 사용률은 숨겨. 알 수 없는 초기화 시각은 `초기화 —`로 표시해.
시각은 OS의 현지 시간대를 따라가. 한도 조회는 별도 Rust 프로세스에서 실행되고 메뉴는 마지막으로 받은 데이터로 즉시 열려.

- Claude Code의 공식 `statusLine` 입력에서 5시간·7일 한도만 읽어. 기존 명령은 동일한 stdin을 받고 터미널 출력과 종료 코드를 유지해. 모델 사용을 유발하는 요청은 보내지 않아.
- 설치된 CLI 버전에서 사용자 계정과 일치하는 로컬 사용량 캐시를 제공하면 원래 수신 시각으로 보완해. 이 캐시는 Claude의 내부 형식이야. Grove를 쓰는 경우에도 CLI가 새 값을 기록해야 갱신돼. 수동 새로고침에서 새 값이 없으면 `다음 Claude Code 응답 대기`가 표시돼.
- Codex는 시작, 120초 주기, 수동 새로고침에 `codex app-server`와 stdio로 통신해. 초기화 핸드셰이크 후 `account/rateLimits/read`를 호출하고 `windowDurationMins`로 기간을 구분해. 조회가 겹치면 진행 중인 결과를 공유해.
- 확장 파일은 사용자 GNOME 확장 디렉터리에 설치돼. Claude 수신값은 `$XDG_CACHE_HOME/ai-usage-tray`(기본 `~/.cache/ai-usage-tray`)에 저장해. 계정 전환 시 이전 값을 표시하지 않도록 프로필을 단방향 해시로 구분해. 대화, 원본 입력, 인증 토큰은 트레이 캐시나 로그에 기록하지 않아.

## Claude와 Codex 예약 호출

**트레이 설정**에서 **Claude 예약** 또는 **Codex 예약**을 선택해. 새 예약은 **매일 예약 호출**이 꺼져 있고, 첫 호출 시간대는 현지 시각 `07:30~07:45`, 호출 횟수는 3회, 후속 호출 범위는 15분, 모델은 자동 선택이야. 서비스별로 예약을 켜고 저장하면 적용돼. 한 번 호출할 때마다 계정 사용량이 소모돼. 사용량 조회와 수동 새로고침은 모델을 호출하지 않아.

Claude 예약을 켤 때는 자동 호출이 계정의 이용약관에 위배될 수 있다는 안내를 읽고 **동의하고 켜기**를 누른 뒤 저장해. 기존 Claude 예약도 동의 후 저장할 때까지 일시 중지돼. [Anthropic 약관 안내](https://code.claude.com/docs/en/legal-and-compliance)

모델 ID를 비워 두면 호출 전에 로그인한 CLI의 모델 목록에서 Claude는 최신 Haiku, Codex는 최신 Luna를 골라. 버전 번호는 숫자로 비교하고, 해당 계열이 없으면 건너뛰어. 모델 ID를 직접 입력하면 그 값을 사용하고, 지우면 자동 선택으로 돌아가. 기존에 저장한 모델 ID도 유지돼.

시작·종료 시각은 매일 첫 호출에 적용돼. `07:30`~`07:45`라면 양 끝을 포함한 16개 분 중 하나를 골라. 두 시각이 같으면 그 분에 실행하고, 종료가 더 이르면 다음 날까지야.

2~3회 예약에서는 **5시간 뒤 호출할 시간 범위 (분)** 하나를 정해. 기본은 15분이고, 1~1440분까지 입력할 수 있어. 예를 들어 첫 호출을 `07:38`에 보내고 범위를 15분으로 설정하면 두 번째는 `12:38`부터 15분 안에 한 번 실행해. 두 번째 호출이 `12:47`이면 세 번째는 `17:47`부터 15분 안에 실행해. 각 회차는 직전 호출을 보낸 시각을 기준으로 별도로 추첨해.

트레이를 다시 켜도 같은 회차의 추첨 결과는 유지돼. 해당 회차의 시간대가 이미 지났거나, 한도 소진·실패·시간 초과가 발생하면 그 예약의 남은 회차를 건너뛰어. 예약을 끄거나 수정해 저장해도 진행 중인 묶음은 중단돼. 이전 버전의 예약에는 기본 15분을 적용해. 트레이 설정의 서비스별 예약 행에서 다음 예정 시각, 최근 결과와 회차를 확인할 수 있어.

트레이는 임시 디렉터리에서 로그인된 Claude Code 또는 Codex CLI로 `Reply with OK only.`를 보내고 응답 본문은 저장하지 않아. 각 시도를 디스크에 먼저 기록해 중복 호출을 막아. 설정은 `$XDG_CONFIG_HOME/ai-usage-tray`의 `claude-schedule.json`과 `schedule.json`(Codex)에, 최근 시도는 `$XDG_STATE_HOME/ai-usage-tray`의 `claude-scheduled-call.json`과 `scheduled-call.json`(Codex)에 저장해.

## 잠금·절전 중 동작

화면을 잠가도 사용량 수집과 예약 호출은 계속돼(`v0.14.0`부터). 잠금 화면에는 패널 아이콘을 표시하지 않고, 잠금을 풀면 다시 보여.

절전 중이거나 로그아웃한 동안에는 실행되지 않아. 다시 켜졌을 때 해당 회차의 시간대가 지났으면 그 예약의 남은 회차를 건너뛰어.

잠긴 상태에서 다른 확장을 끄면 GNOME이 트레이를 다시 시작할 수 있어. 그 순간 진행 중이던 예약 호출은 `응답 미확인`으로 끝나.

## 프로그램 업데이트

트레이 설정의 **자동 업데이트**는 기본으로 켜져 있어. 트레이가 실행 중이면 30분마다 최신 안정 릴리스를 확인하고 설치해. 첫 설치 후에는 약 20초 뒤에 확인해.
끄면 주기적인 확인과 설치가 멈추고 다음 로그인에도 선택이 유지돼. 이미 시작한 설치는 마무리돼. 직접 확인하고 설치하려면 같은 창에서 **프로그램 업데이트**를 눌러.
설치 중에도 사용량 메뉴를 쓸 수 있고 새 확장 코드는 다음 GNOME 로그인에 적용돼. 사용량 새로고침은 별도 동작이야.

터미널에서도 설치된 실행 파일로 확인하거나 업데이트할 수 있어.

```bash
tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
"$tray" check-update
"$tray" update
```

트레이는 공개 GitHub Releases API와 시스템의 `curl`을 사용해. 릴리스의 지정된 아키텍처 파일만 받으며, GitHub가 제공한 크기와 SHA-256을 대조한 뒤 설치해.
Claude 설정, CLI 경로, 사용량 캐시, 표시 방식은 유지돼.

## 다른 VM의 Claude 사용량 받기

메인 PC와 각 VM에는 `0.13.0` 이상이 필요해. 메인 PC에는 위젯을 설치하고, VM에는 같은 실행 파일로 송신기만 설치해. VM에는 GNOME이 필요 없어. Linux의 OpenSSH 클라이언트와 systemd 사용자 세션을 쓰고, 메인 PC에는 VM에서 접근할 수 있는 SSH 서버가 있어야 해.

1. **각 VM**에서 Claude Code 구독 계정으로 로그인해. SSH 접속 대상의 호스트 키를 확인해서 `known_hosts`에 등록해 둬. 아래의 `user@main-pc`는 메인 PC의 SSH 계정과 주소 또는 `~/.ssh/config`의 별칭으로 바꿔. VM마다 별도 전용 키를 한 번 만들어. 같은 이름의 키가 이미 있다면 덮어쓰기 전에 용도를 확인해.

   ```bash
   install -d -m 700 ~/.ssh
   ssh-keygen -t ed25519 -N '' -f ~/.ssh/ai-usage-tray
   ./ai-usage-tray sender-install user@main-pc ~/.ssh/ai-usage-tray > vm-pair.json
   ```

2. **메인 PC**로 `vm-pair.json`만 복사해. 공개키와 계정을 구분하는 해시가 들어 있어. 그 VM을 신뢰하면 아래 명령을 한 번 실행해. 생성되는 SSH 항목은 해당 계정의 사용량을 받는 명령만 허용해. 개인키는 VM에 둬.

   ```bash
   tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
   install -d -m 700 ~/.ssh
   (umask 077; "$tray" receiver-key < vm-pair.json >> ~/.ssh/authorized_keys)
   ```

3. **VM에서 평소처럼 Claude Code를 사용해.** 기존 상태줄 출력은 유지돼. `statusLine`이 5시간·7일 한도 중 하나라도 제공하면 트레이에 반영돼. 이미 열린 터미널에서 상태줄 설정을 읽지 않으면 Claude Code를 다시 열어. 상태줄을 호출하지 않는 실행 방식에서는 전송할 새 값이 생기지 않아.

메인에 로그인된 Claude 계정이 있으면 그 계정의 로컬·원격 기록 중 가장 최근에 관측한 값을 표시해. 메인에서 Claude CLI를 쓰지 않아도 등록한 VM의 기록을 표시할 수 있어. 이때는 첫 수신 계정을 사용해. 여러 VM의 시계를 동기화해 둬. 메인 시각보다 5초 넘게 앞선 기록은 채택하지 않아.

VM은 새 관측값을 파일 한 건으로 보관하고 systemd가 변경을 감지하면 SSH로 전송해. 정상 대기 중에는 송신기 프로세스가 없어. 연결 실패 시 5분 뒤 최신 한 건을 다시 보내고, 메인에서는 VM이 관측한 원래 시각을 유지해. 성공 직후 값이 연속으로 바뀌면 최소 15초 간격으로 모아서 보내고, 같은 사용률과 초기화 시각은 CLI가 계속 알려주는 동안 최대 5분에 한 번 전달해. Claude에 사용량 조회나 모델 호출을 추가하지 않아.

사용자 systemd가 실행 중인 동안 재전송할 수 있어. VM에서 로그아웃해 사용자 세션이 종료되면 다음 세션 시작 때 보관한 값부터 다시 보내. 대기 상태는 `systemctl --user status ai-usage-tray-sender.path`, 전송 실패는 `journalctl --user -u ai-usage-tray-sender.service`로 확인해.

**연결 해제**는 VM에서 `./ai-usage-tray sender-uninstall`을 실행하고 메인 PC의 `~/.ssh/authorized_keys`에서 그 VM의 공개키가 있는 줄만 제거해. 송신기는 기존 상태줄을 복원하고 전용 키 파일은 보존해.

## 제거

```bash
"${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray" uninstall
```

확장을 끄고 트레이가 설치한 파일과 캐시를 제거해. Claude 설정에서는 래퍼만 풀어 다른 설정 변경을 보존해.

**GNOME 확장 앱의 제거 버튼은 쓰지 마.** 확장 폴더만 지워져서 Claude `statusLine`이 없는 실행 파일을 가리키게 되고, 기존 상태줄이 표시되지 않아. 이미 그렇게 지웠다면 다시 `install`한 뒤 위 명령으로 제거해.

## 소스 빌드와 검증

```bash
cargo build --release --locked
cargo test --locked
gjs -m tests/test_display.js
schemas=$(mktemp -d)
glib-compile-schemas --strict --targetdir="$schemas" extension/schemas
GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR="$schemas" gjs -m tests/test_updates.js
./target/release/ai-usage-tray once
```

소스 빌드와 테스트에는 Rust 1.93에서 검증한 Cargo 툴체인을 사용해. 빌드한 파일을 실행할 때는 Rust 설치가 필요 없어.
마지막 명령은 로그인된 CLI의 실제 수신값과 상태를 한 번 출력해.

## 릴리스 발행

앱 버전은 `0.1.0`부터 시작하고, 기능 버전은 `0.2.0`, `0.3.0`처럼 마이너 번호를 올려. 기존 공개 태그 `v1.1.0`은 새 체계의 업데이트 대상에서 제외해.
`Cargo.toml`과 `Cargo.lock`의 앱 버전, `extension/metadata.json`의 `version-name`을 맞추고 GNOME 확장용 정수 `version`도 올려.
그 커밋의 `vX.Y.Z` 태그를 GitHub에 푸시하면 Actions가 Ubuntu 24.04 x86_64와 AArch64에서 각각 테스트하고 빌드해.
두 실행 파일을 모두 업로드한 뒤 릴리스를 공개해. 사용자에게는 Grove, Rust, GitHub 로그인이 필요 없어.
