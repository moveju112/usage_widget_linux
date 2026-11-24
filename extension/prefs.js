import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';
import {panelStyles, detailStyles} from './display.js';

export default class Preferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage();
        const group = new Adw.PreferencesGroup({title: '표시 방식'});
        page.add(group);
        window.add(page);
        const handlers = [];
        for (const [key, title, options] of [
            ['panel-style', '패널 표시', panelStyles],
            ['layout', '상세 메뉴', detailStyles],
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
        const updates = new Adw.PreferencesGroup({title: '프로그램 업데이트'});
        page.add(updates);
        const automatic = new Adw.SwitchRow({title: '자동 설치',
            subtitle: '30분마다 새 버전을 확인해. 끄면 패널 메뉴에서 직접 설치할 수 있어.'});
        updates.add(automatic);
        settings.bind('auto-update', automatic, 'active', Gio.SettingsBindFlags.DEFAULT);
        window.connect('close-request', () => {
            for (const id of handlers) settings.disconnect(id);
            return false;
        });
    }
}
