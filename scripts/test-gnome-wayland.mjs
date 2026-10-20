// Invoked only by test-gnome-wayland.py inside its private X server and D-Bus.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { Rpc } from './standalone-smoke.mjs';
import { extensionUuid } from './gnome-extension.mjs';
import { BUS_NAME, OBJECT_PATH, INTERFACE } from '../native/gnome-extension/lolping-lite@ezreal-byte.github.io/protocol.js';

assert.equal(process.platform, 'linux');
assert.equal(process.env.LOLPING_PRIVATE_GNOME, '1', 'Use the isolated GNOME launcher, never the login desktop');
assert.ok(process.env.DBUS_SESSION_BUS_ADDRESS, 'The private D-Bus session must be running');
assert.ok(process.env.LOLPING_GNOME_TEST_DATA && process.env.XDG_RUNTIME_DIR);
const repository = dirname(dirname(fileURLToPath(import.meta.url)));
const arguments_ = process.argv.slice(2);
const option = name => {
  const index = arguments_.indexOf(name);
  assert.ok(index >= 0 && arguments_[index + 1], `Missing ${name}`);
  return resolve(arguments_[index + 1]);
};
const root = option('--root'), executable = option('--executable');
const testData = process.env.LOLPING_GNOME_TEST_DATA;
const reportPath = join(repository, 'dist/gnome-wayland-report.json');
const logPath = join(testData, 'gnome-shell.log');
const waylandDisplay = 'lolping-test-wayland';
const checks = [];
let shell, rpc, inputClient;

function command(program, args, allowFailure = false) {
  const result = spawnSync(program, args, { encoding: 'utf8', timeout: 15000 });
  if (!allowFailure) assert.equal(result.status, 0, `${program} ${args.join(' ')}: ${result.error ?? result.stderr}`);
  return result;
}
const dbus = (method, args = [], allowFailure = false) => command('gdbus', [
  'call', '--session', '--dest', BUS_NAME, '--object-path', OBJECT_PATH,
  '--method', `${INTERFACE}.${method}`, ...args,
], allowFailure);
async function until(fn, reason, milliseconds = 30000) {
  const deadline = Date.now() + milliseconds;
  do {
    if (shell && shell.exitCode !== null) throw Error(`GNOME Shell exited (${shell.exitCode}); see ${logPath}`);
    const value = await fn();
    if (value) return value;
    await delay(150);
  } while (Date.now() < deadline);
  throw Error(`${reason}; see ${logPath}`);
}
function privateInput(...args) {
  assert.equal(process.env.LOLPING_PRIVATE_GNOME, '1');
  return command('xdotool', args);
}
function releaseKeys() {
  command('xdotool', ['keyup', 'q', 'e', 'p', 'Escape', 'Alt_L', 'Super_L'], true);
}
async function keyCombo(key) {
  privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'key', key, 'keyup', 'Alt_L', 'keyup', 'Super_L');
  await delay(100);
}
async function finishShell() {
  if (!shell || shell.exitCode !== null) return;
  shell.kill('SIGTERM');
  await Promise.race([new Promise(resolveExit => shell.once('exit', resolveExit)), delay(5000)]);
  if (shell.exitCode === null) shell.kill('SIGKILL');
}

try {
  command('gsettings', ['set', 'org.gnome.shell', 'enabled-extensions', `['${extensionUuid}']`]);
  command('gsettings', ['set', 'org.gnome.shell', 'disable-user-extensions', 'false']);
  command('gsettings', ['set', 'org.gnome.desktop.interface', 'enable-animations', 'false']);
  command('gsettings', ['set', 'org.gnome.shell', 'welcome-dialog-last-shown-version', '46.0'], true);
  shell = spawn('gnome-shell', ['--nested', '--wayland', `--wayland-display=${waylandDisplay}`], {
    stdio: ['ignore', 'pipe', 'pipe'], env: process.env,
  });
  for (const stream of [shell.stdout, shell.stderr]) stream.on('data', bytes => appendFileSync(logPath, bytes));
  shell.once('error', error => appendFileSync(logPath, `${error}\n`));
  await until(() => existsSync(join(process.env.XDG_RUNTIME_DIR, waylandDisplay)), 'GNOME Wayland display did not appear');
  await until(() => command('gdbus', ['introspect', '--session', '--dest', BUS_NAME, '--object-path', OBJECT_PATH], true).status === 0,
    'Bundled GNOME extension did not export its bridge');
  checks.push('GNOME 46 loads the bundled extension on a real nested Wayland compositor');

  const windowId = await until(() => {
    const candidates = command('xdotool', ['search', '--onlyvisible', '--pid', String(shell.pid)], true).stdout.trim().split('\n').filter(Boolean);
    const windows = candidates.map(id => {
      const result = command('xdotool', ['getwindowgeometry', '--shell', id], true);
      const geometry = Object.fromEntries(result.stdout.trim().split('\n').map(line => line.split('=')));
      return { id, area: Number(geometry.WIDTH) * Number(geometry.HEIGHT) };
    }).filter(window => window.area >= 640 * 480).sort((a, b) => b.area - a.area);
    return windows[0]?.id;
  }, 'Cannot find the private nested compositor window');
  privateInput('windowfocus', '--sync', windowId);
  privateInput('mousemove', '--window', windowId, '640', '480');
  privateInput('key', 'Escape');
  await delay(600);

  rpc = new Rpc(root, join(testData, 'application'), executable, {
    production: true,
    env: { WAYLAND_DISPLAY: waylandDisplay, XDG_SESSION_TYPE: 'wayland', GDK_BACKEND: 'wayland' },
  });
  await rpc.waitFor(event => event.event === 'ui/ready', 40000);
  await rpc.request('settings', { volume: 0, keepLastPing: false, pingDurationS: 1 });
  await rpc.request('start');
  await rpc.poll('state', value => value.status.state === 'running');
  const initial = await rpc.request('diagnostics');
  assert.equal(initial.renderer, 'native-gnome-wayland');
  assert.equal(initial.helperPid, null, 'Wayland must never spawn the X11 input helper');
  assert.equal(initial.connected, true);
  assert.ok(initial.settingsPid > 0, 'Native settings must remain usable on Wayland');
  checks.push('Packaged Rust controller and native settings start on Wayland without an X11 input helper');

  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  await rpc.poll('diagnostics', value => value.visibleWindows > 0 && value.surfaceBytes > 0 && value.presentedFrames > initial.presentedFrames);
  await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  checks.push('Native signal frames are acknowledged by GNOME and cleared when the animation finishes');

  const foreignClear = dbus('Clear', ['0'], true);
  assert.notEqual(foreignClear.status, 0, 'A different session-bus sender must not control the application overlay');
  assert.match(foreignClear.stderr, /Only the connected LOLPing Lite application/);
  checks.push('A second D-Bus sender cannot clear the active application frames');

  // XTest input goes solely to the private outer Xvfb window, then traverses
  // the real Mutter Wayland seat. This does not bypass the production bridge.
  privateInput('windowfocus', '--sync', windowId);
  privateInput('mousemove', '--window', windowId, '640', '480');
  privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen && value.visibleWindows > 0);
  privateInput('mousemove', '--window', windowId, '755', '480');
  await delay(150);
  releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen);
  await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  checks.push('Real Super+Alt+Q input opens the wheel, moves its selection and releases to send');

  privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen);
  privateInput('key', 'Escape');
  releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen && value.visibleWindows === 0);
  checks.push('Escape cancels the Wayland wheel and releases its compositor input grab');

  const inputEvents = [];
  inputClient = spawn('python3', [join(repository, 'scripts/test-gnome-input-client.py')], {
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, WAYLAND_DISPLAY: waylandDisplay, GDK_BACKEND: 'wayland' },
  });
  const inputLines = createInterface({ input: inputClient.stdout, crlfDelay: Infinity });
  inputLines.on('line', line => inputEvents.push(JSON.parse(line)));
  inputClient.stderr.on('data', bytes => appendFileSync(join(testData, 'input-client.log'), bytes));
  await until(() => inputEvents.find(event => event.type === 'ready'), 'The native Wayland input client did not open');
  assert.match(inputEvents.find(event => event.type === 'ready').displayType, /Wayland/);
  await delay(400);
  privateInput('mousemove', '--window', windowId, '640', '480');
  privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen);
  privateInput('mousemove', '--window', windowId, '755', '480');
  privateInput('mousedown', '1', 'mouseup', '1');
  await rpc.poll('diagnostics', value => !value.wheelOpen);
  releaseKeys();
  const afterConfirmation = inputEvents.length;
  privateInput('key', 'a', 'click', '1');
  await until(() => inputEvents.slice(afterConfirmation).some(event => event.type === 'key' && event.key === 'a') &&
    inputEvents.slice(afterConfirmation).some(event => event.type === 'release' && event.button === 1),
  'Keyboard or pointer input remained captured after click confirmation');
  const received = inputEvents.slice(afterConfirmation);
  const letter = received.find(event => event.type === 'key' && event.key === 'a');
  const keyboardModifiers = 1 | 4 | 8 | 64 | (1 << 26); // Shift, Ctrl, Alt, Mod4, Super
  assert.equal(letter.state & keyboardModifiers, 0, 'The next client must not retain the wheel shortcut modifiers');
  assert.deepEqual(received.filter(event => event.type === 'press' || event.type === 'release').map(event => event.type), ['press', 'release']);
  checks.push('After left-click confirmation and shortcut release, a real Wayland client receives plain a and exactly one normal click pair');

  await keyCombo('p');
  await rpc.poll('state', value => value.status.enabled === false);
  await keyCombo('p');
  await rpc.poll('state', value => value.status.enabled === true);
  checks.push('The compositor pause shortcut toggles off and back on without restarting the application');

  await rpc.request('stop');
  const stopped = await rpc.request('diagnostics');
  assert.equal(stopped.connected, false);
  assert.equal(stopped.visibleWindows, 0);
  assert.equal(stopped.surfaceBytes, 0);
  await rpc.request('start');
  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  await rpc.poll('diagnostics', value => value.visibleWindows > 0);
  checks.push('Stop releases the session and frame buffers; a subsequent start and preview succeeds');

  command('gnome-extensions', ['disable', extensionUuid]);
  await rpc.poll('state', value => value.status.state === 'failed');
  const removed = await rpc.request('diagnostics');
  assert.equal(removed.connected, false);
  assert.equal(removed.helperPid, null);
  checks.push('Disabling the GNOME extension reports a recoverable failure and never falls back to XWayland');

  await rpc.close();
  mkdirSync(dirname(reportPath), { recursive: true });
  writeFileSync(reportPath, JSON.stringify({ platform: 'Ubuntu 24.04 / GNOME 46 Wayland', passed: true,
    environment: 'Private nested GNOME compositor on Xvfb; not physical-desktop acceptance', checks }, null, 2) + '\n');
  console.log(JSON.stringify({ passed: true, checks, report: reportPath }, null, 2));
} finally {
  releaseKeys();
  if (inputClient && inputClient.exitCode === null) inputClient.kill('SIGTERM');
  if (rpc) {
    writeFileSync(join(testData, 'application.log'), rpc.log);
    await rpc.close().catch(error => console.error(error));
  }
  await finishShell();
}
