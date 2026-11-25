import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {textState} from './settings.js';

const interval = 30 * 60;
const now = () => Math.floor(Date.now() / 1000);

export class Updates {
    constructor(settings, binary, runningVersion) {
        this.settings = settings;
        this.binary = binary;
        this.runningVersion = runningVersion;
        this.action = Gio.SimpleAction.new_stateful('update', null, textState('실행 중 v' + runningVersion));
        this.action.connect('activate', () => this.run(true));
        this.changed = settings.connect('changed::auto-update', () => this.schedule());
        this.schedule();
    }

    schedule() {
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        if (!this.settings.get_boolean('auto-update') || this.busy) return;
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
        if (this.busy || (!manual && !this.settings.get_boolean('auto-update'))) return;
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        this.busy = true;
        this.action.set_enabled(false);
        this.action.set_state(textState('새 버전 확인 중…'));
        this.settings.set_int64('last-update-check', now());
        try {
            let result = await this.command(manual ? 'update' : 'check-update');
            if (!this.settings) return;
            if (result.state === 'available' && this.settings.get_boolean('auto-update')) {
                this.action.set_state(textState('v' + result.version + ' 설치 중…'));
                result = await this.command('update');
                if (!this.settings) return;
            }
            const v = 'v' + result.version;
            if (result.state === 'available') this.action.set_state(textState(v + ' 업데이트 가능'));
            else if (result.state === 'installed' || result.version !== this.runningVersion)
                this.action.set_state(textState(v + ' 설치됨 · 다시 로그인하면 적용'));
            else if (result.state === 'unpublished') this.action.set_state(textState('공개된 안정 릴리스 없음'));
            else this.action.set_state(textState(v + ' 최신 버전'));
        } catch (err) {
            if (this.settings) this.action.set_state(textState(err.message));
        } finally {
            if (this.settings) {
                this.busy = false;
                this.action.set_enabled(true);
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
