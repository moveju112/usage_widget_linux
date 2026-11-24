import {present, percent, periodLabel, reading} from '../extension/display.js';
function equal(actual, expected) {
    if (actual !== expected) throw Error(JSON.stringify({actual, expected}));
}
const now = 50000;
const win = (minutes, p, resets = now + 100) => ({minutes, percent: p, resets});
const show = (windows, received = now) => present({windows, received, status: 'ok'}, now, 'Codex');
equal(show([win(300,32),win(10080,68)]).value, '32%');
equal(show([win(300,0)]).value, '0%');
equal(show([win(10080,17)]).value, '주간 17%');
equal(show([win(15,4)]).value, '15분 4%');
equal(show([win(300,32)],now-1800).status, '오래된 기록');
equal(show([win(300,32)],now-1800).value, '32%*');
equal(show([win(300,32)],now-1799).value, '32%');
equal(show([win(300,32,now)]).value, '—');
equal(show([win(300,32,now)]).status, '초기화 후 새 값 대기');
equal(show([win(300,32,now-1),win(10080,17)]).value,'주간 17%');
equal(present({status:'waiting'},now,'Claude').status,'첫 Claude Code 응답 대기');
equal(present({status:'timeout'},now,'Codex').value,'—');
equal(percent(0.001),'<0.1%');

const mixed = show([win(10080,68),win(300,0)]);
equal(reading(mixed,300).text, '0%');
equal(reading(mixed,10080).text, '68%');
equal(reading(show([win(10080,23)]),300).value, null);
equal(reading(show([win(300,32,now)]),300).expired, true);
equal(reading(show([win(300,32,now)]),300).text, '—');
equal(reading(show([win(10080,23)],now-1800),10080).text, '23%*');
equal(reading(show([win(15,15)]),300).value, null);
equal(periodLabel(10080), '7일');
equal(reading(present({status:'error'},now,'Codex'),10080).text, '—');
print('표시 상태 검증 통과');
