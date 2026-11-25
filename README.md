# AI Usage Tray

GNOME 패널에 Claude Code와 Codex의 5시간·7일 사용량을 표시합니다. GNOME Shell 46과 50을 지원합니다.

## 메인 PC (GNOME) 설치

```bash
file="ai-usage-tray-linux-$(uname -m)"
curl -fL -o "$file" "https://github.com/moveju112/usage_widget_linux/releases/latest/download/$file"
chmod u+x "$file"
"./$file" install
"./$file" start
```

`sudo` 없이 실행하세요. 처음 설치했다면 한 번 로그아웃했다가 다시 로그인하세요.

## VM·서버 연결 (Sender)

다른 VM이나 서버에서 쓴 Claude 사용량을 메인 PC 패널에 보여줍니다. VM에는 GNOME이 필요 없습니다.

**1. VM에서 실행합니다.** `user@main-pc`는 메인 PC의 SSH 계정과 주소로 바꾸세요.

```bash
ssh user@main-pc true   # 처음이면 호스트 키를 확인하고 수락하세요. 로그인 실패는 무시해도 됩니다
curl -fL -o ai-usage-tray "https://github.com/moveju112/usage_widget_linux/releases/latest/download/ai-usage-tray-linux-$(uname -m)"
chmod u+x ai-usage-tray
install -d -m 700 ~/.ssh
ssh-keygen -t ed25519 -N '' -f ~/.ssh/ai-usage-tray
./ai-usage-tray sender-install user@main-pc ~/.ssh/ai-usage-tray > vm-pair.json
```

메인 PC의 SSH 포트가 22번이 아니면 VM의 `~/.ssh/config`에 별칭을 만들고, 위 명령의 `user@main-pc` 자리에 별칭(`main-pc`)을 넣으세요.

```
Host main-pc
    HostName 192.168.0.10
    User ubuntu
    Port 49222
```

**2. `vm-pair.json`을 메인 PC로 복사한 뒤 메인 PC에서 실행합니다.**

```bash
tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
install -d -m 700 ~/.ssh
(umask 077; "$tray" receiver-key < vm-pair.json >> ~/.ssh/authorized_keys)
```

**3. VM에서 평소처럼 Claude Code를 사용합니다.** VM과 메인 PC의 시계가 맞아야 합니다.

연결을 끊으려면 VM에서 `./ai-usage-tray sender-uninstall`을 실행하고, 메인 PC의 `~/.ssh/authorized_keys`에서 `ai-usage-tray-sender`로 끝나는 해당 줄을 지우세요.

## Claude 사용량 파일 (claude.json)

Claude Code 터미널을 거치지 않는 작업 에이전트나 도구를 쓴다면, 그 도구가 사용량을 파일로 남기게 해서 패널에 표시할 수 있습니다.

1. 사용하는 에이전트·도구가 Claude에서 한도 정보를 받을 때마다 아래 형식의 `claude.json`을 기록하게 합니다.
2. **트레이 설정 → Claude 사용량 → 이벤트 파일 또는 폴더 경로**에 그 파일이나 폴더 경로를 넣고 적용합니다.

```json
{
  "version": 1,
  "account": "<계정 해시>",
  "windows": {
    "five_hour": {"used_percent": 12.5, "resets_at": 1911938400, "received_at": 1911930000},
    "seven_day": {"used_percent": 40, "resets_at": 1912400000, "received_at": 1911930000}
  }
}
```

| 항목 | 넣을 값 |
|---|---|
| `windows` | 5시간은 `five_hour`, 7일은 `seven_day`. 실제로 받은 기간만 넣습니다 |
| `used_percent` | 사용률 0~100 |
| `resets_at` | 초기화 시각(Unix 초). 모르면 `null` |
| `received_at` | 그 값을 실제로 받은 시각(Unix 초). 새 값을 받지 않았으면 바꾸지 않습니다 |
| `account` | 아래 명령으로 구한 값. 이 PC에 로그인된 Claude 계정과 다르면 무시됩니다 |

```bash
python3 -c 'import json,hashlib,os; u=json.load(open(os.path.expanduser("~/.claude.json")))["oauthAccount"]["accountUuid"]; print(hashlib.sha256(("claude-account-v1/"+u).encode()).hexdigest())'
```

파일은 임시 파일에 쓴 뒤 같은 폴더에서 이름을 바꾸는 방식으로 교체하고, 16KB를 넘기지 않아야 합니다. 에이전트에게는 이 절을 그대로 전달하면 됩니다.

## 업데이트와 제거

```bash
tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
"$tray" update      # 기본은 30분마다 자동 업데이트됩니다
"$tray" uninstall
```

GNOME 확장 앱의 제거 버튼은 사용하지 마세요. Claude 상태줄 설정이 복원되지 않습니다.
