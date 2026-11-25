import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Updates} from '../extension/updates.js';

function assert(value, message) {
    if (!value) throw Error(message);
}

// Run with a compiled test schema and GSETTINGS_BACKEND=memory.
const settings = new Gio.Settings({schema_id: 'org.gnome.shell.extensions.ai-usage-tray'});
const updates = new Updates(settings, '', '0.7.0');
const calls = [];
let finishCheck;
updates.command = async name => {
    calls.push(name);
    if (name === 'check-update')
        return new Promise(resolve => { finishCheck = () => resolve({state: 'available', version: '0.8.0'}); });
    return {state: 'installed', version: '0.8.0'};
};

function fireTimer() {
    const context = GLib.MainContext.default();
    context.find_source_by_id(updates.timer).set_ready_time(0);
    while (!updates.busy && context.pending()) context.iteration(false);
    assert(updates.busy, 'the scheduled check started');
}

assert(settings.get_boolean('auto-update') && updates.timer, 'default schedules automatic updates');
fireTimer();
assert(calls.join() === 'check-update', 'timer checks for a release');
settings.set_boolean('auto-update', false);
finishCheck();
await new Promise(resolve => GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
    resolve();
    return GLib.SOURCE_REMOVE;
}));
assert(calls.join() === 'check-update' && !updates.timer && !updates.busy,
    'opting out while checking skips installation and the next timer');

await updates.run(false);
assert(calls.length === 1, 'disabled automatic updates perform no lookup');
await updates.run(true);
assert(calls.join() === 'check-update,update' && !updates.timer, 'manual update works while disabled');

settings.set_boolean('auto-update', true);
assert(updates.timer, 'opting back in schedules the next check');
const run = updates.run(false);
finishCheck();
await run;
assert(calls.join() === 'check-update,update,check-update,update' && updates.timer,
    'enabled automatic updates install an available version and reschedule');
const pending = updates.timer;
settings.set_boolean('auto-update', false);
assert(!GLib.MainContext.default().find_source_by_id(pending), 'opt-out cancels the existing timer');
updates.destroy();

const restarted = new Updates(settings, '', '0.8.0');
assert(!restarted.timer, 'restart respects the saved preference');
restarted.destroy();
print('자동·수동 업데이트 설정 검증 통과');
