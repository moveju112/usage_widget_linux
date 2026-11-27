import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';

const timePattern = /^(?:[01]\d|2[0-3]):[0-5]\d$/;
const modelPattern = /^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$/;
const claudeTermsVersion = 1;

export function addSchedulePreferences(page, name, filename) {
    const group = new Adw.PreferencesGroup({
        description: '매일 첫 호출을 설정한 시간대에 실행해. 호출할 때마다 ' + name + ' 사용량이 소모돼.'});
    page.add(group);
    const file = Gio.File.new_for_path(GLib.build_filenamev([
        GLib.get_user_config_dir(), 'ai-usage-tray', filename,
    ]));
    let current = {enabled: false, start: '07:30', end: '07:45', model: '', seed: GLib.uuid_string_random(),
        repeatCount: 3, followUpMinutes: 15};
    let etag = null;
    let loadError = false;
    try {
        const [, bytes, loadedEtag] = file.load_contents(null);
        // Schedules saved before repeatCount existed run once.
        Object.assign(current, {repeatCount: 1}, JSON.parse(new TextDecoder().decode(bytes)));
        etag = loadedEtag;
    } catch (e) {
        loadError = !e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND);
    }
    const needsConsent = name === 'Claude';
    let consented = current.enabled === true && current.termsConsentVersion === claudeTermsVersion;
    const enabled = new Adw.SwitchRow({title: '매일 예약 호출',
        active: current.enabled === true && (!needsConsent || consented)});
    if (needsConsent && current.enabled === true && !consented)
        enabled.subtitle = '기존 예약은 일시 중지됐어. 다시 켜고 약관 안내에 동의해.';
    group.add(enabled);
    const entries = [
        ['model', '모델 ID (비워 두면 최신 ' + (needsConsent ? 'Haiku' : 'Luna') + ')'],
        ['start', '첫 호출 시작 시각 (HH:MM)'], ['end', '첫 호출 종료 시각 (HH:MM)'],
    ].map(([key, title]) => {
        const row = new Adw.EntryRow({title, text: current[key]});
        group.add(row);
        return row;
    });
    const [model, start, end] = entries;
    const numbers = [
        ['repeatCount', '예약당 호출 횟수', 3],
        ['followUpMinutes', '5시간 뒤 호출할 시간 범위 (분)', 1440],
    ].map(([key, title, max]) => {
        const row = Adw.SpinRow.new_with_range(1, max, 1);
        row.title = title;
        row.value = current[key];
        group.add(row);
        return row;
    });
    const [count, followUp] = numbers;
    followUp.subtitle = '직전 호출 5시간 뒤부터 이 시간 안에 한 번 호출해. 매 회차에 같은 값을 써.';
    const showFollowUp = () => { followUp.visible = count.value > 1; };
    showFollowUp();
    const hint = '같은 시각을 입력하면 해당 분에 실행해. 종료가 더 이르면 다음 날까지야.';
    const status = new Gtk.Label({wrap: true, justify: Gtk.Justification.CENTER, css_classes: ['caption']});
    const setStatus = (text, error = false) => {
        status.label = text;
        status.css_classes = ['caption', error ? 'error' : 'dim-label'];
    };
    setStatus(loadError ? '기존 예약을 읽을 수 없어.' : hint, loadError);
    const save = new Gtk.Button({label: '저장', halign: Gtk.Align.CENTER, sensitive: !loadError,
        css_classes: ['suggested-action', 'pill']});
    const footer = new Gtk.Box({orientation: Gtk.Orientation.VERTICAL, spacing: 12,
        margin_top: 12, margin_bottom: 18, margin_start: 18, margin_end: 18});
    footer.append(status);
    footer.append(save);
    const modified = () => {
        setStatus(hint);
        save.sensitive = true;
    };
    for (const entry of entries) entry.connect('changed', modified);
    for (const row of numbers) row.connect('notify::value', () => { showFollowUp(); modified(); });
    enabled.connect('notify::active', () => {
        modified();
        if (!needsConsent) return;
        if (!enabled.active) {
            consented = false;
            return;
        }
        const dialog = new Adw.AlertDialog({
            heading: 'Claude 예약 호출을 켤까?',
            body: '자동으로 Claude를 호출하면 계정에 적용되는 Anthropic 이용약관에 위배될 수 있어. 이 가능성을 이해하고 예약 호출을 켜는 데 동의해?\n\n' +
                '<a href="https://code.claude.com/docs/en/legal-and-compliance">Anthropic 약관 안내</a>',
            body_use_markup: true,
        });
        dialog.add_response('cancel', '취소');
        dialog.add_response('accept', '동의하고 켜기');
        dialog.default_response = dialog.close_response = 'cancel';
        dialog.connect('response', (_dialog, response) => {
            consented = response === 'accept' && enabled.active;
            if (!consented) enabled.active = false;
        });
        dialog.present(page);
    });
    save.connect('clicked', () => {
        const next = {enabled: enabled.active, model: model.text.trim(), start: start.text.trim(),
            end: end.text.trim(), seed: current.seed, repeatCount: count.value,
            followUpMinutes: followUp.value,
            revision: GLib.uuid_string_random()};
        if (needsConsent && next.enabled) {
            if (!consented) return;
            next.termsConsentVersion = claudeTermsVersion;
        }
        const baseModel = name === 'Claude' ? next.model.replace(/\[1m\]$/, '') : next.model;
        if (next.enabled && next.model && (!modelPattern.test(baseModel) || next.model.length > 128)) {
            setStatus('CLI에서 사용할 모델 ID를 입력해.', true);
            model.grab_focus();
            return;
        }
        if (next.enabled && (!timePattern.test(next.start) || !timePattern.test(next.end))) {
            setStatus('시각은 00:00부터 23:59 사이의 HH:MM으로 입력해.', true);
            (!timePattern.test(next.start) ? start : end).grab_focus();
            return;
        }
        try {
            const parent = file.get_parent();
            if (GLib.mkdir_with_parents(parent.get_path(), 0o700) !== 0)
                throw Error('directory');
            const [, nextEtag] = file.replace_contents(new TextEncoder().encode(JSON.stringify(next)), etag, false,
                Gio.FileCreateFlags.PRIVATE | Gio.FileCreateFlags.REPLACE_DESTINATION, null);
            etag = nextEtag;
            current = next;
            enabled.subtitle = '';
            setStatus(next.enabled ? '저장했어. 트레이 설정에서 다음 예정 시각을 확인해.' : '예약을 껐어.');
            save.sensitive = false;
        } catch (e) {
            setStatus(e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.WRONG_ETAG)
                ? '다른 창에서 예약이 바뀌었어. 설정을 다시 열어줘.' : '예약 파일을 저장할 수 없어.', true);
        }
    });
    return footer;
}
