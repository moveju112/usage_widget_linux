import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';
import {vmState} from './display.js';

const help = `메인 PC가 SSH로 VM에 접속해 Claude Code가
남긴 사용량 기록(~/.claude.json)을 읽어 와.

<b>호스트</b>
user@vm 또는 ~/.ssh/config의 별칭.
비밀번호 없이 접속돼야 해. 터미널에서
<tt>ssh -oBatchMode=yes user@vm true</tt>
가 성공하는지 먼저 확인해.
<b>읽는 주기</b>
60초마다, 그리고 새로고침을 누를 때.
<b>표시 조건</b>
이 PC와 같은 Claude 계정의 값만 보여줘.
VM과 이 PC의 시계가 맞아야 해.
<b>상태</b>
갱신 중: 30분 안에 기록된 값을 읽었어.
오래된 기록: VM의 기록이 30분보다 오래됐어.
기록 없음: Claude Code가 VM에 사용량을
남기지 않았어. 응답마다 최신 값이 필요하면
VM에 Sender를 설치해.
접속 실패: SSH로 접속하지 못했어.
확인 멈춤: 트레이가 이 VM을 읽지 않고 있어.`;
const icons = {success: 'emblem-ok-symbolic', warning: 'dialog-warning-symbolic',
    error: 'dialog-error-symbolic', pending: 'content-loading-symbolic'};

function clock(seconds) {
    const when = GLib.DateTime.new_from_unix_local(Math.floor(seconds));
    if (!when) return '—';
    const today = when.format('%F') === GLib.DateTime.new_now_local().format('%F');
    return when.format(today ? '오늘 %H:%M' : '%m/%d %H:%M');
}

export function addVmHosts(page, window, binary) {
    const group = new Adw.PreferencesGroup({title: 'VM에서 가져오기',
        description: 'VM에 설치하지 않고 SSH로 Claude Code 사용량 기록을 읽어 와.'});
    group.header_suffix = new Gtk.Button({icon_name: 'dialog-question-symbolic', valign: Gtk.Align.CENTER,
        css_classes: ['flat'], tooltip_markup: help});
    page.add(group);
    const hostsFile = Gio.File.new_for_path(GLib.build_filenamev(
        [GLib.get_user_config_dir(), 'ai-usage-tray', 'claude-vm-hosts']));
    const statusFile = Gio.File.new_for_path(GLib.build_filenamev(
        [GLib.get_user_cache_dir(), 'ai-usage-tray', 'claude-vm-status.json']));
    let open = true;
    const toast = title => { if (open) window.add_toast(new Adw.Toast({title})); };
    const load = file => {
        try {
            return new TextDecoder().decode(file.load_contents(null)[1]);
        } catch {
            return '';
        }
    };
    // The tray binary validates the host and talks to SSH. This window only shows what it reports.
    const run = (action, host) => new Promise(resolve => {
        try {
            const process = Gio.Subprocess.new([binary, action, host],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
            process.communicate_utf8_async(null, null, (proc, res) => {
                try {
                    const [, out, err] = proc.communicate_utf8_finish(res);
                    const ok = proc.get_successful();
                    resolve({ok, text: (ok ? out : err).trim()});
                } catch {
                    resolve({ok: false, text: '결과를 읽을 수 없어.'});
                }
            });
        } catch {
            resolve({ok: false, text: '트레이 실행 파일을 실행할 수 없어.'});
        }
    });
    const entry = new Adw.EntryRow({title: 'SSH 호스트 (user@vm 또는 ~/.ssh/config 별칭)',
        show_apply_button: true});
    group.add(entry);
    const testing = new Set();
    let rows = [];
    const reload = () => {
        if (!open) return;
        for (const row of rows) group.remove(row);
        rows = [];
        let status = {};
        try {
            status = JSON.parse(load(statusFile) || '{}');
        } catch { /* Shown as waiting until the tray writes it again. */ }
        const now = Date.now() / 1000;
        for (const host of load(hostsFile).split('\n').map(line => line.trim()).filter(Boolean)) {
            const last = status[host];
            const {label, tone} = vmState(last, now);
            const detail = last ? [last.text, [last.received ? '기록 ' + clock(last.received) : null,
                '확인 ' + clock(last.checked)].filter(Boolean).join(' · ')].join('\n')
                : '트레이가 곧 읽어 와. 테스트를 누르면 바로 확인해.';
            const busy = testing.has(host);
            const row = new Adw.ActionRow({title: host, use_markup: false,
                subtitle: busy ? '접속 중…' : label + ' · ' + detail});
            row.add_prefix(new Gtk.Image({icon_name: icons[tone], css_classes: tone === 'pending' ? ['dim-label'] : [tone],
                tooltip_text: label}));
            const test = new Gtk.Button({label: '테스트', valign: Gtk.Align.CENTER, sensitive: !busy,
                tooltip_text: host + ' 접속과 현재 값 확인'});
            const remove = new Gtk.Button({icon_name: 'user-trash-symbolic', valign: Gtk.Align.CENTER,
                css_classes: ['flat'], sensitive: !busy, tooltip_text: host + ' 등록 해제'});
            row.add_suffix(test);
            row.add_suffix(remove);
            test.connect('clicked', async () => {
                testing.add(host);
                reload();
                const {ok, text} = await run('vm-test', host);
                testing.delete(host);
                // The result is also in the status file; a failure is worth saying out loud.
                if (!ok) toast(text);
                reload();
            });
            remove.connect('clicked', async () => {
                test.sensitive = remove.sensitive = false;
                toast((await run('vm-remove', host)).text);
                reload();
            });
            group.add(row);
            rows.push(row);
        }
    };
    entry.connect('apply', async () => {
        const host = entry.text.trim();
        if (!host) return;
        entry.sensitive = false;
        const {ok, text} = await run('vm-add', host);
        if (!open) return;
        entry.sensitive = true;
        toast(text);
        if (ok) entry.text = '';
        // Disabling the row dropped the focus; give it back so a typo can be fixed in place.
        else entry.grab_focus();
        reload();
    });
    // The tray rewrites the status every minute; the timer covers a tray that has stopped.
    const monitors = [hostsFile, statusFile].map(file => {
        const monitor = file.monitor_file(Gio.FileMonitorFlags.NONE, null);
        monitor.connect('changed', reload);
        return monitor;
    });
    const timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 30, () => {
        reload();
        return GLib.SOURCE_CONTINUE;
    });
    window.connect('close-request', () => {
        open = false;
        for (const monitor of monitors) monitor.cancel();
        GLib.source_remove(timer);
        return false;
    });
    reload();
}
