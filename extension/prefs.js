import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';
import {panelStyles, detailStyles} from './display.js';

import {actionPath, openScheduleWindow} from './settings.js';

export default class Preferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        window.title = '트레이 설정';
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage();
        const group = new Adw.PreferencesGroup({title: '표시'});
        page.add(group);
        window.add(page);
        const handlers = [];
        for (const [key, title, options] of [
            ['panel-style', '표시 방식', panelStyles],
            ['layout', '상세 메뉴 모양', detailStyles],
        ]) {
            const keys = Object.keys(options);
            const row = new Adw.ComboRow({
                title, model: Gtk.StringList.new(Object.values(options)),
                selected: keys.indexOf(settings.get_string(key)),
            });
            group.add(row);
            row.connect('notify::selected', () => {
                const value = keys[row.selected];
                if (value && settings.get_string(key) !== value) settings.set_string(key, value);
            });
            handlers.push(settings.connect('changed::' + key, () => {
                row.selected = keys.indexOf(settings.get_string(key));
            }));
        }
        const schedules = new Adw.PreferencesGroup({title: '예약 창'});
        page.add(schedules);
        const actions = Gio.DBusActionGroup.get(Gio.DBus.session, 'org.gnome.Shell', actionPath);
        const rows = new Map();
        for (const service of ['Claude', 'Codex']) {
            const row = new Adw.ActionRow({title: service + ' 예약', activatable: true, use_markup: false});
            row.add_suffix(new Gtk.Image({icon_name: 'window-new-symbolic'}));
            row.connect('activated', () => {
                try {
                    openScheduleWindow(this.dir, service, window.get_display().get_app_launch_context());
                } catch (e) {
                    window.add_toast(new Adw.Toast({title: service + ' 예약 창을 열 수 없어.'}));
                    console.error(e);
                }
            });
            schedules.add(row);
            rows.set(service.toLowerCase(), row);
        }
        const updates = new Adw.PreferencesGroup();
        page.add(updates);
        const update = new Adw.ActionRow({title: '프로그램 업데이트', activatable: true, use_markup: false});
        update.add_suffix(new Gtk.Image({icon_name: 'software-update-available-symbolic'}));
        update.connect('activated', () => actions.activate_action('update', null));
        updates.add(update);
        rows.set('update', update);
        const sync = () => {
            for (const [key, row] of rows) {
                const state = actions.get_action_state(key);
                row.subtitle = state?.deepUnpack() ?? '패널 실행 대기';
                if (key === 'update')
                    row.sensitive = actions.get_action_enabled(key);
            }
        };
        const signals = ['action-added', 'action-removed', 'action-state-changed', 'action-enabled-changed']
            .map(signal => actions.connect_after(signal, sync));
        sync();
        window.connect('close-request', () => {
            for (const id of handlers) settings.disconnect(id);
            for (const id of signals) actions.disconnect(id);
            return false;
        });
    }
}
