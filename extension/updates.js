import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const interval = 30 * 60;
const now = () => Math.floor(Date.now() / 1000);

export class UpdateMenu {
    constructor(menu, settings, binary, runningVersion) {
        this.settings = settings;
        this.binary = binary;
        this.runningVersion = runningVersion;
        const updates = new PopupMenu.PopupSubMenuMenuItem('프로그램 업데이트');
        menu.addMenuItem(updates);
        this.status = new PopupMenu.PopupMenuItem('실행 중 v' + runningVersion, {reactive: false});
        updates.menu.addMenuItem(this.status);
        this.manual = new PopupMenu.PopupMenuItem('지금 업데이트');
        updates.menu.addMenuItem(this.manual);
        this.manual.connect('activate', () => {
            GObject.signal_stop_emission_by_name(this.manual, 'activate');
            this.run(true);
        });
        const auto = new PopupMenu.PopupSwitchMenuItem('자동 설치', settings.get_boolean('auto-update'));
        updates.menu.addMenuItem(auto);
        auto.connect('toggled', (_item, enabled) => settings.set_boolean('auto-update', enabled));
        this.changed = settings.connect('changed::auto-update', () => {
            auto.setToggleState(settings.get_boolean('auto-update'));
        });
        this.schedule();
    }

    schedule() {
        const elapsed = now() - this.settings.get_int64('last-update-check');
        const delay = elapsed < 0 ? 20 : Math.max(20, interval - elapsed);
        this.timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, delay, () => {
            this.timer = 0;
            this.run(false);
            return GLib.SOURCE_REMOVE;
        });
    }

    async command(name) {
        const process = Gio.Subprocess.new([this.binary, name, '--json'],
            Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_PIPE);
        const [, out, err] = await new Promise((resolve, reject) => {
            process.communicate_utf8_async(null, null, (proc, res) => {
                try { resolve(proc.communicate_utf8_finish(res)); }
                catch (error) { reject(error); }
            });
        });
        if (!process.get_successful()) throw Error(err.trim() || '업데이트 실패');
        return JSON.parse(out);
    }

    async run(manual) {
        if (this.busy) return;
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        this.busy = true;
        this.manual.setSensitive(false);
        this.status.label.text = '새 버전 확인 중…';
        this.settings.set_int64('last-update-check', now());
        try {
            let result = await this.command(manual ? 'update' : 'check-update');
            if (!this.settings) return;
            if (!manual && result.state === 'available' && this.settings.get_boolean('auto-update')) {
                this.status.label.text = 'v' + result.version + ' 설치 중…';
                result = await this.command('update');
                if (!this.settings) return;
            }
            const v = 'v' + result.version;
            if (result.state === 'available') this.status.label.text = v + ' 업데이트 가능';
            else if (result.state === 'installed' || result.version !== this.runningVersion)
                this.status.label.text = v + ' 설치됨 · 다시 로그인하면 적용';
            else if (result.state === 'unpublished') this.status.label.text = '공개된 안정 릴리스 없음';
            else this.status.label.text = v + ' 최신 버전';
        } catch (err) {
            if (this.settings) this.status.label.text = err.message;
        } finally {
            if (this.settings) {
                this.busy = false;
                this.manual.setSensitive(true);
                this.schedule();
            }
        }
    }

    destroy() {
        if (this.timer) GLib.source_remove(this.timer);
        this.settings.disconnect(this.changed);
        this.settings = null;
    }
}
