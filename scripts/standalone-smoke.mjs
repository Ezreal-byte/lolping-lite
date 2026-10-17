import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const target = process.platform === 'win32' ? 'windows-x64'
  : process.platform === 'darwin' ? 'macos-arm64' : 'linux-x64';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

export function stagedPaths() {
  const stage = join(repository, '.stage', target);
  if (process.platform === 'darwin') {
    const contents = join(stage, 'LOLPing Lite.app', 'Contents');
    return { root: join(contents, 'Resources'), executable: join(contents, 'MacOS', 'lolping-lite') };
  }
  if (process.platform === 'linux') return {
    root: join(stage, 'AppDir/usr/share/lolping-lite'),
    executable: join(stage, 'AppDir/usr/bin/lolping-lite'),
  };
  return { root: stage, executable: join(stage, 'lolping-lite.exe') };
}

/** Isolated test control transport; production settings use inherited pipes too. */
export class Rpc extends EventEmitter {
  next = 0;
  sequence = 0;
  pending = new Map();
  history = [];
  log = '';
  ended = false;

  constructor(root, data, executable = stagedPaths().executable, options = {}) {
    super();
    this.root = resolve(root);
    this.data = resolve(data);
    mkdirSync(this.data, { recursive: true });
    this.child = spawn(resolve(executable), [
      ...(options.production ? ['--benchmark', '--background'] : ['--headless-test']),
      '--root', this.root, '--data-dir', this.data,
    ], {
      cwd: this.root, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
      env: { ...process.env, ...options.env },
    });
    this.exited = new Promise(resolveExit => {
      this.child.once('exit', (code, signal) => {
        this.ended = true;
        this.rejectAll(new Error(`Application exited (${code}, ${signal}): ${this.log}`));
        resolveExit({ code, signal });
      });
      this.child.once('error', error => {
        this.ended = true;
        this.rejectAll(error);
        resolveExit({ error });
      });
    });
    this.child.stderr.on('data', bytes => {
      this.log = (this.log + bytes.toString()).slice(-32768);
    });
    this.child.stdin.on('error', error => this.rejectAll(error));
    const lines = createInterface({ input: this.child.stdout, crlfDelay: Infinity });
    lines.on('line', line => {
      try {
        const message = JSON.parse(line);
        const pending = this.pending.get(message.id);
        if (pending) {
          clearTimeout(pending.timer);
          this.pending.delete(message.id);
          if (message.error !== undefined) {
            pending.reject(new Error(typeof message.error === 'string' ? message.error : JSON.stringify(message.error)));
          } else pending.resolve(message.result);
        } else if (message.event) {
          const event = { ...message, sequence: ++this.sequence };
          this.history.push(event);
          if (this.history.length > 256) this.history.shift();
          this.emit('event', event);
        }
      } catch (error) {
        this.rejectAll(new Error(`Invalid application JSONL: ${error.message}; ${line.slice(0, 200)}`));
      }
    });
  }

  rejectAll(error) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  request(method, params = {}, timeout = 30000) {
    if (this.ended) return Promise.reject(new Error(`Application already exited: ${this.log}`));
    const id = ++this.next;
    return new Promise((resolveRequest, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Request timed out: ${method}\n${this.log}`));
      }, timeout);
      this.pending.set(id, { resolve: resolveRequest, reject, timer });
      this.child.stdin.write(JSON.stringify({ id, method, params }) + '\n');
    });
  }

  waitFor(predicate, timeout = 15000, after = 0) {
    const found = this.history.find(event => event.sequence > after && predicate(event));
    if (found) return Promise.resolve(found);
    return new Promise((resolveEvent, reject) => {
      const listener = event => {
        if (event.sequence > after && predicate(event)) {
          clearTimeout(timer);
          this.off('event', listener);
          resolveEvent(event);
        }
      };
      const timer = setTimeout(() => {
        this.off('event', listener);
        reject(new Error(`Application event timeout\n${this.log}`));
      }, timeout);
      this.on('event', listener);
    });
  }

  async poll(method, predicate, timeout = 20000) {
    const end = Date.now() + timeout;
    let latest;
    do {
      latest = await this.request(method);
      if (predicate(latest)) return latest;
      await delay(100);
    } while (Date.now() < end);
    throw new Error(`${method} condition timed out: ${JSON.stringify(latest)}\n${this.log}`);
  }

  async close() {
    if (this.ended) return;
    this.child.stdin.end();
    let timer;
    const graceful = await Promise.race([
      this.exited.then(() => true),
      new Promise(resolve => { timer = setTimeout(() => resolve(false), 7000); }),
    ]);
    clearTimeout(timer);
    if (!graceful) {
      this.child.kill('SIGKILL');
      await this.exited;
      throw new Error(`Application did not stop on control-pipe EOF\n${this.log}`);
    }
  }
}

export function processAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try { process.kill(pid, 0); return true; } catch (error) {
    if (error.code === 'ESRCH') return false;
    throw error;
  }
}

async function assertProcessGone(pid, reason) {
  const until = Date.now() + 6000;
  while (processAlive(pid) && Date.now() < until) await delay(100);
  assert.equal(processAlive(pid), false, `${reason}: process ${pid} survived`);
}

// A filename is outside the CRC-covered payload. Replace both copies with an
// equal-length traversal name, retaining a valid ZIP and its original entries.
function traversalPack(original) {
  const bytes = Buffer.from(original);
  let end = bytes.length - 22;
  while (end >= Math.max(0, bytes.length - 65557) && bytes.readUInt32LE(end) !== 0x06054b50) end--;
  assert.ok(end >= 0, 'ZIP has an end-of-central-directory record');
  let offset = bytes.readUInt32LE(end + 16);
  for (let entry = 0; entry < bytes.readUInt16LE(end + 10); entry++) {
    assert.equal(bytes.readUInt32LE(offset), 0x02014b50);
    const length = bytes.readUInt16LE(offset + 28);
    const name = bytes.subarray(offset + 46, offset + 46 + length).toString('utf8');
    if (name.endsWith('.png')) {
      const invalid = Buffer.from('../' + name.slice(3));
      assert.equal(invalid.length, length);
      invalid.copy(bytes, offset + 46);
      const local = bytes.readUInt32LE(offset + 42);
      assert.equal(bytes.readUInt32LE(local), 0x04034b50);
      assert.equal(bytes.readUInt16LE(local + 26), length);
      invalid.copy(bytes, local + 30);
      return bytes;
    }
    offset += 46 + length + bytes.readUInt16LE(offset + 30) + bytes.readUInt16LE(offset + 32);
  }
  throw new Error('ZIP contains no PNG to exercise traversal rejection');
}

async function main() {
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const at = args.indexOf(name);
    if (at < 0) return fallback;
    assert.ok(args[at + 1] && !args[at + 1].startsWith('--'), `${name} needs a value`);
    return resolve(args[at + 1]);
  };
  const defaults = stagedPaths();
  const root = option('--root', defaults.root);
  const executable = option('--executable', defaults.executable);
  const data = option('--data-dir', join(repository, '.test-data', `standalone-smoke-${Date.now()}`));
  const desktop = !args.includes('--no-desktop');
  const reportPath = option('--report', join(repository, 'dist', `smoke-${target}-report.json`));
  const packPath = option('--pack', join(repository, 'dist', 'lolping-extra-emotes-1.zip'));
  const results = [];
  const skipped = desktop ? [] : ['Interactive overlays, input helper, settings windows and descendant crash cleanup (--no-desktop)'];
  mkdirSync(data, { recursive: true });
  for (const path of [root, executable, packPath]) assert.ok(existsSync(path), `Missing test input: ${path}`);

  const selfTest = spawnSync(executable, ['--self-test', '--root', root, '--data-dir', data], {
    cwd: root, windowsHide: true, encoding: 'utf8', timeout: 30000,
  });
  assert.equal(selfTest.status, 0, `Resource self-test failed: ${selfTest.error || ''}\n${selfTest.stdout}\n${selfTest.stderr}`);
  results.push('self-test validates packaged resources without starting a desktop UI');

  const rpc = new Rpc(root, data, executable);
  try {
    let state = await rpc.request('state');
    assert.equal(state.version, JSON.parse(readFileSync(join(repository, 'package.json'), 'utf8')).version);
    assert.equal(state.status.state, 'stopped');
    assert.equal(state.emotes.length, 9);
    assert.equal(state.preferences.autostart, false);
    results.push('test mode stays stopped, uses nine core emotes and keeps autostart disabled');

    await rpc.request('settings', { volume: 101, trigger: { vk: 65 }, toggleHotkey: { mods: 4, vk: 66 } });
    state = await rpc.request('state');
    assert.equal(state.settings.volume, 100);
    assert.deepEqual(state.settings.toggleHotkey, { mods: 3, vk: 80 });
    await assert.rejects(rpc.request('settings', 'invalid'), /invalidSettings/);
    await rpc.request('settings', { volume: 0, trigger: 'alt', keepLastPing: false, pingDurationS: 1 });
    results.push('settings validation clamps limits and rejects invalid request shape');

    const settingsEvent = rpc.history.filter(event => event.event === 'lolping/settings').at(-1);
    assert.equal(settingsEvent?.params.settings.volume, 0);
    assert.equal(settingsEvent?.params.emotes, undefined);
    assert.equal(rpc.history.filter(event => event.event === 'lolping/library').length, 0);
    const settingsEvents = rpc.history.filter(event => event.event === 'lolping/settings').length;
    await rpc.request('settings', { volume: 0 });
    assert.equal(rpc.history.filter(event => event.event === 'lolping/settings').length, settingsEvents);
    results.push('settings events omit thumbnails and unchanged updates do not broadcast');

    const imagePath = join(repository, 'resources/emotes/facepalm.png');
    const soundPath = join(repository, 'resources/emotes/facepalm.ogg');
    const { id } = await rpc.request('import', { kind: 'image', path: imagePath });
    assert.match(id, /^c:[0-9a-f]{32}$/);
    state = await rpc.request('state');
    assert.equal(state.emotes.length, 10);
    await assert.rejects(rpc.request('import', { kind: 'image', path: imagePath }), /duplicateEmote/);
    await assert.rejects(rpc.request('import', { kind: 'image', path: 'relative.png' }), /invalidImportPath/);
    const tooLarge = join(data, 'too-large-image.png');
    writeFileSync(tooLarge, Buffer.alloc(8 * 1048576 + 1));
    await assert.rejects(rpc.request('import', { kind: 'image', path: tooLarge }), /fileTooLarge/);
    results.push('custom images import; duplicate, relative-path and oversized imports are rejected');

    await rpc.request('import', { kind: 'sound', path: soundPath, id });
    state = await rpc.request('state');
    assert.equal(state.settings.customEmotes[0].sound, true);
    results.push('custom OGG sound attaches to the selected local emote');

    await rpc.request('import', { kind: 'pack', path: packPath });
    state = await rpc.request('state');
    assert.equal(state.emotes.length, 45);
    assert.equal(state.extraPackInstalled, true);
    await rpc.request('import', { kind: 'pack', path: packPath });
    const corrupt = join(data, 'corrupt-pack.zip');
    writeFileSync(corrupt, 'invalid ZIP');
    await assert.rejects(rpc.request('import', { kind: 'pack', path: corrupt }), /invalidPack/);
    const escaped = join(data, 'traversal-pack.zip');
    writeFileSync(escaped, traversalPack(readFileSync(packPath)));
    await assert.rejects(rpc.request('import', { kind: 'pack', path: escaped }), /unsafePackPath/);
    results.push('offline extension import is idempotent and rejects corrupt or traversal archives');

    await rpc.request('emote/remove', { id });
    state = await rpc.request('state');
    assert.equal(state.emotes.length, 44);
    assert.equal(state.settings.customEmotes.length, 0);
    results.push('removing a custom emote preserves built-in and extension assets');

    if (desktop) {
      await rpc.request('preview', { id: 'danger', wheel: 'ping' });
      let diagnostics = await rpc.poll('diagnostics', value => value.windows > 0 && value.windows === value.readyWindows);
      assert.equal(diagnostics.helperPid, null);
      assert.equal(diagnostics.renderer, 'native');
      if (process.platform === 'win32') assert.deepEqual(diagnostics.systemFrames, Array(diagnostics.windows).fill(false));
      results.push('native preview works without input hooks and without Windows DWM frames');

      await rpc.request('start');
      await rpc.poll('state', value => value.status.state === 'running');
      diagnostics = await rpc.request('diagnostics');
      const helperPid = diagnostics.helperPid;
      assert.ok(helperPid > 0);
      await rpc.request('start');
      assert.equal((await rpc.request('diagnostics')).helperPid, helperPid);
      const writes = diagnostics.helperConfigWrites;
      await rpc.request('settings', { volume: 5, pingSizePx: 120 });
      diagnostics = await rpc.request('diagnostics');
      assert.equal(diagnostics.helperPid, helperPid);
      assert.equal(diagnostics.helperConfigWrites, writes);
      await rpc.request('settings', { volume: 0 });
      results.push('repeated start and appearance changes preserve the input helper and its configuration');

      await rpc.request('pause', { on: true });
      state = await rpc.request('state');
      assert.equal(state.status.enabled, false);
      assert.equal(state.status.pauseReason, 'manual');
      await rpc.request('suspend', { on: true });
      state = await rpc.request('state');
      assert.equal(state.status.pauseReason, 'capture');
      await rpc.request('suspend', { on: false });
      state = await rpc.request('state');
      assert.equal(state.status.enabled, false);
      assert.equal(state.status.pauseReason, 'manual');
      await rpc.request('pause', { on: false });
      await rpc.request('suspend', { on: true });
      await rpc.request('suspend', { on: false });
      state = await rpc.request('state');
      assert.equal(state.status.enabled, true);
      assert.equal(state.status.pauseReason, null);
      results.push('shortcut capture restores both a previous manual pause and a previous running state');

      await rpc.poll('diagnostics', value => value.visibleWindows === 0 && value.lowMemoryWindows === value.windows);
      for (let index = 0; index < 12; index++) await rpc.request('preview', { id: 'danger', wheel: 'ping' });
      diagnostics = await rpc.poll('diagnostics', value => value.frameMs.length >= 12);
      assert.equal(diagnostics.helperPid, helperPid);
      assert.ok(diagnostics.visibleWindows > 0);
      if (process.platform === 'win32') assert.deepEqual(diagnostics.systemFrames, Array(diagnostics.windows).fill(false));
      await rpc.request('pause', { on: true });
      diagnostics = await rpc.poll('diagnostics', value => value.visibleWindows === 0 && value.surfaceBytes === 0);
      await rpc.request('pause', { on: false });
      results.push('idle buffers release, burst previews wake correctly, and pausing clears the native surface');

      const settingsPids = new Set();
      const openSettings = async () => {
        const mark = rpc.sequence;
        await rpc.request('ui/open');
        await rpc.waitFor(event => event.event === 'ui/ready', 30000, mark);
        return rpc.poll('diagnostics', value => value.settingsPid > 0);
      };
      for (let cycle = 0; cycle < 20; cycle++) {
        diagnostics = await openSettings();
        const settingsPid = diagnostics.settingsPid;
        settingsPids.add(settingsPid);
        assert.equal(diagnostics.helperPid, helperPid);
        await rpc.request('ui/open');
        assert.equal((await rpc.request('diagnostics')).settingsPid, settingsPid, 'only one settings process may be open');
        await rpc.request('ui/close');
        await rpc.poll('diagnostics', value => value.settingsPid === null);
        await assertProcessGone(settingsPid, 'closing settings');
      }
      for (const pid of settingsPids) await assertProcessGone(pid, 'settings lifecycle');
      await rpc.request('preview', { id: 'missing', wheel: 'ping' });
      diagnostics = await rpc.poll('diagnostics', value => value.visibleWindows > 0);
      assert.equal(diagnostics.helperPid, helperPid);
      results.push('settings open/close 20 times without residual settings processes or restarting input hooks');

      diagnostics = await openSettings();
      process.kill(diagnostics.settingsPid, 'SIGKILL');
      await rpc.poll('diagnostics', value => value.settingsPid === null);
      assert.equal((await rpc.request('diagnostics')).helperPid, helperPid);
      assert.equal((await rpc.request('state')).status.enabled, true);
      await openSettings();
      await rpc.request('ui/close');
      results.push('settings-process crash does not stop signaling and the settings window can reopen');

      await rpc.request('stop');
      diagnostics = await rpc.request('diagnostics');
      assert.equal(diagnostics.windows, 0);
      assert.equal(diagnostics.helperPid, null);
      await assertProcessGone(helperPid, 'stop');
      results.push('stop removes every overlay and releases the input helper');

      await rpc.request('start');
      await rpc.poll('state', value => value.status.state === 'running');
      diagnostics = await openSettings();
      assert.ok(diagnostics.helperPid > 0);
      await rpc.close();
      await assertProcessGone(diagnostics.helperPid, 'controller EOF');
      await assertProcessGone(diagnostics.settingsPid, 'controller EOF');
      results.push('controller pipe EOF cleans up both the input helper and settings process');
    }
    await rpc.close();

    const second = new Rpc(root, data, executable);
    try {
      state = await second.request('state');
      assert.equal(state.status.state, 'stopped');
      assert.equal(state.settings.volume, 0);
      assert.equal(state.emotes.length, 44);
      assert.equal(state.extraPackInstalled, true);
    } finally { await second.close(); }
    results.push('restart retains settings and extension resources in the independent app data directory');

    if (desktop) {
      const crash = new Rpc(root, data, executable);
      try {
        await crash.request('start');
        await crash.poll('state', value => value.status.state === 'running');
        const mark = crash.sequence;
        await crash.request('ui/open');
        await crash.waitFor(event => event.event === 'ui/ready', 30000, mark);
        const diagnostics = await crash.poll('diagnostics', value => value.settingsPid > 0 && value.helperPid > 0);
        crash.child.kill('SIGKILL');
        await crash.exited;
        await assertProcessGone(diagnostics.helperPid, 'forced main-process termination');
        await assertProcessGone(diagnostics.settingsPid, 'forced main-process termination');
      } finally { await crash.close(); }
      results.push('forced main-process termination leaves no helper or settings descendants');
    }

    if (desktop && process.platform === 'linux' && process.env.LOLPING_PRIVATE_XVFB === '1') {
      // The CI's private dbus-run-session deliberately has no tray host. Exercise
      // normal startup here: headless-test intentionally skips tray creation.
      const fallback = new Rpc(root, data, executable, { production: true });
      try {
        await fallback.waitFor(event => event.event === 'ui/ready', 30000);
        const diagnostics = await fallback.poll('diagnostics', value => value.settingsPid > 0 && value.helperPid > 0);
        assert.match(fallback.log, /No system-tray host is available/);
        const duplicate = spawnSync(executable, ['--background', '--root', root, '--data-dir', data], {
          cwd: root, encoding: 'utf8', timeout: 10000,
        });
        assert.equal(duplicate.status, 0, `Second launch failed: ${duplicate.stderr}`);
        assert.equal((await fallback.request('diagnostics')).helperPid, diagnostics.helperPid);
        assert.equal((await fallback.request('diagnostics')).settingsPid, diagnostics.settingsPid);
        await fallback.request('ui/close');
        let timer;
        const exited = await Promise.race([
          fallback.exited,
          new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Tray-less app remained hidden after its control window closed')), 7000); }),
        ]).finally(() => clearTimeout(timer));
        assert.equal(exited.code, 0);
        await assertProcessGone(diagnostics.helperPid, 'tray-less app exit');
        await assertProcessGone(diagnostics.settingsPid, 'tray-less app exit');
      } finally { await fallback.close(); }
      results.push('Linux without a tray opens visible controls; duplicate launch reuses them and closing exits cleanly');

      // A usable X11 DISPLAY inside a Wayland login is XWayland, not evidence
      // of desktop-wide input access. Simulate that session on our isolated X
      // server so native startup, settings availability and rejection are tested.
      const wayland = new Rpc(root, data, executable, {
        production: true, env: { XDG_SESSION_TYPE: 'wayland' },
      });
      try {
        await wayland.waitFor(event => event.event === 'ui/ready', 30000);
        const unsupported = await wayland.poll('state', value => value.status.state === 'failed');
        assert.equal(unsupported.status.enabled, false);
        assert.match(unsupported.status.error, /X11.*Wayland/s);
        await assert.rejects(wayland.request('start'), /X11.*Wayland/s);
        await assert.rejects(wayland.request('preview', { id: 'missing', wheel: 'ping' }), /X11.*Wayland/s);
        const diagnostics = await wayland.request('diagnostics');
        assert.ok(diagnostics.settingsPid > 0, 'unsupported sessions must retain accessible settings');
        assert.equal(diagnostics.helperPid, null, 'Wayland must never start an X11 input helper');
        assert.equal(diagnostics.windows, 0, 'Wayland must never create misleading partial overlays');
      } finally { await wayland.close(); }
      results.push('Wayland login with an X11 DISPLAY is rejected before helper/overlay creation while native settings remain accessible');
    }
    mkdirSync(dirname(reportPath), { recursive: true });
    const report = { target, desktop, passed: results, skipped, at: new Date().toISOString(), dataDirectory: data };
    writeFileSync(reportPath, JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify(report, null, 2));
  } finally { await rpc.close(); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
