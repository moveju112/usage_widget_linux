import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {detailStyles, panelPeriods, panelStyles, periodLabel, present, reading} from './display.js';
import {box, label, localTime, panelService, usageTable} from './usageView.js';

import {UpdateMenu} from './updates.js';

const names = [['claude', 'Claude'], ['codex', 'Codex']];

function spacer() {
    return new St.Widget({x_expand: true});
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
        this._legend = label('바깥 5시간  ·  안쪽 7일  ·  점선 값 없음', 'aut-meta aut-received');
        this._content.actor.add_child(this._legend);
        this._notice = label('', 'aut-notice');
        this._content.actor.add_child(this._notice);
        this._button.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        const refresh = new PopupMenu.PopupImageMenuItem('새로고침', 'view-refresh-symbolic');
        refresh.connect('activate', () => {
            GObject.signal_stop_emission_by_name(refresh, 'activate');
            this._refresh();
        });
        this._button.menu.addMenuItem(refresh);
        this._choices = {};
        for (const [setting, title, styles] of [
            ['panel-style', '표시 방식', panelStyles], ['layout', '상세 메뉴 모양', detailStyles],
        ]) {
            const submenu = new PopupMenu.PopupSubMenuMenuItem(title);
            this._button.menu.addMenuItem(submenu);
            this._choices[setting] = Object.entries(styles).map(([key, name]) => [key,
                submenu.menu.addAction(name, () => this._settings.set_string(setting, key))]);
        }
        this._updates = new UpdateMenu(this._button.menu, this._settings,
            this.path + '/ai-usage-tray', this.metadata['version-name']);
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
        const signature = JSON.stringify([views, this._state.notice, mode, layout, vertical, thickness, narrow,
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
        this._content.actor.insert_child_at_index(this._body, 0);
        this._legend.visible = mode === 'rings';
        this._notice.text = this._state.notice ?? '';
        this._notice.visible = Boolean(this._state.notice);
        for (const [setting, items] of Object.entries(this._choices)) {
            const selected = this._settings.get_string(setting);
            for (const [key, item] of items)
                item.setOrnament(key === selected ? PopupMenu.Ornament.DOT : PopupMenu.Ornament.NONE);
        }
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
        this._updates.destroy();
        this._updates = null;
        this._settings.disconnect(this._settingsChanged);
        this._button?.destroy();
        this._button = this._body = this._choices = this._settings = this._state = null;
        this._strip = this._legend = this._icons = this._notice = this._content = null;
    }
}
