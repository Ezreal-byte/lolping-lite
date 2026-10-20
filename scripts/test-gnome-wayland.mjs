// Invoked only by test-gnome-wayland.py inside native-headless GNOME and private D-Bus.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { Rpc } from './standalone-smoke.mjs';
import { extensionUuid } from './gnome-extension.mjs';
import { BUS_NAME, OBJECT_PATH, INTERFACE } from '../native/gnome-extension/lolping-lite@ezreal-byte.github.io/protocol.js';

assert.equal(process.platform, 'linux');
assert.equal(process.env.LOLPING_PRIVATE_GNOME, '1', 'Use the isolated GNOME launcher, never the login desktop');
assert.equal(process.env.LOLPING_GNOME_HEADLESS, '1');
assert.equal(process.env.DISPLAY, undefined, 'Native-headless acceptance must not use X11');
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
const inputEvents = [];
const mappingSamples = [];
const keyboardSamples = [];
const keyboardModifiers = 1 | 4 | 8 | 64 | (1 << 26); // Shift, Ctrl, Alt, Mod4, Super
let shell, rpc, inputClient, seat, seatReady, seatError;
let seatSequence = 0;
const seatRequests = new Map();
let passed = false;

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
async function seatRequest(command, params = {}) {
  if (seatError) throw seatError;
  assert.ok(seat && seat.exitCode === null, 'Private compositor input driver must be running');
  const id = ++seatSequence;
  return new Promise((resolveRequest, reject) => {
    const timer = setTimeout(() => { seatRequests.delete(id); reject(Error(`Private seat ${command} timed out`)); }, 10000);
    seatRequests.set(id, { resolve: resolveRequest, reject, timer });
    seat.stdin.write(`${JSON.stringify({ id, command, ...params })}\n`);
  });
}
async function privateInput(...args) {
  // All events go through Mutter's own virtual input device on this private
  // compositor, without XTest or a production shortcut/gesture bypass.
  for (let i = 0; i < args.length; i += 2) {
    const action = args[i], value = args[i + 1];
    if (['keydown', 'keyup', 'key'].includes(action)) {
      await seatRequest('key', { key: value, pressed: action !== 'keyup' });
      if (action === 'key') await seatRequest('key', { key: value, pressed: false });
    } else if (['mousedown', 'mouseup', 'click'].includes(action)) {
      await seatRequest('button', { button: Number(value), pressed: action !== 'mouseup' });
      if (action === 'click') await seatRequest('button', { button: Number(value), pressed: false });
    } else throw Error(`Unexpected private input action ${action}`);
  }
}
async function movePointer(x, y) {
  // Relative motion needs no screencast stream; clamp to the virtual monitor's
  // origin first, then move to the same test coordinates on the real seat.
  await seatRequest('move', { dx: -10000, dy: -10000 });
  await seatRequest('move', { dx: x, dy: y });
}
async function capture(name) {
  // Allow the acknowledged texture to reach the compositor's next frame.
  await delay(name === 'signal' ? 600 : 100);
  console.log(`Private compositor screenshot: ${JSON.stringify(await seatRequest('screenshot', { name }))}`);
}
async function releaseKeys() {
  if (!seatReady || seat?.exitCode !== null) return;
  for (const key of ['q', 'e', 'p', 'Escape', 'Alt_L', 'Super_L'])
    await seatRequest('key', { key, pressed: false });
}
async function sampleKeyboard(stage) {
  const injected = await seatRequest('state');
  const firstEvent = inputEvents.length;
  await privateInput('key', 'a');
  const letter = await until(() => inputEvents.slice(firstEvent).find(event => event.type === 'key' && event.key === 'a'),
    `The focused Wayland client did not receive a (${stage})`);
  const sample = { stage, injected, clientModifiers: letter.state };
  keyboardSamples.push(sample);
  console.log(`Private keyboard state: ${JSON.stringify(sample)}`);
  return sample;
}
async function keyCombo(key) {
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'key', key, 'keyup', 'Alt_L', 'keyup', 'Super_L');
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
  shell = spawn('gnome-shell', ['--headless', '--wayland', '--no-x11', '--virtual-monitor=1280x960', `--wayland-display=${waylandDisplay}`], {
    stdio: ['ignore', 'pipe', 'pipe'], env: process.env,
  });
  for (const stream of [shell.stdout, shell.stderr]) stream.on('data', bytes => appendFileSync(logPath, bytes));
  shell.once('error', error => appendFileSync(logPath, `${error}\n`));
  await until(() => existsSync(join(process.env.XDG_RUNTIME_DIR, waylandDisplay)), 'GNOME Wayland display did not appear');
  await until(() => command('gdbus', ['introspect', '--session', '--dest', BUS_NAME, '--object-path', OBJECT_PATH], true).status === 0,
    'Bundled GNOME extension did not export its bridge');
  checks.push('GNOME 46 loads the bundled extension on a real native-headless Wayland compositor');
  await until(() => command('gdbus', ['introspect', '--session', '--dest', 'org.gnome.Shell.Screenshot',
    '--object-path', '/org/gnome/Shell/Screenshot'], true).status === 0, 'Private compositor screenshot service did not start');
  seat = spawn('python3', [join(repository, 'scripts/gnome-private-seat.py')], { stdio: ['pipe', 'pipe', 'pipe'], env: process.env });
  createInterface({ input: seat.stdout, crlfDelay: Infinity }).on('line', line => {
    const reply = JSON.parse(line);
    if (reply.event === 'ready') { seatReady = reply; return; }
    if (reply.event === 'fatal') { seatError = Error(reply.error); return; }
    const pending = seatRequests.get(reply.id);
    if (!pending) return;
    clearTimeout(pending.timer);
    seatRequests.delete(reply.id);
    if (reply.ok) pending.resolve(reply.result);
    else pending.reject(Error(reply.error));
  });
  seat.stderr.on('data', bytes => appendFileSync(join(testData, 'private-seat.log'), bytes));
  seat.once('error', error => { seatError = error; });
  seat.once('exit', code => {
    for (const pending of seatRequests.values()) {
      clearTimeout(pending.timer);
      pending.reject(Error(`Private input driver exited (${code}): ${seatError ?? 'see private-seat.log'}`));
    }
    seatRequests.clear();
  });
  await until(() => { if (seatError) throw seatError; return seatReady; }, 'Private native virtual seat did not start');
  assert.equal(seatReady.backend, 'native-headless');
  assert.equal(seatReady.shellPid, shell.pid);
  await movePointer(640, 480);
  await privateInput('key', 'Escape');
  await delay(600);

  for (const bundled of [false, true]) {
    const env = { ...process.env };
    if (bundled) env.LD_LIBRARY_PATH = join(dirname(executable), 'usr/lib');
    const diagnostic = spawnSync('python3', [join(repository, 'scripts/diagnose-linux-egl.py')], {
      encoding: 'utf8', timeout: 15000, env,
    });
    appendFileSync(join(testData, 'egl-loader.log'), `${bundled ? 'AppImage' : 'System'} EGL dependencies:\n${diagnostic.stdout ?? ''}${diagnostic.stderr ?? ''}${diagnostic.error ?? ''}\n`);
    console.log(`EGL dependency diagnostics (${bundled ? 'AppImage' : 'system'}):\n${diagnostic.stdout ?? ''}${diagnostic.stderr ?? ''}${diagnostic.error ?? ''}`);
  }
  rpc = new Rpc(root, join(testData, 'application'), executable, {
    production: true,
    env: { WAYLAND_DISPLAY: waylandDisplay, XDG_SESSION_TYPE: 'wayland', GDK_BACKEND: 'wayland',
      EGL_LOG_LEVEL: 'debug', LIBGL_DEBUG: 'verbose' },
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
  async function assertRunning(stage) {
    const state = await rpc.request('state');
    const diagnostics = await rpc.request('diagnostics');
    assert.equal(state.status.state, 'running', `${stage}: ${JSON.stringify(state.status)}`);
    assert.equal(diagnostics.connected, true, `${stage}: bridge disconnected`);
  }
  const installedMetadata = join(process.env.XDG_DATA_HOME, 'gnome-shell/extensions', extensionUuid, 'metadata.json');
  const metadataModified = statSync(installedMetadata).mtimeMs;
  const installation = await rpc.request('wayland/install');
  assert.equal(installation.ok, true);
  assert.equal(installation.extensionId, extensionUuid);
  assert.equal(installation.reloginRequired, false, 'An already-loaded identical extension must not request another login');
  assert.equal(statSync(installedMetadata).mtimeMs, metadataModified, 'Identical extension files must not be rewritten');
  const maps = readFileSync(`/proc/${rpc.child.pid}/maps`, 'utf8');
  assert.ok(maps.includes(join(dirname(executable), 'usr/lib/gio/modules/libdconfsettings.so')),
    'The AppImage must load its own dconf backend built against its bundled GLib');
  checks.push('Installing an identical already-enabled extension succeeds without rewriting it or requesting another login');

  async function sampleMappings(stage) {
    const diagnostics = await rpc.request('diagnostics');
    const processStatus = readFileSync(`/proc/${shell.pid}/status`, 'utf8');
    const kib = field => Number(processStatus.match(new RegExp(`^${field}:\\s+(\\d+)`, 'm'))?.[1] ?? 0);
    mappingSamples.push({ stage, presentedFrames: diagnostics.presentedFrames,
      shellRssKiB: kib('VmRSS'), shellAnonymousKiB: kib('RssAnon'), shellPeakRssKiB: kib('VmHWM'),
      mappedFrameFiles: readFileSync(`/proc/${shell.pid}/maps`, 'utf8').split('\n').filter(line => line.includes('/lolping-lite-frames-')).length });
    return diagnostics;
  }
  await sampleMappings('before-preview');

  await rpc.request('settings', { pingDurationS: 3 });
  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  await rpc.poll('diagnostics', value => value.visibleWindows > 0 && value.surfaceBytes > 0 && value.presentedFrames > initial.presentedFrames);
  const framesBeforeCapture = (await rpc.request('diagnostics')).presentedFrames;
  await capture('signal');
  const framesAfterCapture = (await rpc.request('diagnostics')).presentedFrames;
  console.log(`Native frames during 600 ms without RPC polling: ${framesBeforeCapture} -> ${framesAfterCapture}`);
  assert.ok(framesAfterCapture - framesBeforeCapture >= 3,
    'Native animation must advance on its own timer, without RPC polling or pointer motion');
  await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  await rpc.request('settings', { pingDurationS: 1 });
  checks.push('Native signal frames are acknowledged by GNOME and cleared when the animation finishes');
  for (let cycle = 0; cycle < 20; cycle++) {
    const diagnostics = await sampleMappings(`preview-${cycle}`);
    if (diagnostics.presentedFrames - initial.presentedFrames >= 300) break;
    await rpc.request('preview', { id: 'missing', wheel: 'ping' });
    await rpc.poll('diagnostics', value => value.visibleWindows > 0);
    await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  }
  const afterFrames = await sampleMappings('after-300-frames');
  assert.ok(afterFrames.presentedFrames - initial.presentedFrames >= 300, 'The lifecycle and GC regression must cover at least 300 acknowledged native frames');
  assert.ok(mappingSamples.every(sample => sample.mappedFrameFiles === 0), 'GNOME must copy mutable frame bytes instead of retaining file mappings across animation frames');
  await assertRunning('After native signal previews');
  checks.push('At least 300 acknowledged native animation frames keep the application healthy with zero retained frame-file mappings in GNOME Shell');

  const foreignClear = dbus('Clear', ['0'], true);
  assert.notEqual(foreignClear.status, 0, 'A different session-bus sender must not control the application overlay');
  assert.match(foreignClear.stderr, /Only the connected LOLPing Lite application/);
  checks.push('A second D-Bus sender cannot clear the active application frames');

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
  await movePointer(640, 480);
  await privateInput('click', '1');
  await until(() => inputEvents.some(event => event.type === 'release'), 'Click must focus the real Wayland client before baseline input');
  const initialKeyboard = await sampleKeyboard('before-any-modal');
  assert.equal(initialKeyboard.clientModifiers & keyboardModifiers, 0, 'Initial client keyboard baseline must be clear');
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L');
  await releaseKeys();
  const baseline = await sampleKeyboard('shortcut-modifiers-without-wheel');
  assert.equal(baseline.clientModifiers & keyboardModifiers, 0, 'Super/Alt without a wheel must release normally');
  checks.push('A focused real Wayland client receives unmodified input before any modal wheel, including after plain Super/Alt press and release');

  // Exercise Mutter's native Wayland seat and the production extension.
  await movePointer(640, 480);
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen && value.visibleWindows > 0);
  await capture('wheel');
  await movePointer(755, 480);
  await delay(150);
  await releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen);
  await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  await assertRunning('After Q wheel release');
  await sampleKeyboard('after-Q-release');
  checks.push('Real Super+Alt+Q input opens the wheel, moves its selection and releases to send');

  await rpc.request('settings', { emoteTrigger: 'ctrl' });
  await movePointer(640, 480);
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'e');
  await rpc.poll('diagnostics', value => value.wheelOpen && value.visibleWindows > 0);
  await movePointer(525, 480);
  await delay(150);
  await releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen);
  await rpc.poll('diagnostics', value => value.visibleWindows === 0, 20000);
  await assertRunning('After E wheel release');
  await sampleKeyboard('after-E-release');
  checks.push('With emotes enabled, real Super+Alt+E opens, selects and releases the emote wheel');

  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen);
  await privateInput('key', 'Escape');
  await releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen && value.visibleWindows === 0);
  await assertRunning('After Escape cancellation');
  await sampleKeyboard('after-Escape-cancel');
  checks.push('Escape cancels the Wayland wheel and releases its compositor input grab');

  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen && value.visibleWindows > 0);
  const watchdogStarted = Date.now();
  await rpc.poll('diagnostics', value => !value.wheelOpen && value.visibleWindows === 0, 20000);
  assert.ok(Date.now() - watchdogStarted >= 13000, 'The held wheel must reach the 15-second watchdog, not close from an accidental key release');
  await assertRunning('After held-wheel watchdog cancellation');
  await releaseKeys();
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen && value.visibleWindows > 0);
  await privateInput('key', 'Escape');
  await releaseKeys();
  await rpc.poll('diagnostics', value => !value.wheelOpen && value.visibleWindows === 0);
  await assertRunning('After reopening following watchdog cancellation');
  await sampleKeyboard('after-watchdog-and-reopen');
  checks.push('Holding Q reaches the 15-second safety cancellation without stopping the application; the next wheel still opens and cancels normally');

  await movePointer(640, 480);
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await rpc.poll('diagnostics', value => value.wheelOpen);
  await movePointer(755, 480);
  await privateInput('mousedown', '1', 'mouseup', '1');
  await rpc.poll('diagnostics', value => !value.wheelOpen);
  await releaseKeys();
  await assertRunning('After left-click confirmation');
  await sampleKeyboard('after-left-click-confirmation');
  const afterConfirmation = inputEvents.length;
  await privateInput('key', 'a', 'click', '1');
  await until(() => inputEvents.slice(afterConfirmation).some(event => event.type === 'key' && event.key === 'a') &&
    inputEvents.slice(afterConfirmation).some(event => event.type === 'release' && event.button === 1),
  'Keyboard or pointer input remained captured after click confirmation');
  const received = inputEvents.slice(afterConfirmation);
  const letter = received.find(event => event.type === 'key' && event.key === 'a');
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
  await rpc.request('preview', { id: 'missing', wheel: 'ping' });
  await rpc.poll('diagnostics', value => value.visibleWindows > 0);
  await rpc.request('settings', { tickSound: false });
  await keyCombo('p');
  await privateInput('keydown', 'Super_L', 'keydown', 'Alt_L', 'keydown', 'q');
  await delay(250);
  assert.equal((await rpc.request('diagnostics')).wheelOpen, false, 'Stopped preview settings must not register gesture shortcuts');
  assert.equal((await rpc.request('state')).status.state, 'stopped');
  await releaseKeys();
  const afterStoppedShortcut = inputEvents.length;
  await privateInput('key', 'a', 'click', '1');
  await until(() => inputEvents.slice(afterStoppedShortcut).some(event => event.type === 'key' && event.key === 'a') &&
    inputEvents.slice(afterStoppedShortcut).some(event => event.type === 'release' && event.button === 1),
  'Stopped preview settings left an invisible modal input grab');
  checks.push('A stopped preview followed by settings changes and P/Q shortcuts leaves ordinary Wayland input usable');
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

  // Read back via a fresh host GSettings process, using the same private dconf
  // service as Shell. A builtin keyfile fallback would silently miss this test.
  const unrelatedEnabled = 'unrelated-enabled@lolping-test.invalid';
  const unrelatedDisabled = 'unrelated-disabled@lolping-test.invalid';
  command('gsettings', ['set', 'org.gnome.shell', 'enabled-extensions', `['${unrelatedEnabled}']`]);
  command('gsettings', ['set', 'org.gnome.shell', 'disabled-extensions', `['${unrelatedDisabled}', '${extensionUuid}']`]);
  const reinstalled = await rpc.request('wayland/install');
  assert.equal(reinstalled.ok, true);
  const readExtensions = key => JSON.parse(command('gsettings', ['get', 'org.gnome.shell', key]).stdout.trim()
    .replace(/^@as /, '').replaceAll("'", '"'));
  assert.deepEqual(new Set(readExtensions('enabled-extensions')), new Set([unrelatedEnabled, extensionUuid]));
  assert.deepEqual(readExtensions('disabled-extensions'), [unrelatedDisabled]);
  assert.ok(statSync(join(process.env.XDG_CONFIG_HOME, 'dconf/user')).size > 0);
  assert.equal(command('gsettings', ['get', 'org.gnome.shell', 'disable-user-extensions']).stdout.trim(), 'false');
  checks.push('AppImage installer writes the shared private dconf database; fresh host GSettings reads back its UUID and preserves unrelated extension preferences');

  await rpc.close();
  mkdirSync(dirname(reportPath), { recursive: true });
  const source = JSON.parse(readFileSync(join(repository, 'dist/gnome-source-verification.json'), 'utf8'));
  writeFileSync(reportPath, JSON.stringify({ platform: 'Ubuntu 24.04 / GNOME 46 Wayland', passed: true, ...source,
    environment: 'Private native-headless GNOME Wayland compositor with a virtual input seat; not physical-desktop acceptance', checks, mappingSamples, keyboardSamples }, null, 2) + '\n');
  passed = true;
  console.log(JSON.stringify({ passed: true, checks, report: reportPath }, null, 2));
} finally {
  await releaseKeys().catch(error => console.error(error));
  writeFileSync(join(testData, 'input-events.json'), JSON.stringify(inputEvents, null, 2) + '\n');
  writeFileSync(join(testData, 'mapping-samples.json'), JSON.stringify(mappingSamples, null, 2) + '\n');
  writeFileSync(join(testData, 'keyboard-samples.json'), JSON.stringify(keyboardSamples, null, 2) + '\n');
  if (inputClient && inputClient.exitCode === null) inputClient.kill('SIGTERM');
  if (rpc) {
    const state = await rpc.request('state', {}, 3000).catch(error => ({ error: String(error) }));
    const diagnostics = await rpc.request('diagnostics', {}, 3000).catch(error => ({ error: String(error) }));
    // The state includes base64 thumbnails; keep configuration and error
    // evidence without flooding CI logs with unrelated image data.
    const { emotes: _emotes, ...stateSummary } = state;
    const snapshot = { passed, checks, state: stateSummary, diagnostics, events: rpc.history };
    writeFileSync(join(testData, 'runtime-state.json'), JSON.stringify(snapshot, null, 2) + '\n');
    if (!passed) console.error(`Wayland runtime state:\n${JSON.stringify(snapshot, null, 2)}`);
    writeFileSync(join(testData, 'application.log'), rpc.log);
    await rpc.close().catch(error => console.error(error));
  }
  if (seat && seat.exitCode === null) {
    await seatRequest('quit').catch(error => console.error(error));
    seat.stdin.end();
    await Promise.race([new Promise(resolveExit => seat.once('exit', resolveExit)), delay(3000)]);
    if (seat.exitCode === null) seat.kill('SIGTERM');
  }
  await finishShell();
  if (!passed) {
    for (const name of ['gnome-shell.log', 'application.log', 'input-client.log', 'private-seat.log', 'egl-loader.log']) {
      const path = join(testData, name);
      if (existsSync(path)) console.error(`${name} (tail):\n${readFileSync(path, 'utf8').slice(-12000)}`);
    }
  }
}
