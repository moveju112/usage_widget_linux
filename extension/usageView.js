import Cairo from 'cairo';
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';
import {inlineAmounts, panelPeriods, percent, periodLabel, reading} from './display.js';

const center = Clutter.ActorAlign.CENTER;
const periodIcons = {300: 'preferences-system-time-symbolic', 10080: 'x-office-calendar-symbolic'};

export function box(vertical, style = '') {
    const layout = new St.BoxLayout({style_class: style});
    setBoxVertical(layout, vertical);
    return layout;
}
export function setBoxVertical(layout, vertical) {
    if (layout.set_orientation)
        layout.set_orientation(vertical ? Clutter.Orientation.VERTICAL : Clutter.Orientation.HORIZONTAL);
    else
        layout.set_vertical(vertical);
}
export function label(text, style = '') {
    return new St.Label({text, style_class: style, y_align: center});
}
export function localTime(seconds, includeSeconds = false) {
    if (seconds == null) return '—';
    const when = GLib.DateTime.new_from_unix_local(Math.floor(seconds));
    if (!when) return '—';
    const today = GLib.DateTime.new_now_local();
    const day = when.format('%F') === today.format('%F') ? '오늘' : '%m/%d';
    return when.format(day + (includeSeconds ? ' %H:%M:%S' : ' %H:%M'));
}
function brand(gicon, key, size) {
    return new St.Icon({gicon, icon_size: size, style_class: 'aut-brand-' + key,
        x_align: center, y_align: center});
}
function periodTitle(minutes) {
    const row = box(false, 'aut-period-title');
    if (periodIcons[minutes]) row.add_child(new St.Icon({
        icon_name: periodIcons[minutes], icon_size: 13, y_align: center,
    }));
    row.add_child(label(periodLabel(minutes)));
    return row;
}

function staleClock(size) {
    return new St.Icon({icon_name: 'preferences-system-time-symbolic',
        icon_size: Math.max(9, Math.round(size / 3)), style_class: 'aut-stale-clock',
        x_align: Clutter.ActorAlign.END, y_align: Clutter.ActorAlign.END});
}

function rings(gicon, key, view, size) {
    const group = new St.Widget({layout_manager: new Clutter.BinLayout(), y_align: center, x_align: center,
        style: 'width: ' + size + 'px; height: ' + size + 'px;'});
    const area = new St.DrawingArea({x_expand: true, y_expand: true});
    const values = panelPeriods.map(m => reading(view, m).value);
    area.connect('repaint', widget => {
        const cr = widget.get_context();
        try {
            const [width, height] = widget.get_surface_size();
            const dim = Math.min(width, height);
            if (dim < 1) return;
            cr.translate(width / 2, height / 2);
            cr.scale(dim / 44, dim / 44);
            const fg = widget.get_theme_node().get_foreground_color();
            const ink = [fg.red / 255, fg.green / 255, fg.blue / 255];
            const teal = fg.green > 140 ? [0.51, 0.86, 0.79] : [0.03, 0.44, 0.38];
            for (const [i, value] of values.entries()) {
                const radius = i === 0 ? 20 : 15.5;
                const start = -Math.PI / 2;
                cr.setLineWidth(2.8);
                cr.setLineCap(Cairo.LineCap.BUTT);
                cr.setDash(value === null ? [3, 3] : [], 0);
                cr.setSourceRGBA(...ink, value === null ? 0.64 : 0.28);
                cr.arc(0, 0, radius, start, start + Math.PI * 2);
                cr.stroke();
                if (value !== null && value > 0) {
                    cr.setDash([], 0);
                    cr.setLineCap(Cairo.LineCap.ROUND);
                    cr.setSourceRGBA(...(i === 0 ? ink : teal), 1);
                    cr.arc(0, 0, radius, start, start + Math.PI * 2 * value / 100);
                    cr.stroke();
                }
            }
        } finally {
            cr.$dispose();
        }
    });
    group.add_child(area);
    group.add_child(brand(gicon, key, Math.round(size * 0.43)));
    if (view.stale) group.add_child(staleClock(size));
    return group;
}

function stackedReadings(view, vertical, thickness) {
    const fontSize = Math.max(8, Math.min(13, Math.floor((thickness - 2) / (vertical ? 4.8 : 2.7))));
    const layout = new Clutter.GridLayout();
    const grid = new St.Widget({layout_manager: layout, style_class: 'aut-stacked',
        style: 'font-size: ' + fontSize + 'px;', y_align: center});
    for (const [row, minutes] of panelPeriods.entries()) {
        const {value} = reading(view, minutes);
        layout.attach(label(minutes === 300 ? 'H' : 'W', 'aut-stack-period'), 0, row, 1, 1);
        const amount = label(value === null ? '—' : percent(value), 'aut-amount');
        amount.x_expand = true;
        amount.x_align = Clutter.ActorAlign.END;
        layout.attach(new St.Bin({style_class: 'aut-stack-value', child: amount}), 1, row, 1, 1);
        layout.attach(label(view.stale && value !== null ? '*' : '', 'aut-stack-marker'), 2, row, 1, 1);
    }
    return grid;
}

export function panelService(gicon, key, view, mode, vertical, thickness) {
    if (mode === 'rings') return rings(gicon, key, view, Math.max(16, Math.min(40, thickness - 4)));
    const size = Math.min(22, Math.max(14, thickness - 6));
    const group = box(vertical, mode === 'stacked' ? 'aut-service' : 'aut-capsule');
    group.y_align = center;
    if (mode === 'stacked') {
        group.add_child(brand(gicon, key, size));
        group.add_child(stackedReadings(view, vertical, thickness));
        return group;
    }
    if (vertical) group.add_style_class_name('aut-capsule-vertical');
    const amounts = inlineAmounts(view);
    const iconSize = Math.min(18, size);
    const icon = brand(gicon, key, iconSize);
    if (view.stale && amounts.length) {
        const overlay = new St.Widget({layout_manager: new Clutter.BinLayout(), x_align: center, y_align: center});
        overlay.add_child(icon);
        overlay.add_child(staleClock(iconSize));
        group.add_child(overlay);
    } else {
        group.add_child(icon);
    }
    for (const [index, amount] of amounts.entries()) {
        if (index && !vertical) group.add_child(label('/', 'aut-slash'));
        group.add_child(label(amount, 'aut-amount'));
    }
    if (!amounts.length) group.add_child(label('—', 'aut-amount'));
    return group;
}

function provider(gicon, key, view) {
    const row = box(false, 'aut-service');
    row.add_child(brand(gicon, key, 22));
    const info = box(true);
    info.add_child(label(view.name, 'aut-name'));
    info.add_child(label('마지막 수신 ' + localTime(view.received), 'aut-meta'));
    info.add_child(label(view.status, view.normal ? 'aut-meta' : 'aut-state'));
    row.add_child(info);
    return row;
}
function cell(view, minutes) {
    const r = reading(view, minutes);
    const group = box(true, 'aut-cell');
    group.add_child(label(r.text, 'aut-cell-value'));
    const reset = r.expired ? '초기화 지남' : r.resets != null ? localTime(r.resets) + ' 초기화' :
        r.value !== null ? '초기화 —' : view.windows.length ? '한도 미제공' : '—';
    group.add_child(label(reset, 'aut-meta'));
    return group;
}

function meter(value, key) {
    const track = new St.Widget({layout_manager: new Clutter.BinLayout(),
        style_class: 'aut-track', x_expand: true, y_align: center});
    if (value > 0) {
        const fill = new St.Widget({style_class: 'aut-fill-' + key, x_expand: true, y_expand: true});
        fill.set_pivot_point(0, 0.5);
        fill.set_scale(value / 100, 1);
        track.add_child(fill);
    }
    return track;
}

export function usageMeters(views, services, icons, narrow, contentWidth) {
    const layout = new Clutter.GridLayout();
    const table = new St.Widget({layout_manager: layout, style_class: 'aut-meters',
        style: 'min-width: ' + contentWidth + 'px;'});
    const columns = narrow ? 3 : 4;
    let row = 0;
    const fullRow = actor => layout.attach(actor, 0, row++, columns, 1);
    for (const [index, view] of views.entries()) {
        const key = services[index][0];
        if (index) fullRow(new St.Widget({style_class: 'aut-rule'}));
        const head = box(narrow, 'aut-meter-head');
        const title = box(false, 'aut-service');
        title.add_child(brand(icons[key], key, 19));
        title.add_child(label(view.name, 'aut-name'));
        head.add_child(title);
        const status = box(false, 'aut-service');
        status.x_expand = true;
        status.x_align = narrow ? Clutter.ActorAlign.START : Clutter.ActorAlign.END;
        status.add_child(label(view.status, view.stale ? 'aut-meta aut-stale-text' : 'aut-meta'));
        if (view.received != null) status.add_child(label('· ' + localTime(view.received) + ' 수신', 'aut-meta'));
        head.add_child(status);
        fullRow(head);

        const notes = [];
        const periods = new Set([...panelPeriods, ...view.windows.map(w => w.minutes)]);
        for (const minutes of periods) {
            const {value, resets, expired} = reading(view, minutes);
            if (value === null) {
                if (view.windows.length)
                    notes.push(periodLabel(minutes) + (expired ? ' 초기화 후 새 값 대기' : ' 한도 미제공'));
                continue;
            }
            layout.attach(label(periodLabel(minutes), 'aut-meter-period'), 0, row, 1, 1);
            layout.attach(label(percent(value), 'aut-meter-number'), 1, row, 1, 1);
            layout.attach(meter(value, key), 2, row, 1, 1);
            const reset = label(resets == null ? '초기화 —' : localTime(resets) + ' 초기화', 'aut-meta aut-meter-reset');
            reset.x_expand = false;
            reset.x_align = Clutter.ActorAlign.END;
            if (narrow) {
                row++;
                fullRow(reset);
            } else {
                layout.attach(reset, 3, row++, 1, 1);
            }
        }
        if (notes.length) fullRow(label(notes.join(' · '), 'aut-meta aut-meter-note'));
    }
    return table;
}

export function usageTable(views, services, icons, mode) {
    const body = box(true, 'aut-sections');
    const layout = new Clutter.GridLayout({orientation: Clutter.Orientation.HORIZONTAL});
    const table = new St.Widget({layout_manager: layout, style_class: 'aut-table'});
    const periods = [...new Set([...panelPeriods, ...views.flatMap(v => v.windows.map(w => w.minutes))])];
    const transpose = mode === 'stacked';
    const put = (actor, x, y) => layout.attach(actor, transpose ? y : x, transpose ? x : y, 1, 1);
    periods.forEach((m, i) => put(periodTitle(m), i + 1, 0));
    views.forEach((v, index) => {
        const key = services[index][0];
        put(provider(icons[key], key, v), 0, index + 1);
        periods.forEach((m, i) => put(cell(v, m), i + 1, index + 1));
    });
    body.add_child(table);
    return body;
}
