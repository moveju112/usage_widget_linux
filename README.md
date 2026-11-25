# AI Usage Tray

A GNOME Shell extension that shows your Claude Code and Codex usage limits in the panel.
It sits in the top bar on stock GNOME and on whichever edge Dash to Panel uses. On a left or right panel the service icons stack vertically.
Supported GNOME Shell versions are 46 and 50. Versions 47–49 are not on the supported list yet, so GNOME will not load the extension there.

- **Usage at a glance**: the panel shows the 5-hour and 7-day usage of Claude and Codex. Click it for reset times and collection status.
- **No model calls for collection**: Claude values come only from what the CLI reports, and Codex uses the logged-in CLI's official rate-limit query.
- **Scheduled calls**: optionally send a short message in a time window you choose, to start the 5-hour limit early. Off by default.
- **Multiple machines**: receive Claude usage from other VMs or servers over SSH and show it in one place.

The interface is in Korean. This README quotes each label as it appears on screen, followed by its meaning.

## Choose your setup

The main PC is the one whose panel shows the usage.

| Where you use Claude Code | What to set up |
|---|---|
| Only on this PC | [Install and run](#install-and-run). Nothing else is needed. |
| Also on other VMs or servers | [Install and run](#install-and-run) on the main PC, then add a sender on each VM: [Receiving Claude usage from other VMs](#receiving-claude-usage-from-other-vms). |

## Install and run

Check your version with `gnome-shell --version`.
The commands below download the file matching `uname -m` from the latest [GitHub release](https://github.com/moveju112/usage_widget_linux/releases/latest) and install it.

```bash
file="ai-usage-tray-linux-$(uname -m)"
curl -fL -o "$file" "https://github.com/moveju112/usage_widget_linux/releases/latest/download/$file"
chmod u+x "$file"
"./$file" install
"./$file" start
```

Run them as a regular user, without `sudo`. Builds are provided for `x86_64` and `aarch64`.

On a first install GNOME may not have discovered the new extension yet. In that case it is registered to start at the next login, and the installer tells you to log out and back in. After that it starts automatically every time you log in to GNOME, including after a reboot.
If only one of the two CLIs is installed, the other service still works.

Click the panel item to see usage. In the menu's top-right corner, the circular arrow to the left of the gear refreshes usage. The gear opens **트레이 설정** (Tray settings), where you choose **표시 방식** (Panel style) and **상세 메뉴 모양** (Detail menu layout).

| Panel style | 5-hour and 7-day usage |
|---|---|
| 1안 · 한 줄형 (Inline) | Shows both values next to the service icon as `5% / 19%`. With only one period it shows just that value, such as `88%`. A stale record adds a clock to the icon. |
| 2안 · 두 줄형 (Two rows) | `H` (5-hour) on top, `W` (7-day) below. Values are right-aligned. |
| 3안 · 이중 링형 (Double ring, default) | The outer ring is 5-hour, the inner ring is 7-day. A dashed ring means no value, an empty solid ring means a real 0%, and a clock badge means a stale record. |

The detail menu lists each service's usage, received and reset times, and collection status. Under **상세 메뉴 모양** you can pick a list, per-service gauges, or a side-by-side comparison. Accounts with only a weekly limit show the 7-day value alone.

## Receiving Claude usage from other VMs

The main PC and every VM need `0.13.0` or later. Install the widget on the main PC, and install only the sender on each VM, using the same binary. A VM does not need GNOME. It needs an OpenSSH client and a systemd user session, and the main PC needs an SSH server the VMs can reach.

1. **On each VM**, log in to Claude Code with your subscription account. Verify the SSH host key of the target and add it to `known_hosts`. Replace `user@main-pc` below with the main PC's SSH account and address, or an alias from `~/.ssh/config`. Create one dedicated key per VM. If a key with the same name already exists, check what it is for before overwriting it.

   ```bash
   curl -fL -o ai-usage-tray "https://github.com/moveju112/usage_widget_linux/releases/latest/download/ai-usage-tray-linux-$(uname -m)"
   chmod u+x ai-usage-tray
   install -d -m 700 ~/.ssh
   ssh-keygen -t ed25519 -N '' -f ~/.ssh/ai-usage-tray
   ./ai-usage-tray sender-install user@main-pc ~/.ssh/ai-usage-tray > vm-pair.json
   ```

   **If the main PC's SSH server does not listen on port 22**, `user@main-pc` alone cannot reach it. Add an alias with the port to `~/.ssh/config` on the VM, then pass the alias (`main-pc` below) to `sender-install` instead.

   ```
   Host main-pc
       HostName 192.168.0.10
       User ubuntu
       Port 49222
   ```

2. Copy only `vm-pair.json` to the **main PC**. It holds the public key and a hash that identifies the account. If you trust that VM, run the commands below once. The SSH entry they create allows nothing but the command that receives usage for that account. The private key stays on the VM.

   ```bash
   tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
   install -d -m 700 ~/.ssh
   (umask 077; "$tray" receiver-key < vm-pair.json >> ~/.ssh/authorized_keys)
   ```

3. **Use Claude Code on the VM as usual.** Your existing status line output is preserved. If nothing shows up, reopen Claude Code.

The tray shows the most recent value among the local and VM records of the same Claude account. You do not need to use the Claude CLI on the main PC. **Keep the VM clocks in sync.** A record more than 5 seconds ahead of the main PC's clock is rejected.

A VM sends over SSH only when Claude reports a new value, and no process stays running while it waits. If the connection is down it keeps the latest record and retries every 5 minutes. Check the waiting state with `systemctl --user status ai-usage-tray-sender.path` and send failures with `journalctl --user -u ai-usage-tray-sender.service`.

To **disconnect**, run `./ai-usage-tray sender-uninstall` on the VM and remove only that VM's public-key line from `~/.ssh/authorized_keys` on the main PC. The sender restores your original status line and leaves the dedicated key files in place.

## Reading the values

| Display | Meaning |
|---|---|
| `5시간 0%` | The CLI reported a real 5-hour usage of 0%. |
| Inline `17%` | Only one period is available. Click the panel to see which one. |
| `32%*` | The record is more than 30 minutes old. The menu shows when it was received. |
| `—` | There is no value to show. The menu gives the reason: waiting for the first response, CLI not installed, login required, query failed, and so on. |

Usage whose reset time has passed is hidden. An unknown reset time is shown as `초기화 —`.
Times follow the OS local time zone.

- Claude: only the 5-hour and 7-day limits that the CLI reports while you use Claude Code are read. Your existing status line output is preserved, and no request is sent just to check usage. If a refresh finds nothing new, the menu shows `다음 Claude Code 응답 대기` (waiting for the next Claude Code response).
- Codex: the logged-in CLI is queried for limits at startup, every 120 seconds, and on refresh. No model is called.
- Only usage percentages, reset times, and received times are stored, in `~/.cache/ai-usage-tray`. Conversations, raw input, and auth tokens are never recorded.

## Scheduled calls for Claude and Codex

In **트레이 설정**, open **Claude 예약** (Claude schedule) or **Codex 예약** (Codex schedule). A new schedule has **매일 예약 호출** (Daily scheduled call) turned off, a first-call window of `07:30~07:45` local time, 3 calls, a 15-minute follow-up range, and automatic model selection. Turn it on per service and save to apply it. Every call consumes account usage. Usage queries and manual refreshes never call a model.

When you turn on the Claude schedule, read the notice that automated calls may violate the terms that apply to your account, press **동의하고 켜기** (Agree and enable), then save. [Anthropic legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)

Leave the model ID empty to pick, right before each call, the newest Haiku (Claude) or the newest Luna (Codex) from the logged-in CLI's model list. If that family is missing, the call is skipped. If you type a model ID, that value is used.

The start and end times apply to the first call of each day. With `07:30`~`07:45`, one of the 16 minutes in that range, both ends included, is chosen. If both times are equal the call runs in that minute, and if the end is earlier than the start the window runs into the next day.

For 2 or 3 calls you set a single **5시간 뒤 호출할 시간 범위 (분)** (range, in minutes, for the call 5 hours later). The default is 15 and the allowed range is 1 to 1440. For example, if the first call is sent at `07:38` with a range of 15, the second runs once within 15 minutes from `12:38`. If the second call is sent at `12:47`, the third runs within 15 minutes from `17:47`. Each call is drawn separately, based on when the previous one was actually sent.

Restarting the tray keeps the draw for the same call. If a call's window has already passed, or the limit is exhausted, the call fails, or it times out, the remaining calls of that schedule are skipped. Turning the schedule off, or editing and saving it, also stops a series in progress. The per-service schedule rows in the tray settings show the next planned time, the latest result, and the call number.

A call only sends `Reply with OK only.` through the logged-in CLI, and the response body is not stored. The same call is never sent twice.

## While locked or suspended

Usage collection and scheduled calls keep running while the screen is locked. The panel item is hidden on the lock screen and comes back when you unlock.

Nothing runs while the machine is suspended or you are logged out. If a call's window has passed by the time the machine is back, the remaining calls of that schedule are skipped.

## Updates

**자동 업데이트** (Automatic updates) in the tray settings is on by default. While the tray is running it checks for the latest stable release every 30 minutes and installs it.
Turning it off stops the periodic check and install. To install manually, press **프로그램 업데이트** (Update program) in the same window.
A new interface takes effect at the next GNOME login.

You can also check or update from a terminal with the installed binary.

```bash
tray="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray"
"$tray" check-update
"$tray" update
```

Updating requires the system `curl`. The download is checked against the size and SHA-256 that GitHub provides before it is installed, and your settings are kept.

## Uninstall

```bash
"${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/ai-usage-tray@local/ai-usage-tray" uninstall
```

This disables the extension and removes the files and cache the tray installed. In the Claude settings it only unwraps its own status line wrapper, keeping your other settings.

**Do not use the remove button in the GNOME Extensions app.** It deletes only the extension folder, leaving the Claude `statusLine` pointing at a binary that no longer exists, so your original status line stops showing. If you already removed it that way, run `install` again and then uninstall with the command above.

## Build and test from source

```bash
cargo build --release --locked
cargo test --locked
gjs -m tests/test_display.js
schemas=$(mktemp -d)
glib-compile-schemas --strict --targetdir="$schemas" extension/schemas
GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR="$schemas" gjs -m tests/test_updates.js
./target/release/ai-usage-tray once
```

Verified with Rust 1.93. The last command prints the values actually received from the logged-in CLIs, once.

## Publishing a release

Match the app version in `Cargo.toml` and `Cargo.lock` with `version-name` in `extension/metadata.json`, and bump the integer `version` used by GNOME.
Push a `vX.Y.Z` tag for that commit, and GitHub Actions tests and builds `x86_64` and `aarch64` and publishes the release.
