import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';

// What a writer has to put in the file. Keep in step with FileSource::read in src/claude/events.rs.
const help = `다른 도구가 이 경로에 아래 JSON 파일을
기록하면 트레이가 읽어. 폴더를 지정하면
그 안의 claude.json을 읽어.

<tt>{
  "version": 1,
  "account": "3f9a…(64자)",
  "windows": {
    "five_hour": {
      "used_percent": 12.5,
      "resets_at": 1911938400,
      "received_at": 1911930000
    },
    "seven_day": { 같은 세 값 }
  }
}</tt>

<b>windows</b>
5시간은 five_hour, 7일은 seven_day.
받은 기간만 넣어.
<b>used_percent</b>
사용률 0~100. 0도 실제 값이야.
<b>resets_at</b>
초기화 시각(Unix 초). 모르면 null.
<b>received_at</b>
그 값을 실제로 받은 시각(Unix 초).
Claude Code에서 받은 값과 기간별로
비교해 더 최근 것을 보여줘.
<b>account</b>
"claude-account-v1/" + 계정 UUID의
SHA-256(소문자 16진수). 이 PC의
Claude Code 계정과 다르면 무시해.`;

export function addEventSource(page, window) {
    const group = new Adw.PreferencesGroup({title: 'Claude 사용량',
        description: '사용량 파일이나 그 폴더를 지정해. 비워 두면 파일 감지가 꺼져.'});
    group.header_suffix = new Gtk.Button({icon_name: 'dialog-question-symbolic', valign: Gtk.Align.CENTER,
        css_classes: ['flat'], tooltip_markup: help});
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
