import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {panelPeriods, periodLabel, present, reading} from './display.js';
import {box, label, localTime, panelService, usageTable} from './usageView.js';

import {Updates} from './updates.js';
import {actionPath, textState} from './settings.js';

const names = [['claude', 'Claude'], ['codex', 'Codex']];

function spacer() {
    return new St.Widget({x_expand: true});
}
function menuButton(name, icon) {
    return new St.Button({style_class: 'icon-button flat', accessible_name: name,
        can_focus: true, track_hover: true,
        child: new St.Icon({icon_name: icon, icon_size: 18})});
}
function meter(fraction, key) {
    const width = 85;
    const track = box(false, 'aut-track');
    track.x_align = Clutter.ActorAlign.START;
    track.y_align = Clutter.ActorAlign.CENTER;
    track.set_style('width: ' + width + 'px;');
    if (fraction != null && fraction > 0) {
        const fill = new St.Widget({style_class: 'aut-fill-' + key});
        fill.set_style('width: ' + Math.max(1, Math.round(width * fraction / 100)) + 'px;');
        track.add_child(fill);
    }
    return track;
}

export default class AIUsageTray extends Extension {
    enable() {
        this._state = {};
        this._previous = null;
        this._settings = this.getSettings();
        this._icons = Object.fromEntries(names.map(([key]) => [key,
            new Gio.FileIcon({file: this.dir.get_child(key + '-symbolic.svg')})]));
        this._button = new PanelMenu.Button(0.5, 'AI Usage Tray');
        this._strip = box(false, 'aut-panel');
        this._button.add_child(this._strip);
        Main.panel.addToStatusArea(this.uuid, this._button, 0, 'right');

        this._content = new PopupMenu.PopupMenuSection();
        this._content.actor.add_style_class_name('aut-content');
        this._button.menu.addMenuItem(this._content);
        const header = box(false, 'aut-heading');
        header.add_child(label('사용 한도', 'aut-name'));
        header.add_child(spacer());
        const buttons = box(false, 'aut-menu-actions');
        const refreshButton = menuButton('사용량 새로고침', 'view-refresh-symbolic');
        refreshButton.connect('clicked', () => this._actions.activate_action('refresh', null));
        buttons.add_child(refreshButton);
        const gear = menuButton('트레이 설정', 'emblem-system-symbolic');
        gear.connect('clicked', () => {
            this._button.menu.close();
            const settings = global.get_window_actors().map(actor => actor.meta_window)
                .find(w => w.get_title() === '트레이 설정' && w.get_wm_class() === 'org.gnome.Shell.Extensions');
            if (settings) Main.activateWindow(settings);
            else this.openPreferences();
        });
        buttons.add_child(gear);
        header.add_child(buttons);
        this._content.actor.add_child(header);
        this._legend = label('바깥 5시간  ·  안쪽 7일  ·  점선 값 없음', 'aut-meta aut-received');
        this._content.actor.add_child(this._legend);
        this._notice = label('', 'aut-notice');
        this._content.actor.add_child(this._notice);
        this._updates = new Updates(this._settings, this.path + '/ai-usage-tray', this.metadata['version-name']);
        this._actions = new Gio.SimpleActionGroup();
        this._actions.add_action(this._updates.action);
        const refresh = Gio.SimpleAction.new_stateful('refresh', null, textState(''));
        refresh.connect('activate', () => this._refresh());
        this._actions.add_action(refresh);
        for (const [key] of names) {
            const status = Gio.SimpleAction.new_stateful(key, null, textState('예약 상태 확인 중'));
            status.set_enabled(false);
            this._actions.add_action(status);
        }
        this._exportId = Gio.DBus.session.export_action_group(actionPath, this._actions);
        this._settingsChanged = this._settings.connect('changed', () => this._render(true));
        this._button.menu.connect('open-state-changed', (_menu, open) => {
            if (open) this._render(true);
        });
        this._render(true);
        this._start();
        this._ticker = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
            this._render(false);
            return GLib.SOURCE_CONTINUE;
        });
    }

    _start() {
        if (!this._button || this._process) return;
        const cancel = new Gio.Cancellable();
        this._cancel = cancel;
        try {
            const flags = Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE |
                          Gio.SubprocessFlags.STDERR_SILENCE;
            this._process = Gio.Subprocess.new([this.path + '/ai-usage-tray', 'serve'], flags);
            this._input = this._process.get_stdin_pipe();
            const lines = new Gio.DataInputStream({base_stream: this._process.get_stdout_pipe()});
            const read = () => lines.read_line_async(GLib.PRIORITY_DEFAULT, cancel, (stream, result) => {
                if (cancel.is_cancelled()) return;
                try {
                    const [line] = stream.read_line_finish_utf8(result);
                    if (line === null) { this._failed(); return; }
                    this._state = JSON.parse(line);
                    this._render(false);
                    read();
                } catch (_) {
                    this._failed();
                }
            });
            read();
        } catch (_) {
            this._failed();
        }
    }

    _failed() {
        this._stopWorker();
        this._state = {claude: {status: 'error'}, codex: {status: 'error'}};
        this._render(true);
        if (!this._retry) this._retry = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 10, () => {
            this._retry = 0;
            this._start();
            return GLib.SOURCE_REMOVE;
        });
    }

    _stopWorker() {
        this._cancel?.cancel();
        this._cancel = null;
        if (this._input) {
            try { this._input.close(null); } catch (_) { /* Pipe already closed. */ }
        }
        this._input = null;
        this._process = null;
        this._writing = false;
    }

    _refresh() {
        if (!this._input) { this._start(); return; }
        if (this._writing) return;
        this._writing = true;
        const cancel = this._cancel;
        this._input.write_all_async(new TextEncoder().encode('refresh\n'), GLib.PRIORITY_DEFAULT,
            cancel, (stream, result) => {
                if (cancel.is_cancelled()) return;
                this._writing = false;
                try { stream.write_all_finish(result); } catch (_) { this._failed(); }
            });
    }

    _vertical() {
        const actor = this._button;
        if (!actor?.mapped) return false;
        const [x, y] = actor.get_transformed_position();
        const monitor = Main.layoutManager.findMonitorForActor(actor);
        if (!monitor) return false;
        const [width, height] = actor.get_transformed_size();
        const edge = Math.min(x - monitor.x, monitor.x + monitor.width - x - width);
        const end = Math.min(y - monitor.y, monitor.y + monitor.height - y - height);
        return edge < 8 && end > 8;
    }

    _render(force) {
        const now = Date.now() / 1000;
        const views = names.map(([key, name]) => present(this._state[key], now, name));
        const layout = this._settings.get_string('layout');
        const vertical = this._vertical();
        const mode = this._settings.get_string('panel-style');
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const monitor = Main.layoutManager.findMonitorForActor(this._button) ?? Main.layoutManager.primaryMonitor;
        const narrow = monitor && monitor.width / scale < 530;
        const cross = vertical ? this._button.width : this._button.height;
        const thickness = Math.round((cross > 0 ? cross : Main.panel.height || 32 * scale) / scale);
        const signature = JSON.stringify([views, this._state.notice, this._state.schedule, this._state.claudeSchedule, mode, layout, vertical, thickness, narrow,
            GLib.DateTime.new_now_local().format('%F %z')]);
        if (!force && signature === this._previous) return;
        this._previous = signature;
        this._strip.destroy_all_children();
        this._strip.vertical = vertical;
        for (const [i, view] of views.entries()) {
            const key = names[i][0];
            if (i) this._strip.add_child(new St.Widget({style_class: vertical ? 'aut-rule' : 'aut-separator',
                y_align: Clutter.ActorAlign.CENTER}));
            this._strip.add_child(panelService(this._icons[key], key, view, mode, vertical, thickness));
        }
        this._button.accessible_name = views.map(v => v.name + ' ' + panelPeriods.map(m =>
            periodLabel(m) + ' ' + reading(v, m).text).join(', ') + ', ' + v.status).join('. ');
        this._body?.destroy();
        // Extra nonstandard windows grow vertically instead of overflowing narrow displays.
        const standard = views.every(v => v.windows.every(w => panelPeriods.includes(w.minutes)));
        if (layout === 'auto' && !narrow && (standard || mode === 'stacked')) {
            this._body = usageTable(views, names, this._icons, mode);
        } else {
            const columns = layout === 'columns' && !narrow;
            this._body = box(!columns, columns ? 'aut-columns' : 'aut-sections');
            for (const [i, view] of views.entries()) {
                if (i) this._body.add_child(new St.Widget({style_class: columns ? 'aut-vrule' : 'aut-rule'}));
                this._body.add_child(this._section(view, names[i][0], layout, columns, narrow));
            }
        }
        this._content.actor.insert_child_at_index(this._body, 1);
        this._legend.visible = mode === 'rings';
        const statuses = {off: '꺼짐', running: '호출 중', consent_required: '일시 중지 · 예약 설정에서 약관 안내에 동의해.',
            error: '예약 설정이나 기록을 읽을 수 없어.'};
        for (const [key, name] of names) {
            const schedule = this._state[key === 'claude' ? 'claudeSchedule' : 'schedule'];
            const step = entry => entry?.total > 1 ? ' · ' + entry.ordinal + '/' + entry.total + '회' : '';
            let description = schedule?.next
                ? '다음 ' + localTime(schedule.next, schedule.total > 1) + step(schedule) + ' · ' + schedule.model
                : statuses[schedule?.status] ?? (schedule ? '다음 예약 확인 중' : '예약 상태 확인 중');
            const last = schedule?.last;
            if (last) {
                const result = {ok: '성공', error: '실패', missing: name + ' CLI 미설치', timeout: '응답 미확인',
                    login: name === 'Claude' ? 'Claude 계정 로그인 필요' : 'ChatGPT 로그인 필요',
                    no_model: name === 'Claude' ? '사용 가능한 Haiku 없음' : '사용 가능한 Luna 없음',
                    limited: '한도 소진', changed: '예약 변경', expired: '시간대 지남', unconfirmed: '결과 미확인'};
                description += '\n최근 ' + localTime(last.time, last.total > 1) + step(last) + ' · ' + last.model + ' · ' +
                    (schedule.status === 'running' ? '호출 중' : result[last.status] ?? '실패');
            }
            this._actions.lookup_action(key).set_state(textState(description));
        }
        this._notice.text = this._state.notice ?? '';
        this._notice.visible = Boolean(this._state.notice);
        this._actions.lookup_action('refresh').set_state(textState(this._notice.text));
    }

    _section(view, key, layout, columns, narrow) {
        const group = box(true, 'aut-section');
        const head = box(false, 'aut-heading');
        head.add_child(label(view.name, 'aut-name'));
        head.add_child(spacer());
        head.add_child(label(view.status, view.normal ? 'aut-meta' : 'aut-state'));
        group.add_child(head);
        const periods = [...new Set([300, 10080, ...view.windows.map(w => w.minutes)])];
        for (const minutes of periods) {
            const w = view.windows.find(row => row.minutes === minutes);
            const value = reading(view, minutes).text;
            const reset = w?.expired ? '초기화 지남' : w?.resets ? localTime(w.resets) + ' 초기화' : '초기화 —';
            const row = box(columns || narrow, 'aut-row');
            row.add_child(label(periodLabel(minutes), 'aut-period'));
            row.add_child(label(value, columns ? 'aut-large' : 'aut-number'));
            if (!columns && !narrow) row.add_child(spacer());
            row.add_child(label(reset, 'aut-meta'));
            group.add_child(row);
            if (layout === 'meters' && w && !w.expired) group.add_child(meter(w.percent, key));
        }
        group.add_child(label('마지막 수신 ' + localTime(view.received), 'aut-meta aut-received'));
        return group;
    }

    disable() {
        if (this._ticker) GLib.source_remove(this._ticker);
        if (this._retry) GLib.source_remove(this._retry);
        this._ticker = this._retry = 0;
        this._stopWorker();
        Gio.DBus.session.unexport_action_group(this._exportId);
        this._updates.destroy();
        this._updates = this._actions = null;
        this._settings.disconnect(this._settingsChanged);
        this._button?.destroy();
        this._button = this._body = this._settings = this._state = null;
        this._strip = this._legend = this._icons = this._notice = this._content = null;
    }
}
