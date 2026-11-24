import Adw from 'gi://Adw?version=1';
import System from 'system';
import {addSchedulePreferences} from './schedulePrefs.js';

const files = new Map([['Claude', 'claude-schedule.json'], ['Codex', 'schedule.json']]);
const [service] = ARGV;
if (!files.has(service)) System.exit(2);

const app = new Adw.Application({application_id: 'io.github.moveju112.AIUsageTray.' + service});
app.connect('activate', () => {
    let window = app.active_window;
    if (!window) {
        window = new Adw.PreferencesWindow({application: app, title: service + ' 예약',
            search_enabled: false, default_width: 520, default_height: 650});
        const page = new Adw.PreferencesPage();
        window.add(page);
        addSchedulePreferences(page, service, files.get(service));
    }
    window.present();
});

System.exit(await app.runAsync([System.programInvocationName]));
