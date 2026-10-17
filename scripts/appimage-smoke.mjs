// Exercise the packaged launcher, including its bundled-library environment.
// Run only inside the CI's private Xvfb session, after verify:package extracts it.
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { Rpc, processAlive } from './standalone-smoke.mjs';

assert.equal(process.platform, 'linux');
assert.equal(process.env.LOLPING_PRIVATE_XVFB, '1', 'Use a private Xvfb display, never the user desktop');
const repository = dirname(dirname(fileURLToPath(import.meta.url)));
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(name);
  if (at < 0) return fallback;
  assert.ok(args[at + 1] && !args[at + 1].startsWith('--'), `${name} needs a value`);
  return resolve(args[at + 1]);
};
const appdir = join(repository, '.cache/verify/appimage/squashfs-root');
const root = option('--root', join(appdir, 'usr/share/lolping-lite'));
const executable = option('--executable', join(appdir, 'AppRun'));
const bundle = dirname(executable);
const data = join(repository, '.test-data', `appimage-smoke-${Date.now()}`);
const emptyData = join(data, 'empty-xdg-data');
mkdirSync(emptyData, { recursive: true });
for (const path of [root, executable]) assert.ok(existsSync(path), `Missing extracted AppImage input: ${path}`);

async function gone(pid) {
  const until = Date.now() + 6000;
  while (processAlive(pid) && Date.now() < until) await delay(100);
  assert.equal(processAlive(pid), false, `Packaged descendant ${pid} survived shutdown`);
}
function requireBundledLibrary(pid, library) {
  const maps = readFileSync(`/proc/${pid}/maps`, 'utf8');
  assert.ok(maps.includes(join(bundle, 'usr/lib', library)), `${library} was not loaded from the extracted AppImage`);
}

const checks = [];
const rpc = new Rpc(root, data, executable, { env: { XDG_DATA_DIRS: emptyData, XDG_DATA_HOME: emptyData } });
try {
  await rpc.request('settings', { volume: 0, keepLastPing: false, pingDurationS: 1 });
  await rpc.request('start');
  await rpc.poll('state', value => value.status.state === 'running');
  const started = await rpc.request('diagnostics');
  assert.ok(started.helperPid > 0);
  assert.equal(realpathSync(`/proc/${rpc.child.pid}/exe`), realpathSync(join(bundle, 'usr/bin/lolping-lite')));
  requireBundledLibrary(rpc.child.pid, 'libgtk-3.so.0');
  const environment = readFileSync(`/proc/${rpc.child.pid}/environ`, 'utf8').split('\0');
  assert.ok(environment.includes(`GSETTINGS_SCHEMA_DIR=${join(bundle, 'usr/share/glib-2.0/schemas')}`));
  checks.push('AppRun executes the packaged controller and loads the bundled GTK library');

  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  await rpc.poll('diagnostics', value => value.visibleWindows > 0 && value.surfaceBytes > 0);
  const mark = rpc.sequence;
  await rpc.request('ui/open');
  await rpc.waitFor(event => event.event === 'ui/ready', 30000, mark);
  const opened = await rpc.poll('diagnostics', value => value.settingsPid > 0);
  assert.equal(opened.helperPid, started.helperPid);
  requireBundledLibrary(opened.settingsPid, 'libxkbcommon-x11.so.0');
  checks.push('The packaged overlay renders and native OpenGL settings present with bundled XKB dependencies');

  await rpc.request('ui/close');
  await rpc.poll('diagnostics', value => value.settingsPid === null);
  await gone(opened.settingsPid);
  await rpc.request('pause', { on: true });
  assert.equal((await rpc.request('state')).status.enabled, false);
  await rpc.request('pause', { on: false });
  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  const resumed = await rpc.poll('diagnostics', value => value.visibleWindows > 0);
  assert.equal(resumed.helperPid, started.helperPid);
  checks.push('Closing settings and pausing/resuming preserve the input helper and allow subsequent signals');

  await rpc.close();
  await gone(started.helperPid);
  checks.push('Packaged controller EOF releases the helper and settings processes');
} finally { await rpc.close(); }

const report = { target: 'linux-x64', launcher: 'extracted AppImage/AppRun', passed: true, checks };
mkdirSync(join(repository, 'dist'), { recursive: true });
writeFileSync(join(repository, 'dist/appimage-desktop-report.json'), JSON.stringify(report, null, 2) + '\n');
console.log(JSON.stringify(report, null, 2));
