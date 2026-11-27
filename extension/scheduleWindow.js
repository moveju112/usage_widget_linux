import Adw from 'gi://Adw?version=1';
import Gtk from 'gi://Gtk?version=4.0';
import System from 'system';
import {addSchedulePreferences} from './schedulePrefs.js';

const files = new Map([['Claude', 'claude-schedule.json'], ['Codex', 'schedule.json']]);
const [service] = ARGV;
if (!files.has(service)) System.exit(2);

const app = new Adw.Application({application_id: 'io.github.moveju112.AIUsageTray.' + service});
app.connect('activate', () => {
    let window = app.active_window;
    if (!window) {
        window = new Adw.Window({application: app, title: service + ' 예약',
            default_width: 520, default_height: 650});
        const page = new Adw.PreferencesPage();
        // Keep the save button pinned under the scrolling rows.
        const view = new Adw.ToolbarView({content: page});
        view.add_top_bar(new Adw.HeaderBar());
        view.add_bottom_bar(addSchedulePreferences(page, service, files.get(service)));
        window.content = view;
        const keys = new Gtk.ShortcutController();
        keys.add_shortcut(new Gtk.Shortcut({trigger: Gtk.ShortcutTrigger.parse_string('Escape'),
            action: Gtk.NamedAction.new('window.close')}));
        window.add_controller(keys);
    }
    window.present();
});

System.exit(await app.runAsync([System.programInvocationName]));
