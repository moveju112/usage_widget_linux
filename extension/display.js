const statuses = {
    loading: '조회 중', waiting: '첫 Claude Code 응답 대기', unavailable: '한도 정보 미제공',
    login: '로그인 필요', missing: 'CLI 미설치', timeout: '조회 시간 초과', error: '조회 실패',
};

export function duration(minutes) {
    if (minutes === 10080) return '주간';
    if (minutes % 1440 === 0) return String(minutes / 1440) + '일';
    if (minutes % 60 === 0) return String(minutes / 60) + '시간';
    return String(minutes) + '분';
}

export function percent(value) {
    if (value === 0) return '0%';
    if (value > 0 && value < 0.1) return '<0.1%';
    return String(Math.round(value * 10) / 10) + '%';
}

export function present(data, now, service) {
    const state = data ?? {status: 'loading', windows: []};
    const windows = (state.windows ?? []).map(w => ({
        ...w, stale: now - (w.received ?? state.received ?? now) >= 1800, expired: w.resets != null && w.resets <= now,
    }));
    const valid = windows.filter(w => !w.expired);
    const main = valid.find(w => w.minutes === 300) ?? valid.find(w => w.minutes === 10080) ?? valid[0];
    const stale = valid.some(w => w.stale);
    let status = statuses[state.status] ?? '정상';
    if (state.status === 'ok') {
        if (!valid.length) status = windows.length ? '초기화 후 새 값 대기' : '한도 정보 미제공';
        else if (stale) status = valid.every(w => w.stale) ? '오래된 기록' : '일부 오래된 기록';
    }
    let value = '—';
    if (main) {
        const qualifier = main.minutes === 300 ? '' : duration(main.minutes) + ' ';
        value = qualifier + percent(main.percent) + (main.stale ? '*' : '');
    }
    return {name: service, windows, main, value, status, stale, received: state.received,
        source: state.source, normal: status === '정상'};
}

export const panelStyles = {inline: '1안 · 한 줄형', stacked: '2안 · 두 줄형', rings: '3안 · 이중 링형'};
export const detailStyles = {auto: '패널 방식에 맞춤', list: '목록형', meters: '서비스별 게이지', columns: '나란히 비교'};
export const panelPeriods = [300, 10080];

export function periodLabel(minutes) {
    return minutes === 10080 ? '7일' : duration(minutes);
}

export function reading(view, minutes) {
    const w = view.windows.find(row => row.minutes === minutes);
    const value = w && !w.expired ? w.percent : null;
    const text = value === null ? '—' : percent(value) + (w.stale ? '*' : '');
    return {value, text, stale: w?.stale ?? false, resets: w?.resets, expired: w?.expired ?? false};
}

export function inlineAmounts(view) {
    return panelPeriods.map(minutes => reading(view, minutes).value)
        .filter(value => value !== null).map(percent);
}

// How one VM's last SSH read looks in the settings window. entry comes from claude-vm-status.json.
export function vmState(entry, now) {
    if (!entry) return {label: '확인 대기', tone: 'pending'};
    // The tray checks every minute, so a much older check means nothing is reading this VM.
    if (now - entry.checked > 180) return {label: '확인 멈춤', tone: 'warning'};
    if (entry.status === 'unreachable') return {label: '접속 실패', tone: 'error'};
    if (entry.status === 'ok')
        return now - entry.received < 1800 ? {label: '갱신 중', tone: 'success'} : {label: '오래된 기록', tone: 'warning'};
    const labels = {empty: '기록 없음', none: '프로필 없음', no_account: '로그인 안 됨',
        rejected: '표시 안 함', invalid: '읽기 실패'};
    return {label: labels[entry.status] ?? '확인 실패', tone: 'warning'};
}
