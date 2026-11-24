import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const actionPath = '/io/github/moveju112/AIUsageTray/Panel';

export function textState(text) {
    return new GLib.Variant('s', text);
}

export function openScheduleWindow(dir, service, context) {
    const script = dir.get_child('scheduleWindow.js').get_path();
    const command = ['gjs', '-m', script, service]
        .map(arg => GLib.shell_quote(arg.replaceAll('%', '%%'))).join(' ');
    const app = Gio.AppInfo.create_from_commandline(command, service + ' 예약',
        Gio.AppInfoCreateFlags.SUPPORTS_STARTUP_NOTIFICATION);
    app.launch([], context);
}
