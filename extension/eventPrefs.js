import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';

export function addEventSource(page, window) {
    const group = new Adw.PreferencesGroup({title: 'Claude 사용량',
        description: 'Grove의 claude.json 파일이나 그 폴더를 지정해. 비워 두면 파일 감지가 꺼져.'});
    page.add(group);
    const dir = GLib.build_filenamev([GLib.get_user_config_dir(), 'ai-usage-tray']);
    const file = Gio.File.new_for_path(GLib.build_filenamev([dir, 'claude-events-path']));
    const entry = new Adw.EntryRow({title: '이벤트 파일 또는 폴더 경로', show_apply_button: true});
    group.add(entry);
    const toast = title => window.add_toast(new Adw.Toast({title}));
    try {
        entry.text = new TextDecoder().decode(file.load_contents(null)[1]).trim();
    } catch (e) {
        if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND)) toast('저장한 경로를 읽을 수 없어.');
    }
    const save = () => {
        let path = entry.text.trim();
        if (path.startsWith('~/')) path = GLib.build_filenamev([GLib.get_home_dir(), path.slice(2)]);
        if (path && !GLib.path_is_absolute(path)) {
            toast('절대 경로나 ~/로 시작하는 경로를 입력해.');
            return;
        }
        try {
            GLib.mkdir_with_parents(dir, 0o700);
            const [saved] = file.replace_contents(new TextEncoder().encode(path + '\n'),
                null, false, Gio.FileCreateFlags.PRIVATE, null);
            if (!saved) throw new Error('경로 저장 실패');
            entry.text = path;
            toast(path ? '이 경로에서 Claude 사용량을 감지해.' : '이벤트 파일 감지를 껐어.');
        } catch {
            toast('경로를 저장할 수 없어.');
        }
    };
    entry.connect('apply', save);
    const browse = new Gtk.Button({icon_name: 'folder-open-symbolic', valign: Gtk.Align.CENTER,
        tooltip_text: 'claude.json이 있는 폴더 선택'});
    entry.add_suffix(browse);
    browse.connect('clicked', () => {
        const dialog = new Gtk.FileDialog({title: 'Claude 사용량 파일이 있는 폴더'});
        dialog.select_folder(window, null, (source, result) => {
            try {
                const path = source.select_folder_finish(result)?.get_path();
                if (path) { entry.text = path; save(); }
            } catch (e) {
                if (!e.matches?.(Gtk.DialogError, Gtk.DialogError.DISMISSED)) toast('폴더를 열 수 없어.');
            }
        });
    });
}
